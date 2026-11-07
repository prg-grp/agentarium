// Prevent console window in addition to Slint window in Windows release builds when, e.g., starting the app via file manager. Ignored on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod controller;
mod create;
mod db;
mod net;
mod policy;
mod preset;
mod terminal;
mod vm;

#[cfg(target_os = "macos")]
use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;

use slint::winit_030::WinitWindowAccessor;
use slint::{Model, ModelRc, Timer, VecModel};

use controller::{AfterStop, View, VmControl};
use db::Db;
use vm::BootMode;

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

    let control = VmControl::new(&ui, db.clone(), instances.clone());

    // Closing the window or quitting shuts all VMs down gracefully first.
    let c = control.clone();
    ui.window().on_close_requested(move || {
        c.quit();
        slint::CloseRequestResponse::KeepWindowShown
    });
    #[cfg(target_os = "macos")]
    {
        let c = control.clone();
        QUIT.set(Some(Box::new(move || c.quit())));
        // winit registers its app delegate class once the event loop runs.
        Timer::single_shot(Duration::ZERO, || {
            intercept_terminate();
            set_dock_icon();
        });
    }

    // New instances are drafted on the creation screen; once created, they get a sorted
    // sidebar row and, unless the user opted out, start and provision right away.
    let c = control.clone();
    let ui_handle = ui.as_weak();
    let creator = create::Creator::new(&ui, db, instances.clone(), move |instance, start| {
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
        c.select(&id);
        if start {
            c.start(&id, BootMode::Agent);
        }
    });
    ui.on_create_instance(move || creator.open());

    // The Terminal tab shows the selected instance's session, the Logs tab its boot log.
    let c = control.clone();
    ui.on_instance_selected(move |id| c.select(&id));
    let c = control.clone();
    ui.on_tab_changed(move |tab| c.set_view(if tab == 1 { View::Log } else { View::Session }));
    if let Some(item) = usize::try_from(ui.get_selected_index())
        .ok()
        .and_then(|row| ui.get_instances().row_data(row))
    {
        control.select(&item.id);
    }

    let c = control.clone();
    ui.on_instance_action(move |id, action| match action {
        InstanceAction::Start => c.start(&id, BootMode::Agent),
        InstanceAction::BootMaintenance => c.start(&id, BootMode::Maintenance),
        InstanceAction::Stop => c.stop(&id, AfterStop::Nothing),
        InstanceAction::RestartMaintenance => c.stop(&id, AfterStop::Boot(BootMode::Maintenance)),
        InstanceAction::RestartAgent => c.stop(&id, AfterStop::Boot(BootMode::Agent)),
        InstanceAction::Delete => c.ask_delete(&id),
    });
    let c = control.clone();
    let ui_handle = ui.as_weak();
    ui.global::<Deletion>().on_confirm(move || {
        if let Some(ui) = ui_handle.upgrade() {
            c.delete(&ui.global::<Deletion>().get_instance_id());
        }
    });

    let session = ui.global::<TerminalSession>();
    let c = control.clone();
    session.on_key(move |text, control, alt, meta, shift| {
        c.shown_terminal()
            .is_some_and(|t| t.key(&text, control, alt, meta, shift))
    });
    let c = control.clone();
    session.on_resized(move |cols, rows| c.resize(cols, rows));
    let c = control.clone();
    session.on_scrolled(move |lines| {
        if let Some(t) = c.shown_terminal() {
            t.scroll(lines);
        }
    });
    let c = control.clone();
    session.on_select_start(move |col, row| {
        if let Some(t) = c.shown_terminal() {
            t.select_start(col, row);
        }
    });
    let c = control.clone();
    session.on_select_update(move |col, row| {
        if let Some(t) = c.shown_terminal() {
            t.select_update(col, row);
        }
    });
    session.on_jump_to_latest(move || {
        if let Some(t) = control.shown_terminal() {
            t.jump_to_latest();
        }
    });

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
/// `QUIT` instead; the app then quits once the VMs have shut down.
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
        eprintln!("Couldn't find the app delegate; quitting won't stop the VMs gracefully.");
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
        eprintln!("Couldn't intercept quitting; quitting won't stop the VMs gracefully.");
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

/// A sidebar row for a stored instance; it shows as stopped until its VM runs.
fn to_item(instance: db::Instance) -> InstanceItem {
    let reading = |value: &str| StatusReading {
        value: value.into(),
        tone: Tone::Neutral,
    };
    InstanceItem {
        id: instance.id.into(),
        name: instance.name.into(),
        agent: instance.command.into(),
        state: InstanceState::Stopped,
        summary: "Stopped".into(),
        vm: reading("Stopped"),
        agent_status: reading("Not running"),
        monitoring: reading("Not running"),
        usage: Usage {
            note: "Start the instance to see its resource use.".into(),
            ..Usage::default()
        },
        // Counting saved but inactive changes comes with the settings screen.
        pending_changes: 0,
        has_terminal: true,
        transition: "".into(),
    }
}
