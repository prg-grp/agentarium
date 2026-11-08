//! Runs the instances' VMs, any number at once.
//!
//! Each boot gets its own controller thread, which creates the VM, starts it, watches its
//! state and stops it; the VM itself operates on its own dispatch queue (see `vm.rs`).
//! Controllers report to `VmControl` on the UI thread, which owns the database and keeps
//! the sidebar rows and each instance's two terminals (session and boot log) in sync.
//!
//! The controller also holds the command plane to the guest supervisor: it connects once
//! the VM runs, answers the supervisor's Hello with Start (the boot mode), and sends the
//! session terminal's size (Resize) and Stop. In agent mode it passes the supervisor's
//! Status reports on, with CPU use computed from two reports, and notices when they stop
//! arriving. Only the latest report is kept; nothing is stored.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use command_plane::{
    AgentExit, CollectorState, EnvVar, GuestMessage, HostMessage, Launch, Mode, Mount, PORT,
    PROTOCOL_VERSION, Resize, Start, Status, Stop, guest_message, host_message,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::db::{self, Db};
use crate::editor::{FolderProblem, display_path, env_errors, folder_problem, guest_path_errors};
use crate::policy::Policy;
use crate::terminal::{Console, Terminal};
use crate::vm::{BootMode, SharedFolder, Vm, VmSpec, VmState};
use crate::{AppWindow, Deletion, InstanceItem, InstanceState, StatusReading, Tone, Usage};

const STOPPED_MESSAGE: &str = "The virtual machine is stopped. Start it to open its console.";

/// How long the guest gets to shut down before it is stopped hard.
const STOP_TIMEOUT: Duration = Duration::from_secs(20);

/// How often a controller reads its VM's state, so a shutdown from inside the guest is noticed.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How long the guest supervisor gets from the VM's start to its first Hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(60);

/// How long to wait before connecting to the supervisor again while it doesn't listen yet.
const CONNECT_RETRY: Duration = Duration::from_millis(250);

/// How long an agent-mode boot may go without a Status report before it counts as lost.
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// The shortest interval CPU use is computed over. Reports sent when the agent starts or
/// exits can follow the previous one closely; they don't move the baseline.
const CPU_WINDOW: Duration = Duration::from_secs(1);

/// CPU use between two Status reports, as a share of all vCPUs (0 to 1).
#[derive(Clone, Copy)]
struct Cpu {
    vm: f32,
    /// None if the agent's cgroup isn't measured.
    agent: Option<f32>,
}

/// Messages to a controller thread: from the UI, its VM's completion handlers, and the
/// command plane's reader thread.
enum Command {
    /// Asks the guest to shut down; it is stopped hard after `STOP_TIMEOUT`.
    Stop,
    Started(Result<(), String>),
    ForceStopped(Result<(), String>),
    /// A connection attempt to the supervisor finished.
    Connected(Result<File, String>),
    /// A message from the supervisor.
    Guest(GuestMessage),
    /// The supervisor's connection closed.
    Disconnected,
    /// The session terminal changed size.
    Resize {
        columns: u32,
        rows: u32,
    },
}

/// What a controller thread reports to the UI thread.
enum Event {
    /// The VM is configured and starting; these are the host ends of its consoles.
    Created {
        session: Console,
        log: Console,
    },
    State(VmState),
    /// The guest supervisor got Start, so the session is about to run.
    Ready,
    /// The provisioning script finished with this exit code.
    Provisioned(i32),
    /// The supervisor's latest Status, with the CPU use since an earlier one if enough
    /// time has passed.
    Status(Status, Option<Cpu>),
    /// No Status arrived for `STATUS_TIMEOUT`.
    StatusLost,
    /// The controller ended: the VM stopped, or failed with the error.
    Ended(Result<(), String>),
}

/// The command plane connection to the guest supervisor. Dropping it shuts the socket
/// down, which ends its reader thread.
struct Channel(File);

impl Channel {
    /// Reads the supervisor's messages on a thread of their own and passes them on to the
    /// controller as `Command::Guest`, then `Command::Disconnected` once the connection ends.
    fn open(socket: File, commands: &Sender<Command>) -> io::Result<Channel> {
        let mut reader = socket.try_clone()?;
        let commands = commands.clone();
        std::thread::Builder::new()
            .name("vm-command-plane".into())
            .spawn(move || {
                loop {
                    match command_plane::read_message::<GuestMessage>(&mut reader) {
                        Ok(message) => {
                            if commands.send(Command::Guest(message)).is_err() {
                                return;
                            }
                        }
                        Err(error) => {
                            if error.kind() != io::ErrorKind::UnexpectedEof {
                                eprintln!("Couldn't read from the guest supervisor: {error}");
                            }
                            let _ = commands.send(Command::Disconnected);
                            return;
                        }
                    }
                }
            })?;
        Ok(Channel(socket))
    }

    fn send(&mut self, kind: host_message::Kind) -> io::Result<()> {
        command_plane::write_message(&mut self.0, &HostMessage { kind: Some(kind) })
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        unsafe { libc::shutdown(self.0.as_raw_fd(), libc::SHUT_RDWR) };
    }
}

/// Runs a VM configured from `spec` on a new thread until it stops; its guest supervisor
/// gets `start`. Returns the sender for asking it to stop.
fn spawn(
    spec: VmSpec,
    start: Start,
    report: impl Fn(Event) + Send + 'static,
) -> std::io::Result<Sender<Command>> {
    let (commands, received) = mpsc::channel();
    let handlers = commands.clone();
    std::thread::Builder::new()
        .name("vm-control".into())
        .spawn(move || {
            report(Event::Ended(run(
                &spec, start, &handlers, &received, &report,
            )))
        })?;
    Ok(commands)
}

/// The controller thread's loop; returns once the VM has stopped.
fn run(
    spec: &VmSpec,
    start: Start,
    commands: &Sender<Command>,
    received: &Receiver<Command>,
    report: &impl Fn(Event),
) -> Result<(), String> {
    let (vm, session, log) = Vm::create(spec)?;
    let resized = commands.clone();
    let session = session.with_resize(move |columns, rows| {
        let _ = resized.send(Command::Resize {
            columns: columns.into(),
            rows: rows.into(),
        });
    });
    report(Event::Created { session, log });
    let started = commands.clone();
    vm.start(move |result| {
        let _ = started.send(Command::Started(result));
    });

    // Set once the guest was asked to shut down; it is stopped hard after that.
    let mut deadline: Option<Instant> = None;
    let mut forced = false;
    let mut force_stop = |vm: &Vm| {
        if !forced {
            forced = true;
            let stopped = commands.clone();
            vm.stop(move |result| {
                let _ = stopped.send(Command::ForceStopped(result));
            });
        }
    };
    // The command plane. Once the VM runs, the controller connects to the supervisor,
    // again every CONNECT_RETRY until it listens, and after the connection closes.
    let mut channel: Option<Channel> = None;
    let mut connecting = false;
    let mut next_connect: Option<Instant> = None;
    // Until the supervisor's first Hello; the boot fails without one.
    let mut hello_deadline: Option<Instant> = None;
    // Why the VM is being stopped, if the boot failed while it ran.
    let mut failure: Option<String> = None;
    // The session terminal's latest size, sent again on every new connection.
    let mut size: Option<Resize> = None;
    // Agent mode only: when the last Status arrived (or Start was sent), whether it's
    // overdue, and the report CPU use is computed against.
    let agent_mode = start.mode() == Mode::Agent;
    let mut last_status: Option<Instant> = None;
    let mut status_lost = false;
    let mut baseline: Option<Status> = None;
    let mut last = VmState::Starting;
    loop {
        let wait = [deadline, next_connect]
            .into_iter()
            .flatten()
            .map(|at| at.saturating_duration_since(Instant::now()))
            .fold(POLL_INTERVAL, Duration::min);
        match received.recv_timeout(wait) {
            Ok(Command::Stop) if deadline.is_none() => {
                hello_deadline = None;
                last_status = None;
                deadline = Some(Instant::now() + STOP_TIMEOUT);
                // Before the supervisor answers, the power button asks the guest instead.
                let sent = channel
                    .as_mut()
                    .map(|channel| channel.send(host_message::Kind::Stop(Stop {})));
                if !matches!(sent, Some(Ok(())))
                    && let Err(error) = vm.request_stop()
                {
                    eprintln!("Couldn't ask the virtual machine to stop: {error}");
                    force_stop(&vm);
                }
            }
            Ok(Command::Started(Ok(()))) => {
                next_connect = Some(Instant::now());
                hello_deadline = Some(Instant::now() + HELLO_TIMEOUT);
            }
            Ok(Command::Started(Err(error))) => return Err(error),
            Ok(Command::ForceStopped(Err(error))) => {
                return Err(format!("couldn't stop it: {error}"));
            }
            Ok(Command::Connected(connected)) => {
                connecting = false;
                match connected.and_then(|socket| {
                    Channel::open(socket, commands).map_err(|error| error.to_string())
                }) {
                    Ok(opened) => channel = Some(opened),
                    // Nothing listens yet while the guest boots.
                    Err(_) => next_connect = Some(Instant::now() + CONNECT_RETRY),
                }
            }
            Ok(Command::Guest(message)) => match message.kind {
                Some(guest_message::Kind::Hello(hello)) => {
                    hello_deadline = None;
                    if hello.protocol_version != PROTOCOL_VERSION {
                        failure =
                            Some("The instance's guest supervisor doesn't match this app.".into());
                        force_stop(&vm);
                    } else if deadline.is_none()
                        && let Some(channel) = channel.as_mut()
                    {
                        let mut sent = channel.send(host_message::Kind::Start(start.clone()));
                        if let (Ok(()), Some(size)) = (&sent, size) {
                            sent = channel.send(host_message::Kind::Resize(size));
                        }
                        match sent {
                            Ok(()) => {
                                if agent_mode {
                                    last_status = Some(Instant::now());
                                }
                                report(Event::Ready);
                            }
                            Err(error) => {
                                eprintln!("Couldn't send Start to the guest supervisor: {error}")
                            }
                        }
                    }
                }
                Some(guest_message::Kind::ProvisionFinished(finished)) => {
                    report(Event::Provisioned(finished.exit_code));
                }
                // The supervisor powers off by itself; the boot ends with this reason.
                Some(guest_message::Kind::StartFailed(failed)) => {
                    failure = Some(failed.reason);
                }
                Some(guest_message::Kind::Status(status)) if deadline.is_none() => {
                    last_status = Some(Instant::now());
                    status_lost = false;
                    let cpu = baseline
                        .map(|before| {
                            let elapsed =
                                status.monotonic_usec.saturating_sub(before.monotonic_usec);
                            (before, elapsed)
                        })
                        .filter(|&(_, elapsed)| elapsed >= CPU_WINDOW.as_micros() as u64)
                        .map(|(before, elapsed)| {
                            let total = elapsed as f64 * spec.cpus.max(1) as f64;
                            let share = |now: u64, then: u64| {
                                (now.saturating_sub(then) as f64 / total).min(1.0) as f32
                            };
                            Cpu {
                                vm: share(status.vm_cpu_usec, before.vm_cpu_usec),
                                agent: status
                                    .agent_usage
                                    .zip(before.agent_usage)
                                    .map(|(now, then)| share(now.cpu_usec, then.cpu_usec)),
                            }
                        });
                    if cpu.is_some() || baseline.is_none() {
                        baseline = Some(status);
                    }
                    report(Event::Status(status, cpu));
                }
                // While the guest shuts down, the last reports no longer matter.
                Some(guest_message::Kind::Status(_)) => {}
                None => eprintln!("Ignoring a message from the guest supervisor."),
            },
            Ok(Command::Resize { columns, rows }) => {
                let resize = Resize { columns, rows };
                size = Some(resize);
                if let Some(channel) = channel.as_mut()
                    && let Err(error) = channel.send(host_message::Kind::Resize(resize))
                {
                    eprintln!("Couldn't send Resize to the guest supervisor: {error}");
                }
            }
            Ok(Command::Disconnected) => {
                channel = None;
                if deadline.is_none() {
                    next_connect = Some(Instant::now() + CONNECT_RETRY);
                }
            }
            // A repeated stop request, a completion without error, or the poll interval.
            Ok(_) | Err(_) => {}
        }
        let now = Instant::now();
        if !connecting
            && channel.is_none()
            && deadline.is_none()
            && next_connect.is_some_and(|at| now >= at)
        {
            next_connect = None;
            connecting = true;
            let connected = commands.clone();
            vm.connect(PORT, move |result| {
                let _ = connected.send(Command::Connected(result));
            });
        }
        if !status_lost && last_status.is_some_and(|at| now >= at + STATUS_TIMEOUT) {
            status_lost = true;
            report(Event::StatusLost);
        }
        if hello_deadline.is_some_and(|at| now >= at) {
            hello_deadline = None;
            failure = Some("The guest supervisor didn't answer.".into());
            force_stop(&vm);
        }
        if deadline.is_some_and(|deadline| now >= deadline) {
            force_stop(&vm);
        }
        match vm.state() {
            VmState::Stopped => return failure.map_or(Ok(()), Err),
            VmState::Error => return Err("The virtual machine stopped because of an error.".into()),
            state if state != last => {
                last = state;
                report(Event::State(state));
            }
            _ => {}
        }
    }
}

thread_local! {
    /// Where controller threads' events go; set by `VmControl::new`.
    static CONTROL: RefCell<Weak<VmControl>> = const { RefCell::new(Weak::new()) };
}

/// What happens once a VM has stopped.
#[derive(Clone, Copy, PartialEq)]
pub enum AfterStop {
    Nothing,
    Boot(BootMode),
    /// The instance is deleted; nothing cancels this.
    Delete,
}

/// A running VM, as the UI thread tracks it.
struct Running {
    commands: Sender<Command>,
    /// The mode of this boot; maintenance and provision boots are one-time.
    mode: BootMode,
    /// The VM's vCPUs.
    cpus: usize,
    /// Set while a stop is in progress, with what happens afterwards.
    stop: Option<AfterStop>,
    /// Set once the guest supervisor got Start; until then the boot is still starting.
    ready: bool,
    /// The provisioning script's exit code, once the supervisor reported it.
    provisioned: Option<i32>,
    /// Agent mode: the supervisor's latest Status and the latest CPU use computed.
    status: Option<Status>,
    cpu: Option<Cpu>,
    /// Set while Status reports are overdue.
    status_lost: bool,
}

/// Which of an instance's terminals is shown: the session (Terminal tab) or the boot log
/// (Logs tab).
#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Session,
    Log,
}

/// An instance's two terminals, kept after its VM stops.
struct Terminals {
    session: Rc<Terminal>,
    /// Read-only: kernel, systemd and supervisor output.
    log: Rc<Terminal>,
}

/// Starts and stops the instances' VMs and keeps their sidebar rows and terminals in sync.
/// Lives on the UI thread; each VM runs on its own controller thread.
pub struct VmControl {
    db: Rc<Db>,
    instances: Rc<VecModel<InstanceItem>>,
    ui: slint::Weak<AppWindow>,
    running: RefCell<HashMap<String, Running>>,
    /// Each instance's terminals.
    terminals: RefCell<HashMap<String, Terminals>>,
    /// The instance whose terminal is shown.
    shown: RefCell<Option<String>>,
    /// Which of its terminals is shown.
    view: Cell<View>,
    /// The terminal view's size as (columns, rows).
    size: Cell<(i32, i32)>,
    /// Set once the app is quitting; the event loop ends when all VMs have stopped.
    quitting: Cell<bool>,
}

impl VmControl {
    pub fn new(ui: &AppWindow, db: Rc<Db>, instances: Rc<VecModel<InstanceItem>>) -> Rc<VmControl> {
        let control = Rc::new(VmControl {
            db,
            instances,
            ui: ui.as_weak(),
            running: RefCell::default(),
            terminals: RefCell::default(),
            shown: RefCell::default(),
            view: Cell::new(View::Session),
            size: Cell::new((80, 24)),
            quitting: Cell::new(false),
        });
        CONTROL.set(Rc::downgrade(&control));
        control
    }

    /// Boots the instance's VM unless it already runs.
    pub fn start(&self, id: &str, mode: BootMode) {
        if self.running.borrow().contains_key(id) {
            return;
        }
        let (spec, start, mode, revision) = match self.prepare(id, mode) {
            Ok(prepared) => prepared,
            Err(error) => return self.failed(id, mode, &error.to_string()),
        };
        // Frozen right away, so Settings can't change the revision this boot just read.
        if let Err(error) = self.db.mark_booted(id, revision) {
            return self.failed(id, mode, &format!("couldn't record the boot: {error}"));
        }
        self.update_item(id, |item| {
            item.active_revision = revision as i32;
            item.saved_revision = revision as i32;
            item.pending_changes = 0;
        });
        let cpus = spec.cpus;
        let event_id = id.to_owned();
        let spawned = spawn(spec, start, move |event| {
            let id = event_id.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(control) = CONTROL.with_borrow(Weak::upgrade) {
                    control.handle(&id, event);
                }
            });
        });
        let commands = match spawned {
            Ok(commands) => commands,
            Err(error) => return self.failed(id, mode, &error.to_string()),
        };
        self.running.borrow_mut().insert(
            id.to_owned(),
            Running {
                commands,
                mode,
                cpus,
                stop: None,
                ready: false,
                provisioned: None,
                status: None,
                cpu: None,
                status_lost: false,
            },
        );
        self.show(id, mode, VmState::Starting);
    }

    /// Describes the VM and the Start message for its guest supervisor from the instance's
    /// latest revision, which this boot freezes. Also returns the boot mode and revision
    /// number. An instance that isn't provisioned yet boots in provision mode instead of
    /// agent mode. Agent and maintenance boots get the enabled shared folders and the
    /// environment; a missing folder or invalid setting fails the boot before the VM starts.
    fn prepare(&self, id: &str, mode: BootMode) -> db::Result<(VmSpec, Start, BootMode, i64)> {
        let instance = self.db.instance(id)?;
        let revision = self.db.latest_revision(id)?;
        let mode = if mode == BootMode::Agent && !instance.provisioned {
            BootMode::Provision
        } else {
            mode
        };
        let config = revision.config;
        // Only agent boots are held to the network rules; maintenance and provision boots
        // get open internet. The Mac and the local network stay unreachable in every mode
        // unless the instance allows them.
        let allow_local = config.network_allow_local;
        let network = match mode {
            BootMode::Agent => {
                Policy::new(config.network_default, config.network_rules, allow_local)
            }
            BootMode::Maintenance | BootMode::Provision => Policy::open(allow_local),
        };
        let mut spec = VmSpec {
            cpus: config.cpus as usize,
            memory_mib: config.memory_mib.into(),
            rootfs: instance.disk_path,
            shares: Vec::new(),
            network: Arc::new(network),
        };
        let mut start = match mode {
            BootMode::Agent => Start {
                mode: Mode::Agent.into(),
                launch: Some(Launch {
                    command: config.command,
                    arguments: config.arguments,
                    working_directory: config.working_directory,
                }),
                ..Start::default()
            },
            BootMode::Maintenance => Start {
                mode: Mode::Maintenance.into(),
                ..Start::default()
            },
            BootMode::Provision => Start {
                mode: Mode::Provision.into(),
                provision_script: config.provision_script,
                ..Start::default()
            },
        };
        // Provisioning runs with open internet, so it gets neither the shared folders nor
        // the agent's environment.
        if mode == BootMode::Provision {
            return Ok((spec, start, mode, revision.number));
        }

        // Checked again here: a folder may have gone since the configuration was saved.
        if env_errors(&config.environment)
            .iter()
            .any(|error| !error.is_empty())
        {
            return Err("some environment variables need a fix in the settings.".into());
        }
        let mounts: Vec<db::Mount> = config
            .mounts
            .into_iter()
            .filter(|mount| mount.enabled)
            .collect();
        for (mount, guest_error) in mounts.iter().zip(guest_path_errors(&mounts)) {
            let path = display_path(&mount.host_path);
            match folder_problem(&mount.host_path) {
                Some(FolderProblem::Missing) => {
                    return Err(format!("{path} no longer exists on your Mac.").into());
                }
                Some(FolderProblem::Unreadable) => {
                    return Err(format!("the app can't read {path}.").into());
                }
                None if !guest_error.is_empty() => {
                    return Err(format!("{}: {guest_error}", mount.guest_path).into());
                }
                None => {}
            }
        }

        start.environment = config
            .environment
            .into_iter()
            .filter(|var| !var.name.is_empty())
            .map(|var| EnvVar {
                name: var.name,
                value: var.value,
            })
            .collect();
        for (index, mount) in mounts.into_iter().enumerate() {
            let name = index.to_string();
            start.mounts.push(Mount {
                share: name.clone(),
                guest_path: mount.guest_path,
                read_only: !mount.writable,
            });
            spec.shares.push(SharedFolder {
                name,
                host_path: mount.host_path,
                read_only: !mount.writable,
            });
        }
        Ok((spec, start, mode, revision.number))
    }

    /// Asks the instance's guest to shut down, then does `then`. Its controller stops it hard
    /// if it doesn't shut down in time. A pending deletion stays pending.
    pub fn stop(&self, id: &str, then: AfterStop) {
        let mode = {
            let mut running = self.running.borrow_mut();
            let Some(run) = running.get_mut(id) else {
                return;
            };
            if run.stop.is_none() {
                let _ = run.commands.send(Command::Stop);
            }
            if run.stop != Some(AfterStop::Delete) {
                run.stop = Some(then);
            }
            run.mode
        };
        self.show(id, mode, VmState::Stopping);
    }

    /// Boots the instance in agent mode: after shutting it down if it runs, right away if not.
    pub fn restart(&self, id: &str) {
        if self.running.borrow().contains_key(id) {
            self.stop(id, AfterStop::Boot(BootMode::Agent));
        } else {
            self.start(id, BootMode::Agent);
        }
    }

    /// Shuts all VMs down gracefully, like pressing Stop, then ends the event loop.
    pub fn quit(&self) {
        if self.running.borrow().is_empty() {
            let _ = slint::quit_event_loop();
            return;
        }
        if self.quitting.replace(true) {
            return;
        }
        // Instances already stopping just don't boot again afterwards.
        let ids: Vec<String> = self.running.borrow().keys().cloned().collect();
        for id in ids {
            self.stop(&id, AfterStop::Nothing);
        }
    }

    /// Opens the delete confirmation for the instance, with what deleting it removes.
    pub fn ask_delete(&self, id: &str) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let instance = match self.db.instance(id) {
            Ok(instance) => instance,
            Err(error) => return eprintln!("Couldn't read the instance to delete: {error}"),
        };
        // Allocated, not logical size: the disk is sparse and shares blocks with its base.
        let disk_size = std::fs::metadata(&instance.disk_path).map_or(String::new(), |meta| {
            let bytes = meta.blocks() * 512;
            if bytes >= 1_000_000_000 {
                format!("{:.1} GB", bytes as f64 / 1e9)
            } else {
                format!("{} MB", bytes / 1_000_000)
            }
        });
        let folders: Vec<SharedString> = self
            .db
            .latest_revision(id)
            .map(|revision| {
                revision
                    .config
                    .mounts
                    .iter()
                    .map(|mount| display_path(&mount.host_path).into())
                    .collect()
            })
            .unwrap_or_default();
        let g = ui.global::<Deletion>();
        g.set_instance_id(id.into());
        g.set_name(instance.name.into());
        g.set_running(self.running.borrow().contains_key(id));
        g.set_disk_size(disk_size.into());
        g.set_revisions(self.db.revision_count(id).unwrap_or(0) as i32);
        g.set_folders(ModelRc::new(VecModel::from(folders)));
        g.set_busy(false);
        g.set_error("".into());
        g.set_open(true);
    }

    /// Deletes the instance, after shutting its VM down if it runs.
    pub fn delete(&self, id: &str) {
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Deletion>().set_busy(true);
        }
        if self.running.borrow().contains_key(id) {
            self.stop(id, AfterStop::Delete);
        } else {
            self.finish_delete(id);
        }
    }

    /// Deletes the stopped instance from the database, then its folder with the disk, and
    /// reports the outcome to the dialog. Returns false if nothing was deleted.
    fn finish_delete(&self, id: &str) -> bool {
        let report = |error: String| {
            if let Some(ui) = self.ui.upgrade() {
                let g = ui.global::<Deletion>();
                g.set_busy(false);
                g.set_error(error.as_str().into());
                g.set_open(!error.is_empty());
            }
        };
        if let Err(error) = self.db.delete_instance(id) {
            report(format!("Couldn't delete the instance: {error}"));
            return false;
        }

        if self.shown.borrow().as_deref() == Some(id) {
            if let Some(terminal) = self.shown_terminal() {
                terminal.hide();
            }
            *self.shown.borrow_mut() = None;
        }
        self.terminals.borrow_mut().remove(id);

        // The next instance takes the deleted one's place, or the previous one if it was last.
        if let Some(row) = (0..self.instances.row_count()).find(|&row| {
            self.instances
                .row_data(row)
                .is_some_and(|item| item.id == id)
        }) {
            self.instances.remove(row);
            if let Some(ui) = self.ui.upgrade() {
                let (row, count) = (row as i32, self.instances.row_count() as i32);
                let selected = ui.get_selected_index();
                if selected > row {
                    ui.set_selected_index(selected - 1);
                } else if selected == row {
                    ui.set_selected_index(row.min(count - 1));
                }
            }
        }

        let removed = db::instance_dir(id)
            .map_err(|error| error.to_string())
            .and_then(|dir| match std::fs::remove_dir_all(&dir) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => {
                    Err(format!("{} ({error})", display_path(&dir)))
                }
                _ => Ok(()),
            });
        report(match removed {
            Ok(()) => String::new(),
            Err(error) => {
                format!("The instance was deleted, but its folder couldn't be removed: {error}")
            }
        });
        true
    }

    fn handle(&self, id: &str, event: Event) {
        match event {
            Event::Created { session, log } => {
                if !self.running.borrow().contains_key(id) {
                    return;
                }
                let (session_terminal, log_terminal) = self.terminals(id);
                let attached = session_terminal
                    .attach(session)
                    .and_then(|()| log_terminal.attach(log));
                if let Err(error) = attached {
                    eprintln!("Stopping the virtual machine: couldn't open the console: {error}");
                    self.stop(id, AfterStop::Nothing);
                }
            }
            Event::State(state) => {
                let Some((mode, stopping)) = self
                    .running
                    .borrow()
                    .get(id)
                    .map(|run| (run.mode, run.stop.is_some()))
                else {
                    return;
                };
                // The guest keeps running while it shuts down.
                self.show(id, mode, if stopping { VmState::Stopping } else { state });
            }
            Event::Ready => {
                let Some(mode) = self.running.borrow_mut().get_mut(id).map(|run| {
                    run.ready = true;
                    run.mode
                }) else {
                    return;
                };
                self.show(id, mode, VmState::Running);
            }
            Event::Provisioned(code) => {
                if let Some(run) = self.running.borrow_mut().get_mut(id) {
                    run.provisioned = Some(code);
                }
            }
            Event::Status(..) | Event::StatusLost => {
                let Some((mode, stopping)) = self.running.borrow_mut().get_mut(id).map(|run| {
                    match event {
                        Event::Status(status, cpu) => {
                            run.status = Some(status);
                            run.cpu = cpu.or(run.cpu);
                            run.status_lost = false;
                        }
                        _ => run.status_lost = true,
                    }
                    (run.mode, run.stop.is_some())
                }) else {
                    return;
                };
                // Reports arrive only once the session runs.
                self.show(
                    id,
                    mode,
                    if stopping {
                        VmState::Stopping
                    } else {
                        VmState::Running
                    },
                );
            }
            Event::Ended(result) => {
                let Some(run) = self.running.borrow_mut().remove(id) else {
                    return;
                };
                let quitting = self.quitting.get();
                // The VM is down either way; a failed deletion shows the instance stopped.
                if run.stop == Some(AfterStop::Delete) && self.finish_delete(id) {
                    if quitting && self.running.borrow().is_empty() {
                        let _ = slint::quit_event_loop();
                    }
                    return;
                }
                match result {
                    Err(error) => self.failed(id, run.mode, &error),
                    // A provisioning run powers off by itself once its script is done.
                    Ok(())
                        if run.mode == BootMode::Provision && run.stop.is_none() && !quitting =>
                    {
                        self.finish_provisioning(id, run.provisioned)
                    }
                    Ok(()) => {
                        self.detach(id, "The virtual machine stopped.");
                        self.show(id, run.mode, VmState::Stopped);
                        if let Some(AfterStop::Boot(mode)) = run.stop
                            && !quitting
                        {
                            self.start(id, mode);
                        }
                    }
                }
                if quitting && self.running.borrow().is_empty() {
                    let _ = slint::quit_event_loop();
                }
            }
        }
    }

    /// Handles the end of a provisioning run with the exit code the supervisor reported.
    /// On success the instance counts as provisioned and boots in agent mode.
    fn finish_provisioning(&self, id: &str, exit_code: Option<i32>) {
        let failed = |error: &str| self.failed(id, BootMode::Provision, error);
        match exit_code {
            Some(0) => match self.db.mark_provisioned(id) {
                Ok(()) => {
                    self.update_item(id, |item| item.provisioned = true);
                    self.start(id, BootMode::Agent);
                }
                Err(error) => failed(&format!("couldn't record the provisioning: {error}")),
            },
            Some(code) => failed(&format!("the provisioning script exited with {code}.")),
            None => failed("the provisioning script didn't finish."),
        }
    }

    fn failed(&self, id: &str, mode: BootMode, error: &str) {
        self.detach(id, &format!("Couldn't run the virtual machine: {error}"));
        self.show(id, mode, VmState::Error);
    }

    /// Ends both of the instance's terminals with the same message, so it shows on either tab.
    fn detach(&self, id: &str, message: &str) {
        let (session, log) = self.terminals(id);
        session.detach(message);
        log.detach(message);
    }

    /// The instance's session and log terminals, created on first use.
    fn terminals(&self, id: &str) -> (Rc<Terminal>, Rc<Terminal>) {
        let mut terminals = self.terminals.borrow_mut();
        let terminals = terminals.entry(id.to_owned()).or_insert_with(|| Terminals {
            session: Rc::new(Terminal::new(self.ui.clone(), STOPPED_MESSAGE, false)),
            log: Rc::new(Terminal::new(self.ui.clone(), STOPPED_MESSAGE, true)),
        });
        (terminals.session.clone(), terminals.log.clone())
    }

    /// The terminal shown in the Terminal or Logs tab.
    pub fn shown_terminal(&self) -> Option<Rc<Terminal>> {
        let shown = self.shown.borrow();
        let terminals = self.terminals.borrow();
        let terminals = terminals.get(shown.as_deref()?)?;
        Some(match self.view.get() {
            View::Session => terminals.session.clone(),
            View::Log => terminals.log.clone(),
        })
    }

    /// Shows the instance's terminals, in place of the previous instance's.
    pub fn select(&self, id: &str) {
        if self.shown.borrow().as_deref() != Some(id) {
            self.switch(|control| *control.shown.borrow_mut() = Some(id.to_owned()));
        }
    }

    /// Shows the session or the boot log of the selected instance.
    pub fn set_view(&self, view: View) {
        if self.view.get() != view {
            self.switch(|control| control.view.set(view));
        }
    }

    /// Hides the shown terminal, applies `change`, then sizes and shows the new one.
    fn switch(&self, change: impl FnOnce(&VmControl)) {
        if let Some(previous) = self.shown_terminal() {
            previous.hide();
        }
        change(self);
        let (cols, rows) = self.size.get();
        self.resize(cols, rows);
        if let Some(terminal) = self.shown_terminal() {
            terminal.show();
        }
    }

    /// The terminal view changed size. Both of the shown instance's terminals adopt it, so
    /// the session keeps its size while the Logs tab is open; hidden instances' terminals
    /// adopt it when shown.
    pub fn resize(&self, cols: i32, rows: i32) {
        self.size.set((cols, rows));
        let Some(id) = self.shown.borrow().clone() else {
            return;
        };
        let (session, log) = self.terminals(&id);
        session.resize(cols, rows);
        log.resize(cols, rows);
    }

    /// Changes the instance's sidebar row.
    fn update_item(&self, id: &str, change: impl FnOnce(&mut InstanceItem)) {
        if let Some((row, mut item)) = (0..self.instances.row_count())
            .filter_map(|row| Some((row, self.instances.row_data(row)?)))
            .find(|(_, item)| item.id == id)
        {
            change(&mut item);
            self.instances.set_row_data(row, item);
        }
    }

    /// Updates the instance's sidebar summary and its Status tab: the readings and, in
    /// agent mode, the latest resource use.
    fn show(&self, id: &str, mode: BootMode, state: VmState) {
        let Some((row, mut item)) = (0..self.instances.row_count())
            .filter_map(|row| Some((row, self.instances.row_data(row)?)))
            .find(|(_, item)| item.id == id)
        else {
            return;
        };
        let reading = |value: &str, tone| StatusReading {
            value: value.into(),
            tone,
        };
        let (status, cpu, lost, cpus) = self
            .running
            .borrow()
            .get(id)
            .map_or((None, None, false, 0), |run| {
                (run.status, run.cpu, run.status_lost, run.cpus)
            });
        let exit = |exit: AgentExit| {
            if exit.signal != 0 {
                format!("Signal {}", exit.signal)
            } else {
                format!("Status {}", exit.code)
            }
        };
        // Maintenance and provision boots don't start the agent.
        let active = if mode == BootMode::Maintenance {
            InstanceState::Maintenance
        } else {
            InstanceState::Running
        };
        let agent_reading = || match (mode, status) {
            (BootMode::Maintenance, _) => reading("Not running in maintenance", Tone::Neutral),
            (BootMode::Provision, _) => reading("Not started", Tone::Neutral),
            _ if lost => reading("Unknown", Tone::Warn),
            (_, None) => reading("Starting", Tone::Neutral),
            (_, Some(status)) if status.agent_running => reading("Running", Tone::Ok),
            (_, Some(status)) => match status.agent_last_exit {
                Some(last) => reading(
                    &format!("Exited, {}", exit(last).to_lowercase()),
                    Tone::Neutral,
                ),
                None => reading("Not running", Tone::Neutral),
            },
        };
        let monitoring = match status {
            _ if mode != BootMode::Agent => reading("Not running", Tone::Neutral),
            _ if lost => reading("Unknown", Tone::Warn),
            None => reading("Not running", Tone::Neutral),
            Some(status) => match status.collector() {
                CollectorState::Running => reading("Running", Tone::Ok),
                CollectorState::Stalled => reading("Degraded", Tone::Warn),
                CollectorState::Stopped => reading("Not running", Tone::Warn),
                CollectorState::NotInstalled | CollectorState::Unspecified => {
                    reading("Not installed", Tone::Neutral)
                }
            },
        };
        let note = |text: &str| Usage {
            note: text.into(),
            ..Usage::default()
        };
        let usage = match (state, mode, status) {
            (VmState::Stopped | VmState::Error, ..) => {
                note("Start the instance to see its resource use.")
            }
            (VmState::Stopping, ..) => note("The virtual machine is stopping."),
            (_, BootMode::Maintenance, _) => note("Resource use is reported in agent mode only."),
            (_, BootMode::Provision, _) => {
                note("Resource use is reported once provisioning has finished.")
            }
            (_, BootMode::Agent, None) => note("Waiting for the first status report."),
            (_, BootMode::Agent, Some(status)) => {
                let size = |bytes: u64| {
                    let gib = bytes as f64 / f64::from(1 << 30);
                    if gib >= 1.0 {
                        format!("{gib:.1} GB")
                    } else {
                        format!("{} MB", bytes >> 20)
                    }
                };
                let percent = |share: f32| format!("{:.0}%", share * 100.0);
                let total = status.vm_memory_total_bytes.max(1);
                let fraction = |bytes: u64| (bytes as f64 / total as f64).min(1.0) as f32;
                let agent = status.agent_usage;
                let vcpus = format!("{cpus} vCPU{}", if cpus == 1 { "" } else { "s" });
                let vm_memory = size(status.vm_memory_used_bytes);
                Usage {
                    note: "".into(),
                    live: !lost,
                    measured: agent.is_some(),
                    agent_cpu: cpu.and_then(|cpu| cpu.agent).unwrap_or(0.0),
                    vm_cpu: cpu.map_or(0.0, |cpu| cpu.vm),
                    cpu_text: match cpu {
                        None => "Measuring…".into(),
                        Some(Cpu {
                            vm,
                            agent: Some(agent),
                        }) => format!("Agent {} · VM {} of {vcpus}", percent(agent), percent(vm)),
                        Some(Cpu { vm, agent: None }) => format!("VM {} of {vcpus}", percent(vm)),
                    }
                    .into(),
                    agent_memory: agent.map_or(0.0, |agent| fraction(agent.memory_bytes)),
                    vm_memory: fraction(status.vm_memory_used_bytes),
                    memory_text: match agent {
                        Some(agent) => format!(
                            "Agent {} · VM {vm_memory} of {}",
                            size(agent.memory_bytes),
                            size(total)
                        ),
                        None => format!("VM {vm_memory} of {}", size(total)),
                    }
                    .into(),
                    memory_warning: status.vm_memory_used_bytes >= total / 10 * 9
                        || agent.is_some_and(|agent| agent.oom_kills > 0),
                    restarts: status.agent_restarts as i32,
                    last_exit: status.agent_last_exit.map_or("None".into(), exit).into(),
                    oom_kills: agent.map_or(0, |agent| agent.oom_kills as i32),
                }
            }
        };
        let (instance_state, summary, vm, agent) = match state {
            VmState::Starting => (
                active,
                "Starting",
                reading("Starting", Tone::Neutral),
                reading("Not running", Tone::Neutral),
            ),
            VmState::Running | VmState::Other => (
                active,
                match mode {
                    BootMode::Agent => "Running",
                    BootMode::Maintenance => "Maintenance",
                    BootMode::Provision => "Provisioning",
                },
                reading("Running", Tone::Ok),
                agent_reading(),
            ),
            VmState::Stopping => (
                active,
                "Stopping",
                reading("Stopping", Tone::Neutral),
                agent_reading(),
            ),
            // Stopped, so Start stays available to try again.
            VmState::Error => (
                InstanceState::Stopped,
                "Startup failed",
                reading("Failed", Tone::Bad),
                reading("Not started", Tone::Neutral),
            ),
            VmState::Stopped => (
                InstanceState::Stopped,
                "Stopped",
                reading("Stopped", Tone::Neutral),
                reading("Not running", Tone::Neutral),
            ),
        };
        // A provisioning boot is covered throughout; its output shows dimmed behind it.
        let transition = match self.running.borrow().get(id) {
            Some(Running {
                stop: Some(then), ..
            }) => match then {
                AfterStop::Boot(BootMode::Maintenance) => "Restarting in maintenance…",
                AfterStop::Boot(_) => "Restarting in agent mode…",
                AfterStop::Delete => "Deleting…",
                AfterStop::Nothing => "Stopping…",
            },
            Some(run) if run.mode == BootMode::Provision => "Provisioning…",
            Some(run) if !run.ready => "Starting…",
            _ => "",
        };
        item.state = instance_state;
        item.summary = summary.into();
        item.transition = transition.into();
        item.vm = vm;
        item.agent_status = agent;
        item.monitoring = monitoring;
        item.usage = usage;
        self.instances.set_row_data(row, item);
    }
}
