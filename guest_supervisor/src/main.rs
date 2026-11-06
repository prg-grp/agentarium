//! terrarium-supervisor: the only thing in the guest that starts a session.
//!
//! systemd starts it at boot. It listens on the command plane's vsock port, says Hello
//! to the app, and waits for Start, which decides what this boot runs on the console:
//! the agent's command as `ubuntu` (agent mode), a root shell (maintenance mode), or the
//! provisioning script (provision mode). Until then, nothing logs in or runs there.
//! Stop shuts the guest down. Built for the Linux guest only (see the Makefile).

use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use command_plane::{
    EnvVar, GuestMessage, Hello, HostMessage, Mode, PORT, PROTOCOL_VERSION, ProvisionFinished,
    Start, guest_message, host_message,
};

/// The virtio console, shown in the app's Terminal tab.
const CONSOLE: &str = "/dev/hvc0";
/// How long to wait before starting an exited session again.
const RESTART_DELAY: Duration = Duration::from_secs(2);
/// Where the provisioning script is written; a tmpfs, gone after the boot.
const RUN_DIR: &str = "/run/terrarium";

/// The app's current connection. Provision mode reports its result on it.
type Channel = Arc<Mutex<Option<File>>>;

fn main() {
    // The vsock transport module may still be loading early in the boot.
    let listener = loop {
        match listen() {
            Ok(listener) => break listener,
            Err(error) => {
                eprintln!("Couldn't listen on vsock port {PORT}: {error}. Trying again.");
                thread::sleep(Duration::from_secs(1));
            }
        }
    };
    let channel = Channel::default();
    let mut started = false;
    loop {
        match accept(&listener) {
            Ok(connection) => serve(connection, &channel, &mut started),
            Err(error) => {
                eprintln!("Couldn't accept a connection: {error}");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

/// Says Hello on a new connection from the app, then follows its messages until it closes.
/// The session keeps running when the connection closes.
fn serve(connection: File, channel: &Channel, started: &mut bool) {
    let mut reader = match connection.try_clone() {
        Ok(reader) => reader,
        Err(error) => return eprintln!("Couldn't read the connection: {error}"),
    };
    *channel.lock().unwrap() = Some(connection);
    send(
        channel,
        guest_message::Kind::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
        }),
    );
    loop {
        let message: HostMessage = match command_plane::read_message(&mut reader) {
            Ok(message) => message,
            Err(error) => {
                if error.kind() != io::ErrorKind::UnexpectedEof {
                    eprintln!("Couldn't read from the app: {error}");
                }
                break;
            }
        };
        match message.kind {
            Some(host_message::Kind::Start(start)) if !*started => {
                *started = true;
                let channel = channel.clone();
                thread::spawn(move || run(start, &channel));
            }
            Some(host_message::Kind::Start(_)) => {
                eprintln!("Ignoring Start: this boot has already started.");
            }
            Some(host_message::Kind::Stop(_)) => power_off(),
            None => eprintln!("Ignoring a message this supervisor doesn't know."),
        }
    }
    channel.lock().unwrap().take();
}

/// Sends a message to the app, if it is connected.
fn send(channel: &Channel, kind: guest_message::Kind) {
    if let Some(connection) = channel.lock().unwrap().as_mut()
        && let Err(error) =
            command_plane::write_message(connection, &GuestMessage { kind: Some(kind) })
    {
        eprintln!("Couldn't write to the app: {error}");
    }
}

/// Runs the boot mode the app chose. Agent and maintenance sessions start again
/// whenever they exit; there is no fallback to a shell.
fn run(start: Start, channel: &Channel) {
    match start.mode() {
        Mode::Agent => {
            let Some(launch) = start.launch.filter(|launch| !launch.command.is_empty()) else {
                return say("The app sent no command to start.");
            };
            say("Starting agent mode.");
            repeat(&launch.command, || {
                session(
                    "ubuntu",
                    &launch.command,
                    &launch.arguments,
                    &launch.working_directory,
                    &launch.environment,
                    true,
                )
            });
        }
        Mode::Maintenance => {
            say("Starting maintenance mode.");
            repeat("The root shell", || {
                session("root", "/bin/bash", &["--login".into()], "", &[], true)
            });
        }
        Mode::Provision => {
            say("Starting provision mode. Running the provisioning script.");
            let exit_code = provision(&start.provision_script);
            send(
                channel,
                guest_message::Kind::ProvisionFinished(ProvisionFinished { exit_code }),
            );
            say(&format!(
                "The provisioning script exited with {exit_code}. Powering off."
            ));
            power_off();
        }
        Mode::Unspecified => say("The app sent no mode to start."),
    }
}

/// Starts a session again and again, a little after each exit.
fn repeat(name: &str, start: impl Fn() -> io::Result<ExitStatus>) -> ! {
    loop {
        match start() {
            Ok(status) => say(&format!("{name} {}. Starting it again.", ended(status))),
            Err(error) => say(&format!("Couldn't start {name}: {error}. Trying again.")),
        }
        thread::sleep(RESTART_DELAY);
    }
}

/// Writes the script to the run folder and runs it as root. Returns its exit code, or -1.
fn provision(script: &str) -> i32 {
    let path = format!("{RUN_DIR}/provision.sh");
    let written = fs::create_dir_all(RUN_DIR)
        .and_then(|()| fs::write(&path, script))
        .and_then(|()| fs::set_permissions(&path, fs::Permissions::from_mode(0o700)));
    if let Err(error) = written {
        say(&format!("Couldn't write the provisioning script: {error}"));
        return -1;
    }
    match session("root", "/bin/bash", &[path], "", &[], false) {
        Ok(status) => status.code().unwrap_or(-1),
        Err(error) => {
            say(&format!("Couldn't run the provisioning script: {error}"));
            -1
        }
    }
}

/// Runs `program` as `user` on the console and waits for it. An interactive session
/// reads the console and gets it as its controlling terminal; otherwise stdin is empty.
/// An empty `directory` means the user's home folder.
fn session(
    user: &str,
    program: &str,
    arguments: &[String],
    directory: &str,
    environment: &[EnvVar],
    interactive: bool,
) -> io::Result<ExitStatus> {
    let account = Account::find(user)?;
    let console = open_console()?;
    let home = account.home.as_str();
    let mut command = Command::new(program);
    command
        .args(arguments)
        .env_clear()
        .env("HOME", home)
        .env("USER", user)
        .env("LOGNAME", user)
        .env("SHELL", "/bin/bash")
        .env(
            "PATH",
            format!(
                "{home}/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
            ),
        )
        .env("TERM", "xterm-256color")
        .env("LANG", "C.UTF-8")
        .envs(environment.iter().map(|var| (&var.name, &var.value)))
        .current_dir(if directory.is_empty() {
            home
        } else {
            directory
        })
        .stdout(console.try_clone()?)
        .stderr(console.try_clone()?);
    if interactive {
        command.stdin(console);
    } else {
        command.stdin(Stdio::null());
    }
    let Account {
        uid, gid, groups, ..
    } = account;
    // Runs in the child between fork and exec, so only async-signal-safe calls. The new
    // session takes the console while still root, so it can take it over from a session
    // that left processes behind.
    unsafe {
        command.pre_exec(move || {
            let check = |result: libc::c_int| {
                if result < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            };
            check(libc::setsid())?;
            if interactive {
                check(libc::ioctl(0, libc::TIOCSCTTY, 1))?;
            }
            check(libc::setgroups(groups.len(), groups.as_ptr()))?;
            check(libc::setgid(gid))?;
            check(libc::setuid(uid))
        });
    }
    command.status()
}

/// A user account from /etc/passwd and /etc/group.
struct Account {
    uid: libc::uid_t,
    gid: libc::gid_t,
    home: String,
    groups: Vec<libc::gid_t>,
}

impl Account {
    fn find(name: &str) -> io::Result<Account> {
        let c_name = CString::new(name)?;
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut buf = vec![0 as libc::c_char; 4096];
        let mut found = std::ptr::null_mut();
        let error = unsafe {
            libc::getpwnam_r(
                c_name.as_ptr(),
                &mut entry,
                buf.as_mut_ptr(),
                buf.len(),
                &mut found,
            )
        };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        if found.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("there is no user {name}"),
            ));
        }
        let home = unsafe { CStr::from_ptr(entry.pw_dir) }
            .to_string_lossy()
            .into_owned();
        let mut groups: Vec<libc::gid_t> = vec![0; 64];
        loop {
            let mut count = groups.len() as libc::c_int;
            let result = unsafe {
                libc::getgrouplist(
                    c_name.as_ptr(),
                    entry.pw_gid,
                    groups.as_mut_ptr(),
                    &mut count,
                )
            };
            if result >= 0 {
                groups.truncate(count as usize);
                break;
            }
            // Too small; count now holds the number needed.
            groups.resize(count.max(groups.len() as libc::c_int * 2) as usize, 0);
        }
        Ok(Account {
            uid: entry.pw_uid,
            gid: entry.pw_gid,
            home,
            groups,
        })
    }
}

/// Opens the console without making it the supervisor's controlling terminal: systemd
/// makes the supervisor a session leader, which would otherwise take it.
fn open_console() -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY)
        .open(CONSOLE)
}

/// Shows a status line on the console and logs it to the journal.
fn say(text: &str) {
    eprintln!("{text}");
    if let Ok(mut console) = open_console() {
        let _ = writeln!(console, "\n{text}");
    }
}

fn ended(status: ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exited with {code}"),
        (None, Some(signal)) => format!("was ended by signal {signal}"),
        _ => "ended".into(),
    }
}

/// Shuts the guest down cleanly; systemd ends the session with the supervisor's services.
fn power_off() {
    match Command::new("systemctl")
        .args(["poweroff", "--no-block"])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("systemctl poweroff {}", ended(status)),
        Err(error) => eprintln!("Couldn't run systemctl poweroff: {error}"),
    }
}

/// Listens on the command plane's port for the app. Only root may bind it.
fn listen() -> io::Result<OwnedFd> {
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut address: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    address.svm_family = libc::AF_VSOCK as libc::sa_family_t;
    address.svm_port = PORT;
    address.svm_cid = libc::VMADDR_CID_ANY;
    let bound = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&raw const address).cast(),
            size_of::<libc::sockaddr_vm>() as libc::socklen_t,
        )
    };
    if bound < 0 || unsafe { libc::listen(socket.as_raw_fd(), 1) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(socket)
}

fn accept(listener: &OwnedFd) -> io::Result<File> {
    let fd = unsafe {
        libc::accept4(
            listener.as_raw_fd(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            libc::SOCK_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
