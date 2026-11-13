//! The Snapshots tab: snapshots of the shown instance's disk, each with the configuration
//! revision that was the latest when it was taken. See design/snapshots.md.
//!
//! A snapshot is an APFS clone of the stopped instance's disk (`vm::create_disk`), kept in
//! the instance's `snapshots` folder. Restoring one clones it back over the disk and saves
//! its revision's configuration for the next boot. Both need the instance to be stopped;
//! deleting a snapshot doesn't. After a successful provisioning run, the controller takes a
//! "Freshly provisioned" snapshot unless the user turned that off in Preferences.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::controller::VmControl;
use crate::db::{self, Db};
use crate::settings::SettingsEditor;
use crate::vm;
use crate::{AppWindow, SnapshotItem, Snapshots};

/// The preference that turns the snapshot after provisioning on ("on", the default) or off.
const AUTO_SNAPSHOT_KEY: &str = "auto_snapshot";

/// Whether a snapshot is taken once provisioning succeeded.
pub fn auto_snapshot(db: &Db) -> bool {
    db.preference(AUTO_SNAPSHOT_KEY)
        .inspect_err(|error| eprintln!("Couldn't read the snapshot preference: {error}"))
        .ok()
        .flatten()
        .is_none_or(|value| value != "off")
}

/// Takes a snapshot of the stopped instance's disk, linking its latest revision.
pub fn take(db: &Db, instance_id: &str, name: &str) -> db::Result<()> {
    let instance = db.instance(instance_id)?;
    let revision = db.latest_revision(instance_id)?.number;
    let id = uuid::Uuid::now_v7().to_string();
    let disk = db::instance_dir(instance_id)?
        .join("snapshots")
        .join(format!("{id}.ext4"));
    vm::create_disk(&instance.disk_path, &disk)?;
    db.add_snapshot(&id, instance_id, name, revision, &disk)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&disk);
        })
}

/// The shown instance and its snapshots, newest first, as the UI lists them.
struct Shown {
    id: String,
    snapshots: Vec<db::Snapshot>,
}

pub struct SnapshotsView {
    ui: slint::Weak<AppWindow>,
    db: Rc<Db>,
    control: Rc<VmControl>,
    settings: Rc<SettingsEditor>,
    shown: RefCell<Option<Shown>>,
}

impl SnapshotsView {
    pub fn new(
        ui: &AppWindow,
        db: Rc<Db>,
        control: Rc<VmControl>,
        settings: Rc<SettingsEditor>,
    ) -> Rc<SnapshotsView> {
        let this = Rc::new(SnapshotsView {
            ui: ui.as_weak(),
            db,
            control,
            settings,
            shown: RefCell::new(None),
        });

        let g = ui.global::<Snapshots>();
        g.set_auto_snapshot(auto_snapshot(&this.db));
        let s = this.clone();
        g.on_set_auto_snapshot(move |on| {
            if let Err(error) =
                s.db.set_preference(AUTO_SNAPSHOT_KEY, if on { "on" } else { "off" })
            {
                eprintln!("Couldn't save the snapshot preference: {error}");
            }
            if let Some(ui) = s.ui.upgrade() {
                ui.global::<Snapshots>().set_auto_snapshot(on);
            }
        });
        let s = this.clone();
        g.on_create(move |name| s.create(name.trim()));
        let s = this.clone();
        g.on_restore(move |index| s.restore(index));
        let s = this.clone();
        g.on_delete(move |index| s.delete(index));
        let s = this.clone();
        g.on_refresh(move || {
            let id = s.shown.borrow().as_ref().map(|shown| shown.id.clone());
            if let Some(id) = id {
                s.open(&id);
            }
        });
        this
    }

    /// Lists the instance's snapshots; an error shown for the previous one is cleared.
    pub fn open(&self, id: &str) {
        let snapshots = if id.is_empty() {
            Vec::new()
        } else {
            self.db.snapshots(id).unwrap_or_else(|error| {
                eprintln!("Couldn't read the instance's snapshots: {error}");
                Vec::new()
            })
        };
        if let Some(ui) = self.ui.upgrade() {
            let g = ui.global::<Snapshots>();
            let items: Vec<SnapshotItem> = snapshots
                .iter()
                .map(|snapshot| SnapshotItem {
                    name: snapshot.name.as_str().into(),
                    created: snapshot.created.as_str().into(),
                    revision: snapshot.revision as i32,
                })
                .collect();
            g.set_items(ModelRc::new(VecModel::from(items)));
            g.set_suggested_name(format!("Snapshot {}", snapshots.len() + 1).into());
            g.set_error("".into());
        }
        *self.shown.borrow_mut() = Some(Shown {
            id: id.into(),
            snapshots,
        });
    }

    fn create(&self, name: &str) {
        let Some(id) = self.stopped_instance() else {
            return;
        };
        if name.is_empty() {
            return self.show_error("Name the snapshot to create it.");
        }
        match take(&self.db, &id, name) {
            Ok(()) => self.open(&id),
            Err(error) => self.show_error(&format!("Couldn't create the snapshot: {error}")),
        }
    }

    /// Replaces the stopped instance's disk with a clone of the snapshot and saves the
    /// snapshot's configuration for the next boot. The clone is made next to the disk first
    /// and only renamed over it once the configuration is saved, so a failure leaves both
    /// as they were.
    fn restore(&self, index: i32) {
        let Some(id) = self.stopped_instance() else {
            return;
        };
        let Some((disk, revision)) = self.snapshot(index, |s| (s.disk_path.clone(), s.revision))
        else {
            return;
        };
        let restored = (|| -> db::Result<(String, String)> {
            let instance = self.db.instance(&id)?;
            let config = self.db.revision(&id, revision)?.config;
            let temp = instance.disk_path.with_extension("ext4.restoring");
            if let Err(error) = std::fs::remove_file(&temp)
                && error.kind() != io::ErrorKind::NotFound
            {
                return Err(error.into());
            }
            vm::create_disk(&disk, &temp)?;
            let replaced = (|| -> db::Result<()> {
                if self.db.latest_revision(&id)?.config != config {
                    self.db.save_config(&id, &config)?;
                }
                std::fs::rename(&temp, &instance.disk_path)?;
                Ok(())
            })();
            if replaced.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            replaced.map(|()| (instance.name, config.command))
        })();
        match restored {
            Ok((name, command)) => {
                self.settings.update_row(&id, &name, &command);
                self.settings.open(&id);
                self.open(&id);
            }
            Err(error) => self.show_error(&format!("Couldn't restore the snapshot: {error}")),
        }
    }

    fn delete(&self, index: i32) {
        let Some((snapshot_id, disk)) =
            self.snapshot(index, |s| (s.id.clone(), s.disk_path.clone()))
        else {
            return;
        };
        let Some(id) = self.shown.borrow().as_ref().map(|shown| shown.id.clone()) else {
            return;
        };
        if let Err(error) = self.db.delete_snapshot(&snapshot_id) {
            return self.show_error(&format!("Couldn't delete the snapshot: {error}"));
        }
        self.open(&id);
        if let Err(error) = std::fs::remove_file(&disk)
            && error.kind() != io::ErrorKind::NotFound
        {
            self.show_error(&format!(
                "The snapshot was deleted, but its disk couldn't be removed: {} ({error})",
                disk.display()
            ));
        }
    }

    /// The shown instance, unless its VM runs; the UI only offers creating and restoring
    /// while it is stopped.
    fn stopped_instance(&self) -> Option<String> {
        let id = self.shown.borrow().as_ref()?.id.clone();
        if self.control.running_boot(&id).is_some() {
            self.show_error("Stop the instance to create or restore snapshots.");
            return None;
        }
        Some(id)
    }

    fn snapshot<T>(&self, index: i32, get: impl FnOnce(&db::Snapshot) -> T) -> Option<T> {
        let shown = self.shown.borrow();
        let snapshot = shown
            .as_ref()?
            .snapshots
            .get(usize::try_from(index).ok()?)?;
        Some(get(snapshot))
    }

    fn show_error(&self, error: &str) {
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Snapshots>().set_error(error.into());
        }
    }
}
