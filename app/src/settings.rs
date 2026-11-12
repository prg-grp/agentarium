//! The Settings tab: the shown instance's configuration, edited for its next boot. See
//! design/settings-pending.md.
//!
//! Edits stay here until the user saves: Save writes them into the pending revision (a new
//! one if the latest was booted) and renames the instance; Save and restart also boots it in
//! agent mode. The UI's `Settings` global shows the edits; every edit goes through a callback.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::controller::VmControl;
use crate::db::{self, Config, Db, NetworkAction, RuleTarget};
use crate::editor::{
    self, FolderProblem, Host, Lists, command_error, display_path, env_errors, folder_problem,
    guest_path_errors, pick_folder, remove, working_directory_error,
};
use crate::preset::Preset;
use crate::{AppWindow, InstanceItem, Settings};

/// The shown instance: what is saved, and the edits.
struct Shown {
    id: String,
    saved_name: String,
    /// Until provisioning has succeeded, the provisioning script can be edited too.
    provisioned: bool,
    /// The latest revision's configuration.
    saved: Config,
    /// The newest booted revision's configuration, if the instance was ever booted.
    active: Option<Config>,
    /// The disk's current size in GiB, at least `DISK_MIN_GB`: disks only grow.
    min_disk_gb: u32,
    name: String,
    config: Config,
    /// Why the last save failed; cleared by the next edit.
    save_error: String,
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
        g.set_arguments(ModelRc::from(lists.arguments.clone()));
        g.set_environment(ModelRc::from(lists.environment.clone()));
        g.set_mounts(ModelRc::from(lists.mounts.clone()));
        g.set_allowed(ModelRc::from(lists.allowed.clone()));
        g.set_denied(ModelRc::from(lists.denied.clone()));

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
        g.on_save(move |restart| s.save(restart));

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
                    provisioned: instance.provisioned,
                    active: self.db.active_revision(id)?.map(|revision| revision.config),
                    min_disk_gb: (std::fs::metadata(&instance.disk_path)?
                        .len()
                        .div_ceil(1 << 30) as u32)
                        .max(db::DISK_MIN_GB),
                    name: instance.name,
                    config: saved.clone(),
                    saved,
                    save_error: String::new(),
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

        let changes = e
            .active
            .as_ref()
            .map_or([0; 5], |active| count_changes(active, &e.saved));
        g.set_section_changes(ModelRc::new(VecModel::from(changes.to_vec())));

        let dirty = e.name.trim() != e.saved_name || cleaned(c) != e.saved;
        let check = check(e, name_error);
        g.set_dirty(dirty);
        g.set_can_save(dirty && check.is_ok());
        g.set_can_restart(dirty && matches!(check, Ok(None)));
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

    /// Saves the edits, then boots the instance in agent mode if `restart` is set.
    fn save(&self, restart: bool) {
        let (id, name, config, rename, write) = {
            let shown = self.shown.borrow();
            let Some(e) = shown.as_ref() else {
                return;
            };
            let name_error = editor::name_error(&self.instances, &e.name, &e.id);
            match check(e, name_error) {
                Ok(None) => {}
                Ok(Some(_)) if !restart => {}
                _ => return,
            }
            let name = e.name.trim().to_owned();
            let config = cleaned(&e.config);
            let rename = name != e.saved_name;
            let write = config != e.saved;
            (e.id.clone(), name, config, rename, write)
        };
        if !rename && !write {
            return;
        }

        let saved = (|| -> db::Result<()> {
            if rename {
                self.db.rename_instance(&id, &name)?;
            }
            if write {
                self.db.save_config(&id, &config)?;
            }
            Ok(())
        })();
        if let Err(error) = saved {
            return self.edit(|e| e.save_error = format!("Couldn't save the settings: {error}"));
        }

        self.update_row(&id, &name, &config.command);
        self.open(&id);
        if restart {
            self.control.restart(&id);
        }
    }

    /// Updates the instance's sidebar row after a save and keeps it in name order.
    fn update_row(&self, id: &str, name: &str, command: &str) {
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
        match revision_fields(&self.db, id) {
            Ok((active, saved, changes)) => {
                item.active_revision = active;
                item.saved_revision = saved;
                item.pending_changes = changes;
            }
            Err(error) => eprintln!("Couldn't read the instance's revisions: {error}"),
        }

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
/// instance from booting with them (a provisioning script still to be written, a shared
/// folder that is gone), if there is one.
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
    // The script only runs while the instance isn't provisioned yet.
    if !e.provisioned {
        if c.provision_script.trim().is_empty() {
            return Ok(Some("The provisioning script is empty.".into()));
        }
        if c.provision_script == Preset::Custom.provision_script() {
            return Ok(Some(
                "Fill in the provisioning script so it installs your agent.".into(),
            ));
        }
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

/// The configuration as it is saved: environment rows left empty are dropped.
fn cleaned(config: &Config) -> Config {
    let mut config = config.clone();
    config
        .environment
        .retain(|var| !var.name.is_empty() || !var.value.is_empty());
    config
}

/// The instance's newest booted revision, its latest saved revision, and the number of
/// changes between them (0 and 0 changes if it was never booted).
pub fn revision_fields(db: &Db, id: &str) -> db::Result<(i32, i32, i32)> {
    let saved = db.latest_revision(id)?;
    Ok(match db.active_revision(id)? {
        Some(active) => (
            active.number as i32,
            saved.number as i32,
            count_changes(&active.config, &saved.config).iter().sum(),
        ),
        None => (0, saved.number as i32, 0),
    })
}

/// The changes from `old` to `new` per section of the Settings tab: agent and launch, shared
/// folders, network, environment, resources. A changed value, list item added, removed or
/// changed counts as one; the arguments count as one setting. The provisioning script isn't
/// counted: Settings edits it only before provisioning, and nothing is pending before the
/// first boot.
fn count_changes(old: &Config, new: &Config) -> [i32; 5] {
    let one = |changed: bool| i32::from(changed);
    let rule_key = |rule: &db::NetworkRule| {
        let target = match &rule.target {
            RuleTarget::Domain { name, .. } => name,
            RuleTarget::Cidr { network } => network,
        };
        (rule.action, target.clone())
    };
    [
        one(old.command != new.command)
            + one(old.arguments != new.arguments)
            + one(old.working_directory != new.working_directory),
        list_changes(&old.mounts, &new.mounts, |mount| mount.host_path.clone()),
        one(old.network_default != new.network_default)
            + one(old.network_allow_local != new.network_allow_local)
            + list_changes(&old.network_rules, &new.network_rules, rule_key),
        list_changes(&old.environment, &new.environment, |var| var.name.clone()),
        one(old.cpus != new.cpus)
            + one(old.memory_mib != new.memory_mib)
            + one(old.disk_gb != new.disk_gb),
    ]
}

/// Items of `old` that are gone or changed in `new`, plus items of `new` that are added.
fn list_changes<T: PartialEq, K: PartialEq>(old: &[T], new: &[T], key: impl Fn(&T) -> K) -> i32 {
    let gone = old.iter().filter(|item| !new.contains(item)).count();
    let added = new
        .iter()
        .filter(|item| !old.iter().any(|other| key(other) == key(item)))
        .count();
    (gone + added) as i32
}
