//! The third-party licenses window, opened from Preferences. It browses the generated
//! `licenses.rs` (see about.hbs): license types, the crates under the selected one, and
//! the selected crate's license text. Searching filters the crates by name.

use std::cell::RefCell;
use std::rc::Rc;

use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::licenses::{Crate, LICENSES, License};
use crate::{LicenseGroup, LicensedCrate, LicensesWindow};

/// A crate and the license text that covers it.
type Entry = (&'static Crate, &'static License);

/// A license type (one SPDX id) and all crates under it, sorted by name.
struct Group {
    name: &'static str,
    entries: Vec<Entry>,
}

struct Browser {
    groups: Vec<Group>,
    search: String,
    /// The selected type, an index into `groups`.
    group: usize,
    /// The types that have crates matching the search, as indices into `groups`.
    visible: Vec<usize>,
    /// The selected type's crates that match the search.
    crates: Vec<Entry>,
}

/// The window is created on first use and kept, so reopening it keeps the selection.
#[derive(Default)]
pub struct Licenses {
    window: RefCell<Option<LicensesWindow>>,
}

impl Licenses {
    pub fn open(&self) {
        if self.window.borrow().is_none() {
            match create() {
                Ok(window) => *self.window.borrow_mut() = Some(window),
                Err(error) => {
                    eprintln!("Couldn't open the licenses window: {error}");
                    return;
                }
            }
        }
        if let Some(window) = &*self.window.borrow()
            && let Err(error) = window.show()
        {
            eprintln!("Couldn't show the licenses window: {error}");
        }
    }
}

fn create() -> Result<LicensesWindow, slint::PlatformError> {
    let window = LicensesWindow::new()?;

    let mut groups: Vec<Group> = Vec::new();
    for license in LICENSES {
        let entries = license.used_by.iter().map(|krate| (krate, license));
        match groups.iter_mut().find(|group| {
            group
                .entries
                .first()
                .is_some_and(|(_, other)| other.id == license.id)
        }) {
            Some(group) => group.entries.extend(entries),
            None => groups.push(Group {
                name: license.name,
                entries: entries.collect(),
            }),
        }
    }
    for group in &mut groups {
        group
            .entries
            .sort_by_key(|(krate, _)| (krate.name, krate.version));
    }
    groups.sort_by(|a, b| {
        b.entries
            .len()
            .cmp(&a.entries.len())
            .then(a.name.cmp(b.name))
    });

    let browser = Rc::new(RefCell::new(Browser {
        groups,
        search: String::new(),
        group: 0,
        visible: Vec::new(),
        crates: Vec::new(),
    }));
    browser.borrow_mut().render(&window, 0);

    let (b, handle) = (browser.clone(), window.as_weak());
    window.on_group_selected(move |row| {
        let Some(window) = handle.upgrade() else {
            return;
        };
        let mut browser = b.borrow_mut();
        if let Some(&group) = browser.visible.get(row as usize) {
            browser.group = group;
            browser.render(&window, 0);
        }
    });
    let (b, handle) = (browser.clone(), window.as_weak());
    window.on_crate_selected(move |row| {
        if let Some(window) = handle.upgrade() {
            b.borrow_mut().render(&window, row as usize);
        }
    });
    let handle = window.as_weak();
    window.on_search_edited(move |text| {
        if let Some(window) = handle.upgrade() {
            let mut browser = browser.borrow_mut();
            browser.search = text.trim().to_lowercase();
            browser.render(&window, 0);
        }
    });
    // `open` hands the link to the default browser and returns right away.
    window.on_open_url(|url| {
        if let Err(error) = std::process::Command::new("open")
            .arg(url.as_str())
            .status()
        {
            eprintln!("Couldn't open {url}: {error}");
        }
    });
    let handle = window.as_weak();
    window.on_drag_window(move || {
        if let Some(window) = handle.upgrade() {
            window.window().with_winit_window(|window| {
                let _ = window.drag_window();
            });
        }
    });

    Ok(window)
}

impl Browser {
    /// Filters by the search, keeps the selected type if it still has matches (else the first
    /// that has), selects its crate at `row` and shows it all.
    fn render(&mut self, window: &LicensesWindow, row: usize) {
        let search = &self.search;
        let matches = |(krate, _): &&Entry| krate.name.to_lowercase().contains(search.as_str());

        self.visible = (0..self.groups.len())
            .filter(|&group| self.groups[group].entries.iter().any(|e| matches(&e)))
            .collect();
        if !self.visible.contains(&self.group) {
            self.group = self.visible.first().copied().unwrap_or(0);
        }
        self.crates = self.groups[self.group]
            .entries
            .iter()
            .filter(matches)
            .copied()
            .collect();

        let groups: Vec<LicenseGroup> = self
            .visible
            .iter()
            .map(|&group| LicenseGroup {
                name: self.groups[group].name.into(),
                count: self.groups[group].entries.iter().filter(matches).count() as i32,
            })
            .collect();
        window.set_groups(ModelRc::new(VecModel::from(groups)));
        window.set_selected_group(
            self.visible
                .iter()
                .position(|&group| group == self.group)
                .map_or(-1, |row| row as i32),
        );
        let crates: Vec<LicensedCrate> = self
            .crates
            .iter()
            .map(|(krate, _)| LicensedCrate {
                name: krate.name.into(),
                version: krate.version.into(),
            })
            .collect();
        window.set_crates(ModelRc::new(VecModel::from(crates)));

        let row = row.min(self.crates.len().saturating_sub(1));
        window.set_selected_crate(row as i32);
        let entry = self.crates.get(row);
        let krate = |field: fn(&Crate) -> &str| entry.map_or("", |(krate, _)| field(krate)).into();
        let license =
            |field: fn(&License) -> &str| entry.map_or("", |(_, license)| field(license)).into();
        window.set_crate_name(krate(|krate| krate.name));
        window.set_crate_version(krate(|krate| krate.version));
        window.set_url(krate(|krate| krate.url));
        window.set_license_name(license(|license| license.name));
        window.set_license_id(license(|license| license.id));
        window.set_license_text(license(|license| license.text));
    }
}
