// Prevent console window in addition to Slint window in Windows release builds when, e.g., starting the app via file manager. Ignored on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod data;

use std::error::Error;

use slint::winit_030::WinitWindowAccessor;
use slint::{ModelRc, VecModel};

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
    ui.set_instances(ModelRc::new(VecModel::from(items)));

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
    }
}
