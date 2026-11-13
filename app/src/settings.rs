//! The Settings tab: the shown instance's configuration. See design/settings-pending.md.
//!
//! Edits stay here until the user saves: Save writes them into the pending revision (a new
//! one if the latest was booted) and renames the instance; while the instance runs, it also
//! applies them to the running VM where it can (`VmControl::apply`), restarting the agent or
//! rebooting when the edits need that (`SaveKind`). The footer then says what happened. The
//! UI's `Settings` global shows the edits; every edit goes through a callback.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::controller::VmControl;
use crate::db::{self, Config, Db, NetworkAction};
use crate::editor::{
    self, FolderProblem, Host, Lists, command_error, display_path, env_errors, folder_problem,
    guest_path_errors, pick_folder, remove, working_directory_error,
};
use crate::vm::BootMode;
use crate::{AppWindow, InstanceItem, SaveKind, Settings};

/// The shown instance: what is saved, and the edits.
struct Shown {
    id: String,
    saved_name: String,
    /// The latest revision's configuration.
    saved: Config,
    /// The disk's current size in GiB, at least `DISK_MIN_GB`: disks only grow.
    min_disk_gb: u32,
    name: String,
    config: Config,
    /// Why the last save failed; cleared by the next edit.
    save_error: String,
    /// What the last save did; cleared by the next edit.
    notice: &'static str,
}

pub struct SettingsEditor {
    ui: slint::Weak<AppWindow>,
    db: Rc<Db>,
    instances: Rc<VecModel<InstanceItem>>,
    control: Rc<VmControl>,
    shown: RefCell<Option<Shown>>,
    lists: Lists,
}

impl SettingsEditor {
    pub fn new(
        ui: &AppWindow,
        db: Rc<Db>,
        instances: Rc<VecModel<InstanceItem>>,
        control: Rc<VmControl>,
    ) -> Rc<SettingsEditor> {
        let this = Rc::new(SettingsEditor {
            ui: ui.as_weak(),
            db,
            instances,
            control,
            shown: RefCell::new(None),
            lists: Lists::default(),
        });

        let g = ui.global::<Settings>();
        let lists = &this.lists;
        g.set_max_folders(command_plane::SHARE_SLOTS as i32);
        g.set_arguments(ModelRc::from(lists.arguments.clone()));
        g.set_environment(ModelRc::from(lists.environment.clone()));
        g.set_mounts(ModelRc::from(lists.mounts.clone()));
        g.set_layout(ModelRc::from(lists.layout.clone()));
        g.set_allowed(ModelRc::from(lists.allowed.clone()));
        g.set_denied(ModelRc::from(lists.denied.clone()));
        g.set_forwards(ModelRc::from(lists.forwards.clone()));

        let s = this.clone();
        g.on_set_name(move |name| s.edit(|e| e.name = name.into()));
        let s = this.clone();
        g.on_set_command(move |command| s.edit(|e| e.config.command = command.into()));
        let s = this.clone();
        g.on_set_script(move |script| s.edit(|e| e.config.provision_script = script.into()));
        let s = this.clone();
        g.on_add_argument(move |argument| {
            s.edit(|e| editor::add_argument(&mut e.config, &argument))
        });
        let s = this.clone();
        g.on_remove_argument(move |index| s.edit(|e| remove(&mut e.config.arguments, index)));
        let s = this.clone();
        g.on_edit_argument(move |index, argument| {
            s.edit(|e| editor::edit_argument(&mut e.config, index, &argument))
        });
        let s = this.clone();
        g.on_move_argument(move |from, to| {
            s.edit(|e| editor::move_argument(&mut e.config, from, to))
        });
        let s = this.clone();
        g.on_set_working_directory(move |dir| s.edit(|e| e.config.working_directory = dir.into()));

        let s = this.clone();
        g.on_add_env(move || s.edit(|e| editor::add_env(&mut e.config)));
        let s = this.clone();
        g.on_edit_env(move |index, name, value| {
            s.edit(|e| editor::edit_env(&mut e.config, index, &name, &value))
        });
        let s = this.clone();
        g.on_remove_env(move |index| s.edit(|e| remove(&mut e.config.environment, index)));

        let s = this.clone();
        g.on_add_folder(move || {
            if let Some(folder) = pick_folder() {
                s.edit(|e| editor::add_folder(&mut e.config, folder));
            }
        });
        let s = this.clone();
        g.on_choose_folder(move |index| {
            if let Some(folder) = pick_folder() {
                s.edit(|e| editor::choose_folder(&mut e.config, index, folder));
            }
        });
        let s = this.clone();
        g.on_edit_guest_path(move |index, path| {
            s.edit(|e| editor::edit_guest_path(&mut e.config, index, &path))
        });
        let s = this.clone();
        g.on_set_writable(move |index, writable| {
            s.edit(|e| editor::set_writable(&mut e.config, index, writable))
        });
        let s = this.clone();
        g.on_remove_folder(move |index| s.edit(|e| remove(&mut e.config.mounts, index)));

        let s = this.clone();
        g.on_set_network_deny(move |deny| {
            s.edit(|e| e.config.network_default = editor::action(deny))
        });
        let s = this.clone();
        g.on_set_network_allow_local(move |allow| s.edit(|e| e.config.network_allow_local = allow));
        let s = this.clone();
        g.on_add_rule(move |text, deny| {
            let mut error = "";
            s.edit(|e| error = editor::add_rule(&mut e.config, &text, deny));
            error.into()
        });
        let s = this.clone();
        g.on_toggle_scope(move |deny, index| {
            s.edit(|e| editor::toggle_scope(&mut e.config, deny, index))
        });
        let s = this.clone();
        g.on_remove_rule(move |deny, index| {
            s.edit(|e| editor::remove_rule(&mut e.config, deny, index))
        });
        let s = this.clone();
        g.on_add_forward(move |guest, host| {
            let mut error = "";
            s.edit(|e| error = editor::add_forward(&mut e.config, &guest, &host));
            error.into()
        });
        let s = this.clone();
        g.on_toggle_forward(move |index| {
            s.edit(|e| editor::toggle_forward_scope(&mut e.config, index))
        });
        let s = this.clone();
        g.on_remove_forward(move |index| s.edit(|e| remove(&mut e.config.port_forwards, index)));

        let s = this.clone();
        g.on_set_cpus(move |cpus| s.edit(|e| e.config.cpus = cpus.max(1) as u32));
        let s = this.clone();
        g.on_set_memory(move |gb| s.edit(|e| e.config.memory_mib = gb.max(1) as u32 * 1024));
        let s = this.clone();
        g.on_set_disk(move |gb| {
            s.edit(|e| e.config.disk_gb = (gb.max(0) as u32).max(e.min_disk_gb))
        });

        let s = this.clone();
        g.on_discard(move || {
            s.edit(|e| {
                e.name = e.saved_name.clone();
                e.config = e.saved.clone();
            });
            s.show_fields();
        });
        let s = this.clone();
        g.on_save(move || s.save());

        this
    }

    /// Shows the instance's saved configuration; edits of the previously shown one are dropped.
    pub fn open(&self, id: &str) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let loaded = if id.is_empty() {
            None
        } else {
            let load = || -> db::Result<Shown> {
                let instance = self.db.instance(id)?;
                let saved = self.db.latest_revision(id)?.config;
                Ok(Shown {
                    id: id.into(),
                    saved_name: instance.name.clone(),
                    min_disk_gb: (std::fs::metadata(&instance.disk_path)?
                        .len()
                        .div_ceil(1 << 30) as u32)
                        .max(db::DISK_MIN_GB),
                    name: instance.name,
                    config: saved.clone(),
                    saved,
                    save_error: String::new(),
                    notice: "",
                })
            };
            load()
                .inspect_err(|error| eprintln!("Couldn't read the instance's settings: {error}"))
                .ok()
        };
        *self.shown.borrow_mut() = loaded;

        let host = Host::read();
        let g = ui.global::<Settings>();
        g.set_max_cpus(host.cpus as i32);
        g.set_max_memory_gb(host.max_memory_gb() as i32);
        g.set_host_line(host.line().into());
        self.show_fields();
        self.refresh();
    }

    fn edit(&self, change: impl FnOnce(&mut Shown)) {
        if let Some(shown) = self.shown.borrow_mut().as_mut() {
            change(shown);
            shown.save_error.clear();
            shown.notice = "";
        }
        self.refresh();
    }

    /// Puts the single values into their fields, which then keep them while the user types.
    fn show_fields(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let shown = self.shown.borrow();
        let Some(e) = shown.as_ref() else {
            return;
        };
        let g = ui.global::<Settings>();
        let c = &e.config;
        g.set_name(e.name.as_str().into());
        g.set_command(c.command.as_str().into());
        g.set_script(c.provision_script.as_str().into());
        g.set_working_directory(c.working_directory.as_str().into());
        g.set_cpus(c.cpus as i32);
        g.set_memory_gb((c.memory_mib / 1024) as i32);
        g.set_min_disk_gb(e.min_disk_gb as i32);
        g.set_disk_gb(c.disk_gb as i32);
    }

    /// Shows the lists, the checks, and whether and how the edits can be saved.
    fn refresh(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let shown = self.shown.borrow();
        let Some(e) = shown.as_ref() else {
            return;
        };
        let g = ui.global::<Settings>();
        let c = &e.config;
        let name_error = editor::name_error(&self.instances, &e.name, &e.id);
        g.set_name_error(name_error.into());
        g.set_command_error(command_error(&c.command).into());
        g.set_working_directory_error(working_directory_error(&c.working_directory).into());
        g.set_network_deny(c.network_default == NetworkAction::Deny);
        g.set_network_allow_local(c.network_allow_local);
        self.lists.show(c);

        let dirty = e.name.trim() != e.saved_name || cleaned(c) != e.saved;
        let check = check(e, name_error);
        let kind = save_kind(c, self.control.live(&e.id));
        g.set_dirty(dirty);
        g.set_notice(e.notice.into());
        g.set_save_kind(kind);
        // A reboot needs every shared folder.
        g.set_can_save(
            dirty
                && match check {
                    Ok(None) => true,
                    Ok(Some(_)) => kind != SaveKind::Reboot,
                    Err(_) => false,
                },
        );
        g.set_note(
            match (&check, dirty) {
                _ if !e.save_error.is_empty() => e.save_error.clone(),
                (Err(error), true) => error.to_string(),
                (Ok(Some(problem)), true) => problem.clone(),
                _ => String::new(),
            }
            .into(),
        );
    }

    /// Saves the edits and applies them to the running instance, then restarts its agent
    /// or reboots it in agent mode if the edits need that (`save_kind`).
    fn save(&self) {
        let (id, name, config, rename, write, kind) = {
            let shown = self.shown.borrow();
            let Some(e) = shown.as_ref() else {
                return;
            };
            let name_error = editor::name_error(&self.instances, &e.name, &e.id);
            let kind = save_kind(&e.config, self.control.live(&e.id));
            match check(e, name_error) {
                Ok(None) => {}
                Ok(Some(_)) if kind != SaveKind::Reboot => {}
                _ => return,
            }
            let name = e.name.trim().to_owned();
            let config = cleaned(&e.config);
            let rename = name != e.saved_name;
            let write = config != e.saved;
            (e.id.clone(), name, config, rename, write, kind)
        };
        if !rename && !write {
            return;
        }

        let saved = (|| -> db::Result<Option<i64>> {
            if rename {
                self.db.rename_instance(&id, &name)?;
            }
            if write {
                return Ok(Some(self.db.save_config(&id, &config)?));
            }
            Ok(None)
        })();
        let revision = match saved {
            Ok(revision) => revision,
            Err(error) => {
                return self
                    .edit(|e| e.save_error = format!("Couldn't save the settings: {error}"));
            }
        };

        let applied = kind != SaveKind::Reboot
            && revision.is_some_and(|number| self.control.apply(&id, number, &config));
        let notice = match (revision, kind) {
            (None, _) => "Saved.",
            (_, SaveKind::Reboot) => "Saved; the instance is rebooting.",
            (_, SaveKind::RestartAgent) if applied => "Saved and applied; the agent is restarting.",
            _ if applied => "Saved and applied to the running instance.",
            _ if self.control.running_boot(&id).is_some() => {
                "Saved. Changes apply on the next boot."
            }
            _ => "Saved. Changes apply when you start the instance.",
        };
        self.update_row(&id, &name, &config.command);
        self.open(&id);
        if let Some(shown) = self.shown.borrow_mut().as_mut() {
            shown.notice = notice;
        }
        self.refresh();
        match kind {
            SaveKind::Reboot => self.control.restart(&id),
            // After the Update, so the agent starts with the new settings.
            SaveKind::RestartAgent if applied => self.control.restart_agent(&id),
            _ => {}
        }
    }

    /// Updates the instance's sidebar row after a save and keeps it in name order.
    pub fn update_row(&self, id: &str, name: &str, command: &str) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let Some((row, mut item)) = (0..self.instances.row_count())
            .filter_map(|row| Some((row, self.instances.row_data(row)?)))
            .find(|(_, item)| item.id == id)
        else {
            return;
        };
        item.name = name.into();
        item.agent = command.into();

        let selected = usize::try_from(ui.get_selected_index())
            .ok()
            .and_then(|row| self.instances.row_data(row))
            .map(|item| item.id);
        self.instances.remove(row);
        let lower = name.to_lowercase();
        let row = (0..self.instances.row_count())
            .find(|&row| {
                self.instances
                    .row_data(row)
                    .is_some_and(|item| item.name.to_lowercase() > lower)
            })
            .unwrap_or(self.instances.row_count());
        self.instances.insert(row, item);
        if let Some(selected) = selected
            && let Some(row) = (0..self.instances.row_count()).find(|&row| {
                self.instances
                    .row_data(row)
                    .is_some_and(|item| item.id == selected)
            })
        {
            ui.set_selected_index(row as i32);
        }
    }
}

/// Whether the edits can be saved: an error if not; otherwise a problem that keeps the
/// instance from booting with them (a shared folder that is gone), if there is one.
fn check(e: &Shown, name_error: &'static str) -> Result<Option<String>, &'static str> {
    let c = &e.config;
    let invalid = !name_error.is_empty()
        || !command_error(&c.command).is_empty()
        || !working_directory_error(&c.working_directory).is_empty()
        || env_errors(&c.environment)
            .iter()
            .any(|error| !error.is_empty())
        || guest_path_errors(&c.mounts)
            .iter()
            .any(|error| !error.is_empty());
    if invalid {
        return Err("Fix the marked fields to save.");
    }
    Ok(c.mounts.iter().find_map(|mount| {
        let path = display_path(&mount.host_path);
        match folder_problem(&mount.host_path)? {
            FolderProblem::Missing => Some(format!(
                "{path} no longer exists on your Mac, so the instance can't start. You can still save."
            )),
            FolderProblem::Unreadable => Some(format!(
                "The app can't read {path}, so the instance can't start. You can still save."
            )),
        }
    }))
}

/// What Save does with `config` beyond saving it, given the running boot's mode and the
/// configuration in effect (`VmControl::live`): apply it, apply it and restart the agent for
/// new launch settings or environment, or reboot for new processors, memory or disk size.
/// Without a running boot that takes settings, it only saves.
fn save_kind(config: &Config, live: Option<(BootMode, Config)>) -> SaveKind {
    let Some((mode, active)) = live else {
        return SaveKind::Save;
    };
    let config = cleaned(config);
    if (config.cpus, config.memory_mib, config.disk_gb)
        != (active.cpus, active.memory_mib, active.disk_gb)
    {
        SaveKind::Reboot
    } else if mode == BootMode::Agent
        && (config.command != active.command
            || config.arguments != active.arguments
            || config.working_directory != active.working_directory
            || config.environment != active.environment)
    {
        SaveKind::RestartAgent
    } else {
        SaveKind::Save
    }
}

/// The configuration as it is saved: environment rows left empty are dropped.
fn cleaned(config: &Config) -> Config {
    let mut config = config.clone();
    config
        .environment
        .retain(|var| !var.name.is_empty() || !var.value.is_empty());
    config
}
