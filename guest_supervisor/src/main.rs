//! agentarium-supervisor: the only thing in the guest that starts a session.
//!
//! systemd starts it at boot. It listens on the command plane's vsock port, says Hello
//! to the app, and waits for Start, which decides what this boot runs on the session
//! console (`hvc1`, the app's Terminal tab): the agent's command as `ubuntu` (agent mode),
//! a root shell (maintenance mode), or the provisioning script (provision mode). Until
//! then, nothing logs in or runs there. Stop shuts the guest down; Resize passes on the
//! size of the app's terminal view.
//!
//! In agent mode the agent runs in a cgroup of its own, and the supervisor reports Status
//! (the guest's and the agent's resource use, and whether the agent runs) every
//! STATUS_INTERVAL and whenever the agent starts or exits. Before the agent starts, it
//! starts the observer (agentarium-observer) for that cgroup and keeps it running; the
//! observer streams what the agent does to the app over the data plane. Maintenance and
//! provision boots aren't observed: nothing starts the observer there.
//!
//! In agent and maintenance mode it also accepts forwarded ports from the app on their
//! own vsock port and connects each to the guest port it names, on the guest's loopback.
//!
//! The supervisor logs with `tracing` to stdout, which systemd sends to the journal and
//! to the kernel console (`hvc0`, the app's read-only Logs tab). Logs name programs and
//! variables but never contain argument or variable values or the provisioning script.
//! Built for the Linux guest only (see the Makefile).

use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, TcpStream};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use command_plane::vsock::{accept, listen};
use command_plane::{
    AgentExit, AgentUsage, CollectorState, EnvVar, FORWARD_PORT, Forward, GuestMessage, Hello,
    HostMessage, Mode, Mount, PORT, PROTOCOL_VERSION, ProvisionFinished, SHARE_TAG,
    STATUS_INTERVAL, Start, StartFailed, Status, guest_message, host_message, read_message,
};
use tracing::{debug, error, info, info_span, warn};

/// The session's virtio console, shown in the app's Terminal tab. `hvc0` is the kernel
/// console, shown in the Logs tab.
const SESSION_TTY: &str = "/dev/hvc1";
/// How long to wait before starting an exited session again.
const RESTART_DELAY: Duration = Duration::from_secs(2);
/// Where the provisioning script is written; a tmpfs, gone after the boot.
const RUN_DIR: &str = "/run/agentarium";
/// Where the virtio-fs share with all shared folders is mounted. Root only: the session
/// reaches each folder only where it is mounted.
const SHARES_DIR: &str = "/run/agentarium/shares";
/// The observer, baked into the base rootfs beside the supervisor.
const OBSERVER: &str = "/usr/local/sbin/agentarium-observer";
/// How long the agent's start waits for the observer to attach its programs.
const OBSERVER_READY: Duration = Duration::from_secs(10);

/// The app's current connection. Provision mode reports its result on it.
type Channel = Arc<Mutex<Option<File>>>;

/// The agent's state during an agent-mode boot, as Status reports it.
#[derive(Default)]
struct AgentState {
    started: bool,
    running: bool,
    restarts: u32,
    last_exit: Option<AgentExit>,
    /// Whether the observer runs.
    collector: CollectorState,
}

/// The agent's cgroup (v2), beside the supervisor's own in the service's cgroup, which
/// systemd delegates to it (`Delegate=` in agentarium-supervisor.service).
struct AgentCgroup {
    dir: PathBuf,
    /// Its `cgroup.procs`; a new session writes to it before exec, which moves it in.
    procs: File,
}

impl AgentCgroup {
    fn create() -> io::Result<AgentCgroup> {
        let own = fs::read_to_string("/proc/self/cgroup")?;
        let path = own
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .ok_or_else(|| io::Error::other("the guest has no cgroup v2 hierarchy"))?;
        // systemd starts the supervisor in a subgroup of the service's cgroup
        // (`DelegateSubgroup=`), because a cgroup that hands controllers to its children
        // can't hold processes itself.
        let own = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
        let base = own
            .parent()
            .ok_or_else(|| io::Error::other("the supervisor runs in the root cgroup"))?;
        fs::write(base.join("cgroup.subtree_control"), "+cpu +memory")?;
        let dir = base.join("agent");
        fs::create_dir_all(&dir)?;
        let procs = OpenOptions::new()
            .write(true)
            .open(dir.join("cgroup.procs"))?;
        Ok(AgentCgroup { dir, procs })
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .init();
    info!(
        version = env!("CARGO_PKG_VERSION"),
        protocol = PROTOCOL_VERSION,
        "agentarium-supervisor starting"
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
        match listen(PORT) {
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

/// Accepts the app's forwarded ports for this boot. Each connection names its guest port in
/// a Forward and is then relayed to that port on the guest's loopback, if the boot's Start
/// allows the port.
fn forward(ports: Vec<u32>) {
    let listener = match listen(FORWARD_PORT) {
        Ok(listener) => listener,
        Err(error) => {
            return error!(port = FORWARD_PORT, %error, "couldn't listen for forwarded ports");
        }
    };
    info!(port = FORWARD_PORT, ?ports, "listening for forwarded ports");
    loop {
        let connection = match accept(&listener) {
            Ok((connection, ..)) => connection,
            Err(error) => {
                warn!(%error, "couldn't accept a forwarded connection");
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let ports = ports.clone();
        thread::spawn(move || {
            let mut connection = connection;
            let port = match read_message::<Forward>(&mut connection) {
                Ok(request) => request.port,
                Err(error) => return debug!(%error, "a forwarded connection sent no port"),
            };
            if !ports.contains(&port) {
                return warn!(port, "refused a forward to a port that isn't forwarded");
            }
            let Ok(port16) = u16::try_from(port) else {
                return;
            };
            // Servers bound to `localhost` may listen on ::1 only.
            let target = TcpStream::connect((Ipv4Addr::LOCALHOST, port16))
                .or_else(|_| TcpStream::connect((Ipv6Addr::LOCALHOST, port16)));
            match target {
                Ok(target) => relay(connection, target),
                Err(error) => debug!(port, %error, "nothing accepts the forwarded port"),
            }
        });
    }
}

/// Copies both directions between a forwarded connection and its guest port until both
/// ends are done, passing on each end's close of its sending side.
fn relay(connection: File, target: TcpStream) {
    let (Ok(mut from_app), Ok(mut to_target)) = (connection.try_clone(), target.try_clone()) else {
        return;
    };
    let inbound = thread::spawn(move || {
        let _ = io::copy(&mut from_app, &mut to_target);
        let _ = to_target.shutdown(Shutdown::Write);
    });
    let (mut from_target, mut to_app) = (target, connection);
    let _ = io::copy(&mut from_target, &mut to_app);
    unsafe { libc::shutdown(to_app.as_raw_fd(), libc::SHUT_WR) };
    let _ = inbound.join();
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
    if matches!(mode, Mode::Agent | Mode::Maintenance) && !start.forward_ports.is_empty() {
        let ports = start.forward_ports.clone();
        thread::spawn(move || forward(ports));
    }
    match mode {
        Mode::Agent => {
            let Some(launch) = start.launch.filter(|launch| !launch.command.is_empty()) else {
                error!("the app sent no command to start");
                return tty.notify("The app sent no command to start.");
            };
            info!("starting agent mode");
            let cgroup = AgentCgroup::create()
                .inspect_err(|error| {
                    warn!(%error, "couldn't set up the agent's cgroup; its resource use isn't reported");
                })
                .ok();
            let cgroup_dir = cgroup.as_ref().map(|cgroup| cgroup.dir.clone());
            let agent = Arc::new(Mutex::new(AgentState::default()));
            let report = {
                let (channel, agent, cgroup_dir) =
                    (channel.clone(), agent.clone(), cgroup_dir.clone());
                move || {
                    let status = status(&agent.lock().unwrap(), cgroup_dir.as_deref());
                    send(&channel, guest_message::Kind::Status(status));
                }
            };
            let periodic = report.clone();
            thread::spawn(move || {
                loop {
                    thread::sleep(STATUS_INTERVAL);
                    periodic();
                }
            });
            // Without its own cgroup, the agent can't be told apart from the rest of the
            // guest, so it runs unobserved.
            match &cgroup_dir {
                Some(dir) => observe(dir, &agent),
                None => {
                    warn!("not observing the agent: it has no cgroup");
                    agent.lock().unwrap().collector = CollectorState::Stopped;
                }
            }
            repeat(tty, &launch.command, || {
                {
                    let mut state = agent.lock().unwrap();
                    state.restarts += u32::from(state.started);
                    state.started = true;
                    state.running = true;
                }
                report();
                let result = session(
                    tty,
                    "ubuntu",
                    &launch.command,
                    &launch.arguments,
                    &launch.working_directory,
                    &start.environment,
                    true,
                    cgroup.as_ref().map(|cgroup| &cgroup.procs),
                );
                {
                    let mut state = agent.lock().unwrap();
                    state.running = false;
                    if let Ok(status) = &result {
                        state.last_exit = Some(AgentExit {
                            code: status.code().unwrap_or(-1),
                            signal: status.signal().unwrap_or(0),
                        });
                    }
                }
                report();
                result
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
                    None,
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

/// Starts the observer for the agent's cgroup and waits until its programs are attached, at
/// most OBSERVER_READY. A thread keeps it running: whenever it exits, it is started again
/// after RESTART_DELAY. The agent starts either way; Status tells whether it is observed.
fn observe(cgroup: &Path, agent: &Arc<Mutex<AgentState>>) {
    if !Path::new(OBSERVER).exists() {
        warn!(
            path = OBSERVER,
            "not observing the agent: the observer isn't installed"
        );
        agent.lock().unwrap().collector = CollectorState::NotInstalled;
        return;
    }
    agent.lock().unwrap().collector = CollectorState::Stopped;
    let (ready, waited) = mpsc::channel();
    let (cgroup, agent) = (cgroup.to_owned(), agent.clone());
    thread::spawn(move || {
        let collector = |state| agent.lock().unwrap().collector = state;
        loop {
            // It runs in the supervisor's cgroup, not the agent's, so it doesn't observe
            // itself. Its logs go where the supervisor's go.
            let started = Command::new(OBSERVER)
                .arg("--cgroup")
                .arg(&cgroup)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .spawn();
            match started {
                Ok(mut child) => {
                    info!(pid = child.id(), "started the observer");
                    // It prints `ready` once its programs are attached, or exits.
                    let attached = child.stdout.take().is_some_and(|stdout| {
                        BufReader::new(stdout)
                            .lines()
                            .next()
                            .is_some_and(|line| line.is_ok_and(|line| line == "ready"))
                    });
                    if attached {
                        info!("the observer is ready");
                        collector(CollectorState::Running);
                    }
                    let _ = ready.send(());
                    let status = child.wait();
                    collector(CollectorState::Stopped);
                    match status {
                        Ok(status) => warn!(status = %ended(status), "the observer stopped"),
                        Err(error) => warn!(%error, "lost the observer"),
                    }
                }
                Err(error) => error!(%error, "couldn't start the observer"),
            }
            let _ = ready.send(());
            debug!(delay = ?RESTART_DELAY, "starting the observer again after a delay");
            thread::sleep(RESTART_DELAY);
        }
    });
    if waited.recv_timeout(OBSERVER_READY).is_err() {
        warn!("the observer isn't ready yet; starting the agent without waiting");
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
    match session(tty, "root", "/bin/bash", &[path], "", &[], false, None) {
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
/// empty. An empty `directory` means the user's home folder. With `cgroup` (a
/// `cgroup.procs` file), the session runs in that cgroup.
#[allow(clippy::too_many_arguments)]
fn session(
    tty: &Tty,
    user: &str,
    program: &str,
    arguments: &[String],
    directory: &str,
    environment: &[EnvVar],
    interactive: bool,
    cgroup: Option<&File>,
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
    let cgroup = cgroup.map(AsRawFd::as_raw_fd);
    // Runs in the child between fork and exec, so only async-signal-safe calls. The new
    // session takes the console and joins its cgroup while still root, so it can take the
    // console over from a session that left processes behind.
    unsafe {
        command.pre_exec(move || {
            let check = |result: libc::c_int| {
                if result < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            };
            // "0" moves the writing process. Without the move the session still runs;
            // its use just isn't counted.
            if let Some(fd) = cgroup {
                libc::write(fd, b"0".as_ptr().cast(), 1);
            }
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

/// Takes a Status report: the guest's counters, the agent's state, and the usage of the
/// agent's cgroup in `cgroup`. A counter that can't be read is 0.
fn status(agent: &AgentState, cgroup: Option<&Path>) -> Status {
    let read = |path: &Path| fs::read_to_string(path).unwrap_or_default();
    // The number after `key` on its line, in files like /proc/meminfo or cpu.stat.
    let field = |text: &str, key: &str| -> u64 {
        text.lines()
            .find_map(|line| {
                line.strip_prefix(key)?
                    .split_whitespace()
                    .next()?
                    .parse()
                    .ok()
            })
            .unwrap_or(0)
    };
    let meminfo = read(Path::new("/proc/meminfo"));
    let (total, available) = (
        field(&meminfo, "MemTotal:") * 1024,
        field(&meminfo, "MemAvailable:") * 1024,
    );
    // The first line adds up all CPUs' ticks by kind: user nice system idle iowait irq
    // softirq steal (guest time is already in user). Everything but idle and iowait is busy.
    let ticks: u64 = read(Path::new("/proc/stat"))
        .lines()
        .next()
        .map(|line| {
            line.split_whitespace()
                .skip(1)
                .take(8)
                .enumerate()
                .filter(|(kind, _)| !matches!(kind, 3 | 4))
                .filter_map(|(_, value)| value.parse::<u64>().ok())
                .sum()
        })
        .unwrap_or(0);
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    let mut now: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
    Status {
        monotonic_usec: now.tv_sec as u64 * 1_000_000 + now.tv_nsec as u64 / 1_000,
        vm_cpu_usec: ticks * 1_000_000 / hz,
        vm_memory_used_bytes: total.saturating_sub(available),
        vm_memory_total_bytes: total,
        agent_running: agent.running,
        agent_restarts: agent.restarts,
        agent_last_exit: agent.last_exit,
        // Like the VM's figure, the agent's memory leaves out cache the kernel can reclaim.
        agent_usage: cgroup.map(|dir| AgentUsage {
            cpu_usec: field(&read(&dir.join("cpu.stat")), "usage_usec "),
            memory_bytes: read(&dir.join("memory.current"))
                .trim()
                .parse::<u64>()
                .unwrap_or(0)
                .saturating_sub(field(&read(&dir.join("memory.stat")), "inactive_file ")),
            oom_kills: field(&read(&dir.join("memory.events")), "oom_kill "),
        }),
        collector: agent.collector.into(),
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
