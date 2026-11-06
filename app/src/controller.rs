//! Runs the instances' VMs, any number at once.
//!
//! Each boot gets its own controller thread, which creates the VM, starts it, watches its
//! state and stops it; the VM itself operates on its own dispatch queue (see `vm.rs`).
//! Controllers report to `VmControl` on the UI thread, which owns the database and keeps
//! the sidebar rows and each instance's terminal in sync.
//!
//! The controller also holds the command plane to the guest supervisor: it connects once
//! the VM runs, answers the supervisor's Hello with Start (the boot mode), and sends Stop.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::rc::{Rc, Weak};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use command_plane::{
    EnvVar, GuestMessage, HostMessage, Launch, Mode, PORT, PROTOCOL_VERSION, Start, Stop,
    guest_message, host_message,
};
use slint::{ComponentHandle, Model, VecModel};

use crate::db::{self, Db};
use crate::terminal::{Console, Terminal};
use crate::vm::{BootMode, Vm, VmSpec, VmState};
use crate::{AppWindow, InstanceItem, InstanceState, StatusReading, Tone};

const STOPPED_MESSAGE: &str = "The virtual machine is stopped. Start it to open its console.";

/// How long the guest gets to shut down before it is stopped hard.
const STOP_TIMEOUT: Duration = Duration::from_secs(20);

/// How often a controller reads its VM's state, so a shutdown from inside the guest is noticed.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How long the guest supervisor gets from the VM's start to its first Hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(60);

/// How long to wait before connecting to the supervisor again while it doesn't listen yet.
const CONNECT_RETRY: Duration = Duration::from_millis(250);

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
}

/// What a controller thread reports to the UI thread.
enum Event {
    /// The VM is configured and starting; this is the host end of its console.
    Created(Console),
    State(VmState),
    /// The provisioning script finished with this exit code.
    Provisioned(i32),
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
    let (vm, console) = Vm::create(spec)?;
    report(Event::Created(console));
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
                        && let Err(error) = channel.send(host_message::Kind::Start(start.clone()))
                    {
                        eprintln!("Couldn't send Start to the guest supervisor: {error}");
                    }
                }
                Some(guest_message::Kind::ProvisionFinished(finished)) => {
                    report(Event::Provisioned(finished.exit_code));
                }
                None => eprintln!("Ignoring a message from the guest supervisor."),
            },
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

/// A running VM, as the UI thread tracks it.
struct Running {
    commands: Sender<Command>,
    /// The mode of this boot; maintenance and provision boots are one-time.
    mode: BootMode,
    /// The configuration revision this boot uses.
    revision: i64,
    /// Set while a stop is in progress; `Some(true)` boots maintenance mode afterwards.
    stop: Option<bool>,
    /// The provisioning script's exit code, once the supervisor reported it.
    provisioned: Option<i32>,
}

/// Starts and stops the instances' VMs and keeps their sidebar rows and terminals in sync.
/// Lives on the UI thread; each VM runs on its own controller thread.
pub struct VmControl {
    db: Rc<Db>,
    instances: Rc<VecModel<InstanceItem>>,
    ui: slint::Weak<AppWindow>,
    running: RefCell<HashMap<String, Running>>,
    /// Each instance's console, kept after its VM stops.
    terminals: RefCell<HashMap<String, Rc<Terminal>>>,
    /// The instance whose terminal is shown.
    shown: RefCell<Option<String>>,
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
                revision,
                stop: None,
                provisioned: None,
            },
        );
        self.show(id, mode, VmState::Starting);
    }

    /// Describes the VM and the Start message for its guest supervisor from the instance's
    /// latest revision, which this boot freezes. Also returns the boot mode and revision
    /// number. An instance that isn't provisioned yet boots in provision mode instead of
    /// agent mode.
    fn prepare(&self, id: &str, mode: BootMode) -> db::Result<(VmSpec, Start, BootMode, i64)> {
        let instance = self.db.instance(id)?;
        let revision = self.db.latest_revision(id)?;
        let mode = if mode == BootMode::Agent && !instance.provisioned {
            BootMode::Provision
        } else {
            mode
        };
        let config = revision.config;
        let start = match mode {
            BootMode::Agent => Start {
                mode: Mode::Agent.into(),
                launch: Some(Launch {
                    command: config.command,
                    arguments: config.arguments,
                    working_directory: config.working_directory,
                    environment: config
                        .environment
                        .into_iter()
                        .map(|var| EnvVar {
                            name: var.name,
                            value: var.value,
                        })
                        .collect(),
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
        let spec = VmSpec {
            cpus: config.cpus as usize,
            memory_mib: config.memory_mib.into(),
            rootfs: instance.disk_path,
        };
        Ok((spec, start, mode, revision.number))
    }

    /// Asks the instance's guest to shut down, then boots it in maintenance mode if
    /// `then_maintenance` is set. Its controller stops it hard if it doesn't shut down in time.
    pub fn stop(&self, id: &str, then_maintenance: bool) {
        let mode = {
            let mut running = self.running.borrow_mut();
            let Some(run) = running.get_mut(id) else {
                return;
            };
            if run.stop.is_none() {
                let _ = run.commands.send(Command::Stop);
            }
            run.stop = Some(then_maintenance);
            run.mode
        };
        self.show(id, mode, VmState::Stopping);
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
        // Instances already stopping just don't boot maintenance mode afterwards.
        let ids: Vec<String> = self.running.borrow().keys().cloned().collect();
        for id in ids {
            self.stop(&id, false);
        }
    }

    fn handle(&self, id: &str, event: Event) {
        match event {
            Event::Created(console) => {
                let Some(revision) = self.running.borrow().get(id).map(|run| run.revision) else {
                    return;
                };
                let attached = self
                    .db
                    .mark_booted(id, revision)
                    .map_err(|error| format!("couldn't record the boot: {error}"))
                    .and_then(|()| {
                        self.terminal(id)
                            .attach(console)
                            .map_err(|error| format!("couldn't open the console: {error}"))
                    });
                if let Err(error) = attached {
                    eprintln!("Stopping the virtual machine: {error}");
                    self.stop(id, false);
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
            Event::Provisioned(code) => {
                if let Some(run) = self.running.borrow_mut().get_mut(id) {
                    run.provisioned = Some(code);
                }
            }
            Event::Ended(result) => {
                let Some(run) = self.running.borrow_mut().remove(id) else {
                    return;
                };
                let quitting = self.quitting.get();
                match result {
                    Err(error) => self.failed(id, run.mode, &error),
                    // A provisioning run powers off by itself once its script is done.
                    Ok(())
                        if run.mode == BootMode::Provision && run.stop.is_none() && !quitting =>
                    {
                        self.finish_provisioning(id, run.provisioned)
                    }
                    Ok(()) => {
                        self.terminal(id).detach("The virtual machine stopped.");
                        self.show(id, run.mode, VmState::Stopped);
                        if run.stop == Some(true) && !quitting {
                            self.start(id, BootMode::Maintenance);
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
                Ok(()) => self.start(id, BootMode::Agent),
                Err(error) => failed(&format!("couldn't record the provisioning: {error}")),
            },
            Some(code) => failed(&format!("the provisioning script exited with {code}.")),
            None => failed("the provisioning script didn't finish."),
        }
    }

    fn failed(&self, id: &str, mode: BootMode, error: &str) {
        self.terminal(id)
            .detach(&format!("Couldn't run the virtual machine: {error}"));
        self.show(id, mode, VmState::Error);
    }

    /// The instance's terminal, created on first use.
    fn terminal(&self, id: &str) -> Rc<Terminal> {
        self.terminals
            .borrow_mut()
            .entry(id.to_owned())
            .or_insert_with(|| Rc::new(Terminal::new(self.ui.clone(), STOPPED_MESSAGE)))
            .clone()
    }

    /// The terminal shown in the Terminal tab.
    pub fn shown_terminal(&self) -> Option<Rc<Terminal>> {
        let shown = self.shown.borrow();
        self.terminals.borrow().get(shown.as_deref()?).cloned()
    }

    /// Shows the instance's terminal in the Terminal tab, in place of the previous one.
    pub fn select(&self, id: &str) {
        if self.shown.borrow().as_deref() == Some(id) {
            return;
        }
        if let Some(previous) = self.shown_terminal() {
            previous.hide();
        }
        *self.shown.borrow_mut() = Some(id.to_owned());
        let terminal = self.terminal(id);
        let (cols, rows) = self.size.get();
        terminal.resize(cols, rows);
        terminal.show();
    }

    /// The terminal view changed size; hidden terminals adopt it when shown.
    pub fn resize(&self, cols: i32, rows: i32) {
        self.size.set((cols, rows));
        if let Some(terminal) = self.shown_terminal() {
            terminal.resize(cols, rows);
        }
    }

    /// Updates the instance's sidebar summary and header readings.
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
        let agent_name = item.agent.clone();
        // Maintenance and provision boots don't start the agent.
        let active = if mode == BootMode::Maintenance {
            InstanceState::Maintenance
        } else {
            InstanceState::Running
        };
        let agent_reading = || {
            if mode == BootMode::Agent {
                reading(&agent_name, Tone::Ok)
            } else {
                reading("Not started", Tone::Neutral)
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
        item.state = instance_state;
        item.summary = summary.into();
        item.vm = vm;
        item.agent_status = agent;
        self.instances.set_row_data(row, item);
    }
}
