// Prevent console window in addition to Slint window in Windows release builds when, e.g., starting the app via file manager. Ignored on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod data;
mod net;
mod terminal;
mod vm;

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;

use slint::winit_030::WinitWindowAccessor;
use slint::{Model, ModelRc, Timer, TimerMode, VecModel};

use terminal::Terminal;
use vm::{Vm, VmState};

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

    let items: Vec<InstanceItem> = data::instances().into_iter().map(to_item).collect();
    let instances = Rc::new(VecModel::from(items));
    ui.set_instances(ModelRc::from(instances.clone()));

    // The demo VM's console is shown in the Terminal tab.
    let terminal = Rc::new(Terminal::start(&ui, STOPPED_MESSAGE));
    let control = Rc::new(VmControl {
        instances,
        terminal: terminal.clone(),
        vm: RefCell::new(None),
        state: Cell::new(VmState::Stopped),
        maintenance: Cell::new(false),
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

    ui.on_instance_action(move |id, action| {
        if id != data::DEMO_VM_ID {
            return;
        }
        match action {
            InstanceAction::Start => VmControl::start(&control, false),
            InstanceAction::BootMaintenance => VmControl::start(&control, true),
            InstanceAction::Stop => VmControl::stop(&control, false),
            InstanceAction::RestartMaintenance => VmControl::stop(&control, true),
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

/// Runs the demo VM and keeps its instance row and terminal in sync.
/// Lives on the main thread, like the VM itself.
struct VmControl {
    instances: Rc<VecModel<InstanceItem>>,
    terminal: Rc<Terminal>,
    vm: RefCell<Option<Vm>>,
    state: Cell<VmState>,
    /// Whether the current boot is a one-time maintenance boot.
    maintenance: Cell<bool>,
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
    fn start(this: &Rc<VmControl>, maintenance: bool) {
        if this.vm.borrow().is_some() {
            return;
        }
        this.maintenance.set(maintenance);
        let (vm, console) = match Vm::create(&data::demo_vm(maintenance)) {
            Ok(created) => created,
            Err(error) => return this.failed(&error),
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
                let then_maintenance = this.stop_request.get() == Some(true);
                this.end();
                this.terminal.detach("The virtual machine stopped.");
                this.show(VmState::Stopped);
                if this.quitting.get() {
                    let _ = slint::quit_event_loop();
                } else if then_maintenance {
                    VmControl::start(this, true);
                }
            }
            VmState::Error => this.failed("The virtual machine stopped because of an error."),
            // The guest keeps running while it shuts down.
            _ if this.stop_request.get().is_some() => this.show(VmState::Stopping),
            _ => this.show(state),
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

    /// Updates the demo VM's sidebar summary and header readings.
    fn show(&self, state: VmState) {
        if self.state.replace(state) == state && state != VmState::Error {
            return;
        }
        let Some((row, mut item)) = (0..self.instances.row_count())
            .filter_map(|row| Some((row, self.instances.row_data(row)?)))
            .find(|(_, item)| item.id == data::DEMO_VM_ID)
        else {
            return;
        };
        let reading = |value: &str, tone| StatusReading {
            value: value.into(),
            tone,
        };
        // A maintenance boot runs a root shell instead of the agent.
        let maintenance = self.maintenance.get();
        let active = if maintenance {
            InstanceState::Maintenance
        } else {
            InstanceState::Running
        };
        let agent_reading = || {
            if maintenance {
                reading("Not started", Tone::Neutral)
            } else {
                reading("Claude Code", Tone::Ok)
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
                if maintenance {
                    "Maintenance"
                } else {
                    "Running"
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

fn to_item(instance: data::Instance) -> InstanceItem {
    let reading = |reading: data::Reading| StatusReading {
        value: reading.value.into(),
        tone: match reading.tone {
            data::Tone::Ok => Tone::Ok,
            data::Tone::Bad => Tone::Bad,
            data::Tone::Neutral => Tone::Neutral,
        },
    };

    InstanceItem {
        id: instance.id.into(),
        name: instance.name.into(),
        agent: instance.agent.into(),
        state: match instance.state {
            data::InstanceState::Running => InstanceState::Running,
            data::InstanceState::Maintenance => InstanceState::Maintenance,
            data::InstanceState::StartupFailed => InstanceState::StartupFailed,
            data::InstanceState::Stopped => InstanceState::Stopped,
        },
        summary: instance.summary.into(),
        vm: reading(instance.vm),
        agent_status: reading(instance.agent_status),
        monitoring: reading(instance.monitoring),
        pending_changes: instance.pending_changes,
        has_terminal: instance.has_terminal,
    }
}
