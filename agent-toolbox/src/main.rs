// Prevent console window in addition to Slint window in Windows release builds when, e.g., starting the app via file manager. Ignored on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod data;
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
    });
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

const STOPPED_MESSAGE: &str = "The virtual machine is stopped. Start it to open its console.";

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
                        Ok(()) => control.check(),
                        Err(error) => control.failed(&error),
                    }
                }
            });
        }

        let control = Rc::downgrade(this);
        this.poll
            .start(TimerMode::Repeated, Duration::from_secs(1), move || {
                if let Some(control) = control.upgrade() {
                    control.check();
                }
            });
    }

    /// Stops the VM, then boots it in maintenance mode if `then_maintenance` is set.
    fn stop(this: &Rc<VmControl>, then_maintenance: bool) {
        let control = Rc::downgrade(this);
        if let Some(vm) = this.vm.borrow().as_ref() {
            this.show(VmState::Stopping);
            vm.stop(move |result| {
                if let Err(error) = result {
                    eprintln!("Couldn't stop the virtual machine: {error}");
                }
                if let Some(control) = control.upgrade() {
                    control.check();
                    if then_maintenance && control.vm.borrow().is_none() {
                        VmControl::start(&control, true);
                    }
                }
            });
        }
    }

    /// Reads the VM state and cleans up once it has stopped.
    fn check(&self) {
        let state = match self.vm.borrow().as_ref() {
            Some(vm) => vm.state(),
            None => return,
        };
        match state {
            VmState::Stopped => {
                self.end();
                self.terminal.detach("The virtual machine stopped.");
                self.show(VmState::Stopped);
            }
            VmState::Error => self.failed("The virtual machine stopped because of an error."),
            _ => self.show(state),
        }
    }

    fn failed(&self, error: &str) {
        self.end();
        self.terminal
            .detach(&format!("Couldn't run the virtual machine: {error}"));
        self.show(VmState::Error);
    }

    fn end(&self) {
        self.poll.stop();
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
