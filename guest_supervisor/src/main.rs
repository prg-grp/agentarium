//! terrarium-supervisor: the only thing in the guest that starts a session.
//!
//! systemd starts it at boot. It listens on the command plane's vsock port, says Hello
//! to the app, and waits for Start, which decides what this boot runs on the session
//! console (`hvc1`, the app's Terminal tab): the agent's command as `ubuntu` (agent mode),
//! a root shell (maintenance mode), or the provisioning script (provision mode). Until
//! then, nothing logs in or runs there. Stop shuts the guest down; Resize passes on the
//! size of the app's terminal view.
//!
//! The supervisor logs with `tracing` to stdout, which systemd sends to the journal and
//! to the kernel console (`hvc0`, the app's read-only Logs tab). Logs name programs and
//! variables but never contain argument or variable values or the provisioning script.
//! Built for the Linux guest only (see the Makefile).

use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use command_plane::{
    EnvVar, GuestMessage, Hello, HostMessage, Mode, Mount, PORT, PROTOCOL_VERSION,
    ProvisionFinished, SHARE_TAG, Start, StartFailed, guest_message, host_message,
};
use tracing::{debug, error, info, info_span, warn};

/// The session's virtio console, shown in the app's Terminal tab. `hvc0` is the kernel
/// console, shown in the Logs tab.
const SESSION_TTY: &str = "/dev/hvc1";
/// How long to wait before starting an exited session again.
const RESTART_DELAY: Duration = Duration::from_secs(2);
/// Where the provisioning script is written; a tmpfs, gone after the boot.
const RUN_DIR: &str = "/run/terrarium";
/// Where the virtio-fs share with all shared folders is mounted. Root only: the session
/// reaches each folder only where it is mounted.
const SHARES_DIR: &str = "/run/terrarium/shares";

/// The app's current connection. Provision mode reports its result on it.
type Channel = Arc<Mutex<Option<File>>>;

fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .init();
    info!(
        version = env!("CARGO_PKG_VERSION"),
        protocol = PROTOCOL_VERSION,
        "terrarium-supervisor starting"
    );
    // Without the session console, no mode can run; systemd starts the supervisor again.
    let tty = match Tty::new() {
        Ok(tty) => Arc::new(tty),
        Err(error) => {
            error!(path = SESSION_TTY, %error, "couldn't open the session console");
            std::process::exit(1);
        }
    };
    info!(path = SESSION_TTY, "found the session console");

    // The vsock transport module may still be loading early in the boot.
    let mut attempt = 1;
    let listener = loop {
        match listen() {
            Ok(listener) => break listener,
            Err(error) if attempt == 1 => {
                warn!(port = PORT, %error, "couldn't listen on vsock, trying again every second");
            }
            Err(error) => debug!(port = PORT, attempt, %error, "couldn't listen on vsock"),
        }
        attempt += 1;
        thread::sleep(Duration::from_secs(1));
    };
    info!(port = PORT, attempt, "listening for the app on vsock");

    let channel = Channel::default();
    let mut started = false;
    loop {
        match accept(&listener) {
            Ok((connection, cid, port)) => info_span!("connection", cid, port)
                .in_scope(|| serve(connection, &channel, &mut started, &tty)),
            Err(error) => {
                warn!(%error, "couldn't accept a connection");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

/// Says Hello on a new connection from the app, then follows its messages until it closes.
/// The session keeps running when the connection closes.
fn serve(connection: File, channel: &Channel, started: &mut bool, tty: &Arc<Tty>) {
    info!("the app connected");
    let mut reader = match connection.try_clone() {
        Ok(reader) => reader,
        Err(error) => return warn!(%error, "couldn't read the connection"),
    };
    *channel.lock().unwrap() = Some(connection);
    let hello = guest_message::Kind::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
    });
    if send(channel, hello) {
        debug!("sent Hello");
    }
    loop {
        let message: HostMessage = match command_plane::read_message(&mut reader) {
            Ok(message) => message,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                info!("the app disconnected");
                break;
            }
            Err(error) => {
                warn!(%error, "lost the connection to the app");
                break;
            }
        };
        match message.kind {
            Some(host_message::Kind::Start(start)) if !*started => {
                info!(mode = ?start.mode(), "received Start");
                *started = true;
                let (channel, tty) = (channel.clone(), tty.clone());
                thread::spawn(move || run(start, &channel, &tty));
            }
            Some(host_message::Kind::Start(start)) => {
                warn!(mode = ?start.mode(), "ignoring Start: this boot has already started");
            }
            Some(host_message::Kind::Stop(_)) => {
                info!("received Stop, powering off");
                power_off();
            }
            Some(host_message::Kind::Resize(resize)) => {
                debug!(
                    columns = resize.columns,
                    rows = resize.rows,
                    "received Resize"
                );
                if let Err(error) = tty.resize(resize.columns, resize.rows) {
                    warn!(%error, "couldn't resize the session console");
                }
            }
            None => warn!("ignoring a message this supervisor doesn't know"),
        }
    }
    channel.lock().unwrap().take();
}

/// Sends a message to the app. Returns whether it was sent; false if the app isn't connected.
fn send(channel: &Channel, kind: guest_message::Kind) -> bool {
    let mut channel = channel.lock().unwrap();
    let Some(connection) = channel.as_mut() else {
        return false;
    };
    match command_plane::write_message(connection, &GuestMessage { kind: Some(kind) }) {
        Ok(()) => true,
        Err(error) => {
            warn!(%error, "couldn't write to the app");
            false
        }
    }
}

/// Runs the boot mode the app chose. Agent and maintenance sessions start again
/// whenever they exit; there is no fallback to a shell.
fn run(start: Start, channel: &Channel, tty: &Tty) {
    let mode = start.mode();
    let _span = info_span!("mode", ?mode).entered();
    // Nothing starts without all of its shared folders.
    if matches!(mode, Mode::Agent | Mode::Maintenance)
        && let Err(error) = mount_shares(&start.mounts)
    {
        error!("not starting: the shared folders aren't all mounted, powering off");
        tty.notify(&format!(
            "Couldn't mount the shared folders: {error}. Nothing was started. Powering off."
        ));
        let failed = guest_message::Kind::StartFailed(StartFailed {
            reason: format!("couldn't mount the shared folders: {error}"),
        });
        if send(channel, failed) {
            debug!("sent StartFailed");
        } else {
            warn!("couldn't report the failed start: the app isn't connected");
        }
        return power_off();
    }
    match mode {
        Mode::Agent => {
            let Some(launch) = start.launch.filter(|launch| !launch.command.is_empty()) else {
                error!("the app sent no command to start");
                return tty.notify("The app sent no command to start.");
            };
            info!("starting agent mode");
            repeat(tty, &launch.command, || {
                session(
                    tty,
                    "ubuntu",
                    &launch.command,
                    &launch.arguments,
                    &launch.working_directory,
                    &start.environment,
                    true,
                )
            });
        }
        Mode::Maintenance => {
            info!("starting maintenance mode");
            repeat(tty, "The root shell", || {
                session(
                    tty,
                    "root",
                    "/bin/bash",
                    &["--login".into()],
                    "",
                    &start.environment,
                    true,
                )
            });
        }
        Mode::Provision => {
            info!("starting provision mode");
            let exit_code = provision(tty, &start.provision_script);
            if exit_code == 0 {
                info!(exit_code, "the provisioning script finished");
            } else {
                warn!(exit_code, "the provisioning script failed");
            }
            let finished = guest_message::Kind::ProvisionFinished(ProvisionFinished { exit_code });
            if send(channel, finished) {
                debug!("sent ProvisionFinished");
            } else {
                warn!("couldn't report the provisioning result: the app isn't connected");
            }
            tty.notify(&format!(
                "The provisioning script exited with {exit_code}. Powering off."
            ));
            power_off();
        }
        Mode::Unspecified => {
            error!("the app sent no mode to start");
            tty.notify("The app sent no mode to start.");
        }
    }
}

/// Mounts the virtio-fs share at `SHARES_DIR`, then binds each shared folder to its guest
/// path, parents before folders nested in them. Read-only folders are also mounted
/// read-only, though the Mac already enforces it. The mounts last for this boot only.
fn mount_shares(mounts: &[Mount]) -> io::Result<()> {
    if mounts.is_empty() {
        return Ok(());
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(SHARES_DIR)
        .and_then(|()| {
            mount(
                Path::new(SHARE_TAG),
                Path::new(SHARES_DIR),
                Some("virtiofs"),
                0,
            )
        })
        .inspect_err(|error| error!(tag = SHARE_TAG, %error, "couldn't mount the share"))?;
    info!(tag = SHARE_TAG, path = SHARES_DIR, "mounted the share");

    // Mount points the supervisor creates belong to the agent, like the rest of its home.
    let agent = Account::find("ubuntu")?;
    let mut mounts: Vec<&Mount> = mounts.iter().collect();
    mounts.sort_by_key(|mount| Path::new(&mount.guest_path).components().count());
    for folder in mounts {
        let (share, guest_path, read_only) = (&folder.share, &folder.guest_path, folder.read_only);
        let fail = |step: &str, error: io::Error| {
            error!(%share, %guest_path, %error, "couldn't {step}");
            io::Error::new(
                error.kind(),
                format!("{guest_path}: couldn't {step}: {error}"),
            )
        };
        // The agent may have left a symbolic link at the path on the persistent disk;
        // following it would mount the folder somewhere else.
        let mut target = PathBuf::from("/");
        for part in Path::new(guest_path).components() {
            let Component::Normal(part) = part else {
                continue;
            };
            target.push(part);
            match fs::symlink_metadata(&target) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(fail(
                        "create the mount point",
                        io::Error::other(format!("{} is a symbolic link", target.display())),
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&target)
                        .and_then(|()| {
                            std::os::unix::fs::chown(&target, Some(agent.uid), Some(agent.gid))
                        })
                        .map_err(|error| fail("create the mount point", error))?;
                }
                Err(error) => return Err(fail("create the mount point", error)),
            }
        }
        let source = Path::new(SHARES_DIR).join(share);
        mount(&source, &target, None, libc::MS_BIND)
            .map_err(|error| fail("bind the shared folder", error))?;
        if read_only {
            mount(
                &source,
                &target,
                None,
                libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY,
            )
            .map_err(|error| fail("make it read-only", error))?;
        }
        info!(%share, %guest_path, read_only, "mounted a shared folder");
    }
    Ok(())
}

/// mount(2).
fn mount(
    source: &Path,
    target: &Path,
    fstype: Option<&str>,
    flags: libc::c_ulong,
) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    let fstype = fstype.map(CString::new).transpose()?;
    let mounted = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            fstype
                .as_ref()
                .map_or(std::ptr::null(), |fstype| fstype.as_ptr()),
            flags,
            std::ptr::null(),
        )
    };
    if mounted < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Starts a session again and again, a little after each exit, and tells the user so on
/// the session console.
fn repeat(tty: &Tty, name: &str, start: impl Fn() -> io::Result<ExitStatus>) -> ! {
    loop {
        match start() {
            Ok(status) => {
                let ended = ended(status);
                info!(session = name, status = %ended, "the session ended");
                tty.notify(&format!("{name} {ended}. Starting it again."));
            }
            Err(error) => {
                error!(session = name, %error, "couldn't start the session");
                tty.notify(&format!("Couldn't start {name}: {error}. Trying again."));
            }
        }
        debug!(delay = ?RESTART_DELAY, "starting the session again after a delay");
        thread::sleep(RESTART_DELAY);
    }
}

/// Writes the script to the run folder and runs it as root, with its output on the session
/// console. Returns its exit code, or -1.
fn provision(tty: &Tty, script: &str) -> i32 {
    let path = format!("{RUN_DIR}/provision.sh");
    let written = fs::create_dir_all(RUN_DIR)
        .and_then(|()| fs::write(&path, script))
        .and_then(|()| fs::set_permissions(&path, fs::Permissions::from_mode(0o700)));
    if let Err(error) = written {
        error!(path, %error, "couldn't write the provisioning script");
        tty.notify(&format!("Couldn't write the provisioning script: {error}"));
        return -1;
    }
    info!(path, bytes = script.len(), "wrote the provisioning script");
    match session(tty, "root", "/bin/bash", &[path], "", &[], false) {
        Ok(status) => status.code().unwrap_or(-1),
        Err(error) => {
            error!(%error, "couldn't run the provisioning script");
            tty.notify(&format!("Couldn't run the provisioning script: {error}"));
            -1
        }
    }
}

/// Runs `program` as `user` on the session console and waits for it. An interactive
/// session reads the console and gets it as its controlling terminal; otherwise stdin is
/// empty. An empty `directory` means the user's home folder.
fn session(
    tty: &Tty,
    user: &str,
    program: &str,
    arguments: &[String],
    directory: &str,
    environment: &[EnvVar],
    interactive: bool,
) -> io::Result<ExitStatus> {
    let account = Account::find(user)
        .inspect_err(|error| error!(user, %error, "couldn't look up the user"))?;
    let console = tty.prepare()?;
    let home = account.home.as_str();
    let directory = if directory.is_empty() {
        home
    } else {
        directory
    };
    info!(
        user,
        program,
        directory,
        arguments = arguments.len(),
        environment = ?environment.iter().map(|var| var.name.as_str()).collect::<Vec<_>>(),
        interactive,
        "starting a session"
    );
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
        .current_dir(directory)
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

/// The session console. It is opened anew for each session: when a session leader exits,
/// the kernel hangs the console up, which ends every descriptor open on it. So the
/// console's initial settings and the app's latest size are kept here and applied to
/// each new session.
struct Tty {
    /// The settings the console had at boot, before any session changed them.
    termios: libc::termios,
    /// The app's latest terminal size.
    size: Mutex<Option<libc::winsize>>,
}

impl Tty {
    fn new() -> io::Result<Tty> {
        let file = Tty::open()?;
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(file.as_raw_fd(), &mut termios) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Tty {
            termios,
            size: Mutex::new(None),
        })
    }

    /// Opens the console without making it the supervisor's controlling terminal: systemd
    /// makes the supervisor a session leader, which would otherwise take it.
    fn open() -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(SESSION_TTY)
    }

    /// Opens the console for a new session: drops input typed while no session ran, and
    /// restores the boot settings and the latest size, so nothing a previous session left
    /// behind carries over.
    fn prepare(&self) -> io::Result<File> {
        let file = Tty::open()?;
        let fd = file.as_raw_fd();
        let size = self.size.lock().unwrap();
        let prepared = unsafe {
            libc::tcflush(fd, libc::TCIFLUSH) >= 0
                && libc::tcsetattr(fd, libc::TCSANOW, &self.termios) >= 0
                && size
                    .as_ref()
                    .is_none_or(|size| libc::ioctl(fd, libc::TIOCSWINSZ, size) >= 0)
        };
        if !prepared {
            warn!(error = %io::Error::last_os_error(), "couldn't reset the session console");
        }
        Ok(file)
    }

    /// Keeps the size for later sessions and applies it to the current one; the kernel
    /// tells the session's programs (SIGWINCH).
    fn resize(&self, columns: u32, rows: u32) -> io::Result<()> {
        let size = libc::winsize {
            ws_row: rows.try_into().unwrap_or(u16::MAX),
            ws_col: columns.try_into().unwrap_or(u16::MAX),
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let mut kept = self.size.lock().unwrap();
        *kept = Some(size);
        let file = Tty::open()?;
        if unsafe { libc::ioctl(file.as_raw_fd(), libc::TIOCSWINSZ, &size) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Shows a line to the user on the session console.
    fn notify(&self, text: &str) {
        match Tty::open() {
            // The console may still be in raw mode, so return the carriage explicitly.
            Ok(mut file) => {
                let _ = write!(file, "\r\n{text}\r\n");
            }
            Err(error) => warn!(%error, "couldn't write to the session console"),
        }
    }
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
        Ok(status) => error!(status = %ended(status), "systemctl poweroff failed"),
        Err(error) => error!(%error, "couldn't run systemctl poweroff"),
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

/// Accepts the next connection; returns it with the peer's vsock CID and port.
fn accept(listener: &OwnedFd) -> io::Result<(File, u32, u32)> {
    let mut address: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    let mut length = size_of::<libc::sockaddr_vm>() as libc::socklen_t;
    let fd = unsafe {
        libc::accept4(
            listener.as_raw_fd(),
            (&raw mut address).cast(),
            &mut length,
            libc::SOCK_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((
        unsafe { File::from_raw_fd(fd) },
        address.svm_cid,
        address.svm_port,
    ))
}
