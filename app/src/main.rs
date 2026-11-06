// Prevent console window in addition to Slint window in Windows release builds when, e.g., starting the app via file manager. Ignored on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod create;
mod db;
mod net;
mod terminal;
mod vm;

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;

use slint::winit_030::WinitWindowAccessor;
use slint::{Model, ModelRc, Timer, TimerMode, VecModel};

use db::Db;
use terminal::{Console, Terminal};
use vm::{BootMode, Vm, VmSpec, VmState};

slint::include_modules!();

fn main() -> Result<(), Box<dyn Error>> {
    // Let the sidebar run under the window buttons, as in the design.
    #[cfg(target_os = "macos")]
    {
        use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
        slint::BackendSelector::new()
            .with_winit_window_attributes_hook(|attributes| {
                attributes
                    .with_titlebar_transparent(true)
                    .with_fullsize_content_view(true)
                    .with_title_hidden(true)
            })
            .select()?;
    }

    let ui = AppWindow::new()?;

    let db = Rc::new(Db::open().map_err(|error| format!("Couldn't open the database: {error}"))?);
    let items: Vec<InstanceItem> = db.instances()?.into_iter().map(to_item).collect();
    let instances = Rc::new(VecModel::from(items));
    ui.set_instances(ModelRc::from(instances.clone()));

    // The running instance's console is shown in the Terminal tab.
    // TODO(multi-vm): one terminal per running instance, shown for the selected one.
    let terminal = Rc::new(Terminal::start(&ui, STOPPED_MESSAGE));
    let control = Rc::new(VmControl {
        db: db.clone(),
        instances: instances.clone(),
        instance_id: RefCell::new(None),
        terminal: terminal.clone(),
        vm: RefCell::new(None),
        state: Cell::new(VmState::Stopped),
        mode: Cell::new(BootMode::Agent),
        poll: Timer::default(),
        stop_request: Cell::new(None),
        stop_fallback: Timer::default(),
        quitting: Cell::new(false),
    });

    // Closing the window or quitting shuts the VM down gracefully first.
    let c = control.clone();
    ui.window().on_close_requested(move || {
        VmControl::quit(&c);
        slint::CloseRequestResponse::KeepWindowShown
    });
    #[cfg(target_os = "macos")]
    {
        let c = control.clone();
        QUIT.set(Some(Box::new(move || VmControl::quit(&c))));
        // winit registers its app delegate class once the event loop runs.
        Timer::single_shot(Duration::ZERO, || {
            intercept_terminate();
            set_dock_icon();
        });
    }

    // New instances are drafted on the creation screen; once created, they get a sorted
    // sidebar row and, unless the user opted out, start and provision right away.
    let c = control.clone();
    let busy = control.clone();
    let ui_handle = ui.as_weak();
    let creator = create::Creator::new(
        &ui,
        db,
        instances.clone(),
        move || busy.vm.borrow().is_some(),
        move |instance, start| {
            let Some(ui) = ui_handle.upgrade() else {
                return;
            };
            let id = instance.id.clone();
            let name = instance.name.to_lowercase();
            let row = (0..instances.row_count())
                .find(|&row| {
                    instances
                        .row_data(row)
                        .is_some_and(|item| item.name.to_lowercase() > name)
                })
                .unwrap_or(instances.row_count());
            instances.insert(row, to_item(instance));
            ui.set_selected_index(row as i32);
            ui.set_current_tab(0);
            if start {
                VmControl::start(&c, &id, BootMode::Agent);
            }
        },
    );
    ui.on_create_instance(move || creator.open());

    ui.on_instance_action(move |id, action| {
        let current = control.instance_id.borrow().as_deref() == Some(id.as_str());
        match action {
            InstanceAction::Start => VmControl::start(&control, &id, BootMode::Agent),
            InstanceAction::BootMaintenance => {
                VmControl::start(&control, &id, BootMode::Maintenance)
            }
            InstanceAction::Stop if current => VmControl::stop(&control, false),
            InstanceAction::RestartMaintenance if current => VmControl::stop(&control, true),
            _ => {}
        }
    });

    let session = ui.global::<TerminalSession>();
    let t = terminal.clone();
    session.on_key(move |text, control, alt, meta, shift| t.key(&text, control, alt, meta, shift));
    let t = terminal.clone();
    session.on_resized(move |cols, rows| t.resize(cols, rows));
    let t = terminal.clone();
    session.on_scrolled(move |lines| t.scroll(lines));
    let t = terminal.clone();
    session.on_select_start(move |col, row| t.select_start(col, row));
    let t = terminal.clone();
    session.on_select_update(move |col, row| t.select_update(col, row));
    session.on_jump_to_latest(move || terminal.jump_to_latest());

    let ui_handle = ui.as_weak();
    ui.on_drag_window(move || {
        if let Some(ui) = ui_handle.upgrade() {
            ui.window().with_winit_window(|window| {
                let _ = window.drag_window();
            });
        }
    });

    ui.run()?;

    Ok(())
}

#[cfg(target_os = "macos")]
thread_local! {
    /// Runs when the app is asked to quit with Cmd+Q or from the Dock.
    static QUIT: RefCell<Option<Box<dyn Fn()>>> = RefCell::new(None);
}

/// Cmd+Q and the Dock's Quit send `terminate:`, which ends the process right away
/// because winit's app delegate doesn't answer `applicationShouldTerminate:`.
/// This adds that method to the delegate class, so it cancels termination and runs
/// `QUIT` instead; the app then quits once the VM has shut down.
#[cfg(target_os = "macos")]
fn intercept_terminate() {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};

    unsafe extern "C-unwind" fn should_terminate(
        _: *mut AnyObject,
        _: Sel,
        _: *mut AnyObject,
    ) -> usize {
        // Run outside AppKit's callback, like other UI events.
        Timer::single_shot(Duration::ZERO, || {
            QUIT.with_borrow(|quit| {
                if let Some(quit) = quit {
                    quit();
                }
            })
        });
        0 // NSTerminateCancel
    }

    let Some(class) = AnyClass::get(c"WinitApplicationDelegate") else {
        eprintln!("Couldn't find the app delegate; quitting won't stop the VM gracefully.");
        return;
    };
    let added = unsafe {
        let imp = std::mem::transmute::<
            unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize,
            Imp,
        >(should_terminate);
        objc2::ffi::class_addMethod(
            class as *const AnyClass as *mut AnyClass,
            objc2::sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        )
    };
    if !added.as_bool() {
        eprintln!("Couldn't intercept quitting; quitting won't stop the VM gracefully.");
    }
}

/// winit ignores the window icon on macOS, so the Dock would show a generic icon
/// for the unbundled binary. This sets the app icon on `NSApplication` directly.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSData;

    let (Some(image_class), Some(app_class)) =
        (AnyClass::get(c"NSImage"), AnyClass::get(c"NSApplication"))
    else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../ui/app-icon.png"));
    unsafe {
        let image: Allocated<AnyObject> = objc2::msg_send![image_class, alloc];
        let image: Option<Retained<AnyObject>> = objc2::msg_send![image, initWithData: &*data];
        let app: *mut AnyObject = objc2::msg_send![app_class, sharedApplication];
        if let Some(image) = image {
            let _: () = objc2::msg_send![app, setApplicationIconImage: &*image];
        }
    }
}

const STOPPED_MESSAGE: &str = "The virtual machine is stopped. Start it to open its console.";

/// How long the guest gets to shut down before it is stopped hard.
const STOP_TIMEOUT: Duration = Duration::from_secs(20);

/// Runs one instance's VM at a time and keeps its instance row and terminal in sync.
/// Lives on the main thread, like the VM itself.
// TODO(multi-vm): several VMs should run at once. Then this holds one VM, state, mode and
// terminal per instance id instead of the single `instance_id`, `vm` and `terminal`.
struct VmControl {
    db: Rc<Db>,
    instances: Rc<VecModel<InstanceItem>>,
    /// The instance whose VM runs, or ran last.
    instance_id: RefCell<Option<String>>,
    terminal: Rc<Terminal>,
    vm: RefCell<Option<Vm>>,
    state: Cell<VmState>,
    /// The mode of the current boot; maintenance and provision boots are one-time.
    mode: Cell<BootMode>,
    /// Checks the VM state, so a shutdown from inside the guest is noticed.
    poll: Timer,
    /// Set while a stop is in progress; `Some(true)` boots maintenance mode afterwards.
    stop_request: Cell<Option<bool>>,
    /// Stops the VM hard if the guest doesn't shut down within `STOP_TIMEOUT`.
    stop_fallback: Timer,
    /// Set once the app is quitting; the event loop ends when the VM has stopped.
    quitting: Cell<bool>,
}

impl VmControl {
    fn start(this: &Rc<VmControl>, id: &str, mode: BootMode) {
        // TODO(multi-vm): only refuse when this instance's own VM already runs.
        if this.vm.borrow().is_some() {
            return;
        }
        *this.instance_id.borrow_mut() = Some(id.to_owned());
        let (vm, console) = match this.create_vm(id, mode) {
            Ok(created) => created,
            Err(error) => return this.failed(&error.to_string()),
        };
        if let Err(error) = this.terminal.attach(console) {
            return this.failed(&error.to_string());
        }
        this.show(VmState::Starting);
        *this.vm.borrow_mut() = Some(vm);

        let control = Rc::downgrade(this);
        if let Some(vm) = this.vm.borrow().as_ref() {
            vm.start(move |result| {
                if let Some(control) = control.upgrade() {
                    match result {
                        Ok(()) => VmControl::check(&control),
                        Err(error) => control.failed(&error),
                    }
                }
            });
        }

        let control = Rc::downgrade(this);
        this.poll
            .start(TimerMode::Repeated, Duration::from_secs(1), move || {
                if let Some(control) = control.upgrade() {
                    VmControl::check(&control);
                }
            });
    }

    /// Configures the VM from the instance's latest revision, which this boot freezes.
    /// An instance that isn't provisioned yet boots in provision mode instead of agent mode.
    fn create_vm(&self, id: &str, mode: BootMode) -> db::Result<(Vm, Console)> {
        let instance = self.db.instance(id)?;
        let revision = self.db.latest_revision(id)?;
        let mode = if mode == BootMode::Agent && !instance.provisioned {
            BootMode::Provision
        } else {
            mode
        };
        self.mode.set(mode);
        // The guest runs provision.sh from the shared folder and leaves its exit code in status.
        let share = if mode == BootMode::Provision {
            let dir = db::instance_dir(id)?.join("provision");
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("provision.sh"), &revision.config.provision_script)?;
            match std::fs::remove_file(dir.join("status")) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into());
                }
                _ => {}
            }
            Some(dir)
        } else {
            None
        };
        let created = Vm::create(&VmSpec {
            cpus: revision.config.cpus as usize,
            memory_mib: revision.config.memory_mib.into(),
            rootfs: instance.disk_path,
            mode,
            share,
        })?;
        self.db.mark_booted(id, revision.number)?;
        Ok(created)
    }

    /// Asks the guest to shut down, then boots it in maintenance mode if
    /// `then_maintenance` is set. Stops it hard if it doesn't shut down in time.
    fn stop(this: &Rc<VmControl>, then_maintenance: bool) {
        let requested = match this.vm.borrow().as_ref() {
            Some(vm) => vm.request_stop(),
            None => return,
        };
        this.stop_request.set(Some(then_maintenance));
        this.show(VmState::Stopping);
        match requested {
            Ok(()) => {
                let control = Rc::downgrade(this);
                this.stop_fallback
                    .start(TimerMode::SingleShot, STOP_TIMEOUT, move || {
                        if let Some(control) = control.upgrade() {
                            VmControl::force_stop(&control);
                        }
                    });
            }
            Err(error) => {
                eprintln!("Couldn't ask the virtual machine to stop: {error}");
                VmControl::force_stop(this);
            }
        }
    }

    /// Shuts the VM down gracefully, like pressing Stop, then ends the event loop.
    fn quit(this: &Rc<VmControl>) {
        if this.vm.borrow().is_none() {
            let _ = slint::quit_event_loop();
            return;
        }
        if this.quitting.replace(true) {
            return;
        }
        if this.stop_request.get().is_some() {
            // Already stopping; just don't boot maintenance mode afterwards.
            this.stop_request.set(Some(false));
        } else {
            VmControl::stop(this, false);
        }
    }

    /// Stops the VM immediately, like pulling the power cord.
    fn force_stop(this: &Rc<VmControl>) {
        let control = Rc::downgrade(this);
        if let Some(vm) = this.vm.borrow().as_ref() {
            vm.stop(move |result| {
                let Some(control) = control.upgrade() else {
                    return;
                };
                if let Err(error) = result {
                    eprintln!("Couldn't stop the virtual machine: {error}");
                    // Quitting takes the VM down with the app anyway.
                    if control.quitting.get() {
                        let _ = slint::quit_event_loop();
                        return;
                    }
                }
                VmControl::check(&control);
            });
        }
    }

    /// Reads the VM state and cleans up once it has stopped.
    fn check(this: &Rc<VmControl>) {
        let state = match this.vm.borrow().as_ref() {
            Some(vm) => vm.state(),
            None => return,
        };
        match state {
            VmState::Stopped => {
                let requested = this.stop_request.get();
                this.end();
                let id = this.instance_id.borrow().clone();
                // A provisioning run powers off by itself once its script is done.
                if this.mode.get() == BootMode::Provision
                    && requested.is_none()
                    && !this.quitting.get()
                    && let Some(id) = id.as_deref()
                {
                    return VmControl::finish_provisioning(this, id);
                }
                this.terminal.detach("The virtual machine stopped.");
                this.show(VmState::Stopped);
                if this.quitting.get() {
                    let _ = slint::quit_event_loop();
                } else if requested == Some(true)
                    && let Some(id) = id
                {
                    VmControl::start(this, &id, BootMode::Maintenance);
                }
            }
            VmState::Error => this.failed("The virtual machine stopped because of an error."),
            // The guest keeps running while it shuts down.
            _ if this.stop_request.get().is_some() => this.show(VmState::Stopping),
            _ => this.show(state),
        }
    }

    /// Reads the exit code the provisioning script left in the shared folder. On success
    /// the instance counts as provisioned and boots in agent mode.
    fn finish_provisioning(this: &Rc<VmControl>, id: &str) {
        let status = db::instance_dir(id)
            .ok()
            .and_then(|dir| std::fs::read_to_string(dir.join("provision").join("status")).ok());
        match status.as_deref().map(str::trim) {
            Some("0") => match this.db.mark_provisioned(id) {
                Ok(()) => VmControl::start(this, id, BootMode::Agent),
                Err(error) => this.failed(&format!("couldn't record the provisioning: {error}")),
            },
            Some(code) => this.failed(&format!("the provisioning script exited with {code}.")),
            None => this.failed("the provisioning script didn't finish."),
        }
    }

    fn failed(&self, error: &str) {
        self.end();
        self.terminal
            .detach(&format!("Couldn't run the virtual machine: {error}"));
        self.show(VmState::Error);
        if self.quitting.get() {
            let _ = slint::quit_event_loop();
        }
    }

    fn end(&self) {
        self.poll.stop();
        self.stop_fallback.stop();
        self.stop_request.set(None);
        self.vm.borrow_mut().take();
    }

    /// Updates the instance's sidebar summary and header readings.
    fn show(&self, state: VmState) {
        if self.state.replace(state) == state && state != VmState::Error {
            return;
        }
        let Some(id) = self.instance_id.borrow().clone() else {
            return;
        };
        let Some((row, mut item)) = (0..self.instances.row_count())
            .filter_map(|row| Some((row, self.instances.row_data(row)?)))
            .find(|(_, item)| item.id == id.as_str())
        else {
            return;
        };
        let reading = |value: &str, tone| StatusReading {
            value: value.into(),
            tone,
        };
        let agent_name = item.agent.clone();
        // Maintenance and provision boots don't start the agent.
        let mode = self.mode.get();
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

/// A sidebar row for a stored instance; it shows as stopped until its VM runs.
fn to_item(instance: db::Instance) -> InstanceItem {
    let reading = |value: &str| StatusReading {
        value: value.into(),
        tone: Tone::Neutral,
    };
    InstanceItem {
        id: instance.id.into(),
        name: instance.name.into(),
        agent: instance.agent.name().into(),
        state: InstanceState::Stopped,
        summary: "Stopped".into(),
        vm: reading("Stopped"),
        agent_status: reading("Not running"),
        monitoring: reading("Not running"),
        // Counting saved but inactive changes comes with the settings screen.
        pending_changes: 0,
        has_terminal: true,
    }
}
