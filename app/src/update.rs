//! Updates through Velopack, from the GitHub releases that .github/workflows/release.yml
//! publishes: stable releases, and dev prereleases for every change on main.
//!
//! The app checks at launch and every few hours, downloads an update in the background
//! (as a delta when it can) and installs it when the user restarts from Preferences.
//! Restarting shuts the running instances down gracefully first. An update that is
//! downloaded but not installed is applied by Velopack at the next launch.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::mpsc;
use std::time::Duration;

use semver::Version;
use slint::{ComponentHandle, Timer, TimerMode};
use velopack::sources::{GithubSource, HttpSource};
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, UpdateOptions};

use crate::controller::VmControl;
use crate::db::Db;
use crate::guest::REPO_URL;
use crate::{AppWindow, UpdateState, Updates};

const CHANNEL_KEY: &str = "update_channel";
const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);

/// The version shown in Preferences: the release's, or the crate's in development builds.
pub fn version() -> &'static str {
    option_env!("AGENTARIUM_VERSION")
        .filter(|version| !version.is_empty())
        .unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Channel {
    Stable,
    Dev,
}

impl Channel {
    /// The channel this build was released on, and the default.
    fn of_build() -> Channel {
        Channel::named(option_env!("AGENTARIUM_CHANNEL").unwrap_or("dev"))
    }

    fn named(name: &str) -> Channel {
        if name == "stable" {
            Channel::Stable
        } else {
            Channel::Dev
        }
    }

    fn name(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Dev => "dev",
        }
    }

    /// The release feeds to check: dev installs also take stable releases once those are
    /// newer, as every stable release is newer than the dev builds before it.
    fn feeds(self) -> &'static [Channel] {
        match self {
            Channel::Stable => &[Channel::Stable],
            Channel::Dev => &[Channel::Dev, Channel::Stable],
        }
    }

    fn manager(self) -> Result<UpdateManager, velopack::Error> {
        let options = UpdateOptions {
            AllowVersionDowngrade: false,
            ExplicitChannel: Some(self.name().into()),
            MaximumDeltasBeforeFallback: 10,
        };
        match self {
            // GitHub's "latest" release is always the newest stable one; prereleases never
            // count, however many dev builds came after it.
            Channel::Stable => UpdateManager::new(
                HttpSource::new(format!("{REPO_URL}/releases/latest/download")),
                Some(options),
                None,
            ),
            // The GitHub source reads the ten newest releases, which hold the newest dev builds.
            Channel::Dev => {
                UpdateManager::new(GithubSource::new(REPO_URL, None, true), Some(options), None)
            }
        }
    }
}

/// An update and the manager that found it, which also downloads and installs it.
struct Found {
    manager: UpdateManager,
    info: UpdateInfo,
}

impl Found {
    fn version(&self) -> String {
        self.info.TargetFullRelease.Version.clone()
    }
}

enum Event {
    /// Not installed by Velopack, e.g. started with `cargo run`.
    Unavailable,
    UpToDate,
    Downloading {
        version: String,
        notes: String,
    },
    Progress(i16),
    Downloaded(Box<Found>),
    Failed(String),
}

thread_local! {
    static UPDATER: RefCell<Weak<Updater>> = const { RefCell::new(Weak::new()) };
}

/// Owns the update state in the Preferences view (`Updates`).
pub struct Updater {
    ui: slint::Weak<AppWindow>,
    db: Rc<Db>,
    control: Rc<VmControl>,
    channel: Cell<Channel>,
    /// Counts checks; events of an earlier check (before a channel switch) are dropped.
    check_number: Cell<u32>,
    busy: Cell<bool>,
    downloading: RefCell<String>,
    ready: RefCell<Option<Box<Found>>>,
    install_on_exit: Cell<bool>,
    timer: Timer,
}

impl Updater {
    pub fn new(ui: &AppWindow, db: Rc<Db>, control: Rc<VmControl>) -> Rc<Updater> {
        let channel = match db.preference(CHANNEL_KEY) {
            Ok(Some(name)) => Channel::named(&name),
            Ok(None) => Channel::of_build(),
            Err(error) => {
                eprintln!("Couldn't read the update channel: {error}");
                Channel::of_build()
            }
        };
        let updater = Rc::new(Updater {
            ui: ui.as_weak(),
            db,
            control,
            channel: Cell::new(channel),
            check_number: Cell::new(0),
            busy: Cell::new(false),
            downloading: RefCell::new(String::new()),
            ready: RefCell::new(None),
            install_on_exit: Cell::new(false),
            timer: Timer::default(),
        });
        UPDATER.set(Rc::downgrade(&updater));

        let g = ui.global::<Updates>();
        g.set_version(version().into());
        g.set_channel(i32::from(channel == Channel::Dev));
        let weak = Rc::downgrade(&updater);
        g.on_set_channel(move |index| {
            if let Some(updater) = weak.upgrade() {
                updater.set_channel(if index == 1 {
                    Channel::Dev
                } else {
                    Channel::Stable
                });
            }
        });
        let weak = Rc::downgrade(&updater);
        g.on_check(move || {
            if let Some(updater) = weak.upgrade() {
                updater.check();
            }
        });
        let weak = Rc::downgrade(&updater);
        g.on_restart(move || {
            if let Some(updater) = weak.upgrade() {
                updater.restart();
            }
        });

        let weak = Rc::downgrade(&updater);
        updater
            .timer
            .start(TimerMode::Repeated, CHECK_EVERY, move || {
                if let Some(updater) = weak.upgrade() {
                    updater.check();
                }
            });
        updater.check();
        updater
    }

    /// Looks for an update on a thread of its own and downloads it right away. Nothing
    /// happens while a check runs or an update waits for the restart.
    fn check(&self) {
        if self.busy.get() || self.ready.borrow().is_some() {
            return;
        }
        self.busy.set(true);
        let number = self.check_number.get() + 1;
        self.check_number.set(number);
        self.show(UpdateState::Checking, "Checking for updates…");

        let channel = self.channel.get();
        let spawned = std::thread::Builder::new()
            .name("update".into())
            .spawn(move || run(channel, &|event| report(number, event)));
        if let Err(error) = spawned {
            self.busy.set(false);
            self.show(
                UpdateState::Failed,
                &format!("Couldn't check for updates: {error}"),
            );
        }
    }

    fn set_channel(&self, channel: Channel) {
        if channel == self.channel.get() {
            return;
        }
        self.channel.set(channel);
        if let Err(error) = self.db.set_preference(CHANNEL_KEY, channel.name()) {
            eprintln!("Couldn't save the update channel: {error}");
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Updates>()
                .set_channel(i32::from(channel == Channel::Dev));
        }
        // What was found or is being fetched belongs to the other channel.
        self.check_number.set(self.check_number.get() + 1);
        self.busy.set(false);
        self.ready.borrow_mut().take();
        self.check();
    }

    /// Shuts the instances down gracefully and quits; `install_pending` then installs the
    /// update and relaunches.
    fn restart(&self) {
        if self.ready.borrow().is_none() {
            return;
        }
        self.install_on_exit.set(true);
        self.show(UpdateState::Ready, "Shutting instances down to restart…");
        self.control.quit();
    }

    /// Runs after the event loop ended: installs the update the user restarted for. This
    /// exits the process and relaunches the new version.
    pub fn install_pending(&self) {
        if !self.install_on_exit.get() {
            return;
        }
        if let Some(found) = self.ready.borrow().as_ref()
            && let Err(error) = found.manager.apply_updates_and_restart(&found.info)
        {
            eprintln!("Couldn't install the update: {error}");
        }
    }

    fn handle(&self, number: u32, event: Event) {
        if number != self.check_number.get() {
            return;
        }
        match event {
            Event::Unavailable => {
                self.busy.set(false);
                self.show(
                    UpdateState::Unavailable,
                    "Updates work only in installed builds.",
                );
            }
            Event::UpToDate => {
                self.busy.set(false);
                self.show(UpdateState::UpToDate, "Agentarium is up to date.");
            }
            Event::Downloading { version, notes } => {
                if let Some(ui) = self.ui.upgrade() {
                    let g = ui.global::<Updates>();
                    g.set_available(version.as_str().into());
                    g.set_notes(notes.into());
                    g.set_progress(0.0);
                }
                self.show(
                    UpdateState::Downloading,
                    &format!("Downloading version {version}…"),
                );
                *self.downloading.borrow_mut() = version;
            }
            Event::Progress(percent) => {
                if let Some(ui) = self.ui.upgrade() {
                    let g = ui.global::<Updates>();
                    g.set_progress(f32::from(percent.clamp(0, 100)) / 100.0);
                    g.set_status(
                        format!(
                            "Downloading version {} · {percent}%",
                            self.downloading.borrow()
                        )
                        .into(),
                    );
                }
            }
            Event::Downloaded(found) => {
                self.busy.set(false);
                let version = found.version();
                *self.ready.borrow_mut() = Some(found);
                self.show(
                    UpdateState::Ready,
                    &format!("Version {version} is ready to install."),
                );
            }
            Event::Failed(message) => {
                self.busy.set(false);
                self.show(UpdateState::Failed, &message);
            }
        }
    }

    fn show(&self, state: UpdateState, status: &str) {
        if let Some(ui) = self.ui.upgrade() {
            let g = ui.global::<Updates>();
            g.set_state(state);
            g.set_status(status.into());
        }
    }
}

fn report(number: u32, event: Event) {
    let _ = slint::invoke_from_event_loop(move || {
        UPDATER.with_borrow(|updater| {
            if let Some(updater) = updater.upgrade() {
                updater.handle(number, event);
            }
        })
    });
}

/// Checks the channel's feeds and downloads the newest update found.
fn run(channel: Channel, report: &(dyn Fn(Event) + Sync)) {
    let mut best: Option<Found> = None;
    for &feed in channel.feeds() {
        let Ok(manager) = feed.manager() else {
            return report(Event::Unavailable);
        };
        match manager.check_for_updates() {
            Ok(UpdateCheck::UpdateAvailable(info)) => {
                let found = Found {
                    manager,
                    info: *info,
                };
                let newer = |found: &Found| Version::parse(&found.version()).ok();
                if best.as_ref().is_none_or(|best| newer(&found) > newer(best)) {
                    best = Some(found);
                }
            }
            Ok(UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty) => {}
            // A new stable release has no files yet while its build still uploads them.
            Err(error) if feed == Channel::Stable && error.to_string().contains("404") => {}
            Err(error) => {
                return report(Event::Failed(format!(
                    "Couldn't check for updates: {error}"
                )));
            }
        }
    }
    let Some(found) = best else {
        return report(Event::UpToDate);
    };

    report(Event::Downloading {
        version: found.version(),
        notes: found.info.TargetFullRelease.NotesMarkdown.clone(),
    });
    let (progress, received) = mpsc::channel::<i16>();
    let downloaded = std::thread::scope(|scope| {
        scope.spawn(|| {
            for percent in received {
                report(Event::Progress(percent));
            }
        });
        found.manager.download_updates(&found.info, Some(progress))
    });
    match downloaded {
        Ok(()) => report(Event::Downloaded(Box::new(found))),
        Err(error) => report(Event::Failed(format!(
            "Couldn't download the update: {error}"
        ))),
    }
}
