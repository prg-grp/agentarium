//! The Activity tab: what the shown instance's agent did in one run. See
//! design/activity-commands.md.
//!
//! This side runs on the UI thread. It keeps what the user chose (run, view, filters,
//! expanded rows, selection), asks the loader in activity_data.rs for the view on every
//! change, and shows each answer. While the shown run is still going, it asks again every
//! two seconds as long as the tab is visible. The UI thread never reads activity itself.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::activity_data::{self, Loaded, Loader, Request, View};
use crate::controller::VmControl;
use crate::db::Db;
use crate::{
    Activity, ActivityRow, AppWindow, DetailEntry, DetailField, DetailItem, DetailSection, RunItem,
    Tone,
};

const REFRESH: Duration = Duration::from_secs(2);
const VIEWS: [View; 4] = [View::Commands, View::Files, View::Network, View::Events];
const VIEW_NAMES: [&str; 4] = ["Commands", "Files", "Network", "All events"];

thread_local! {
    static SHOWN: RefCell<Weak<ActivityView>> = const { RefCell::new(Weak::new()) };
}

struct State {
    instance: Option<String>,
    /// The instance's agent runs (boot ids), newest first.
    runs: Vec<i64>,
    run: usize,
    view: usize,
    search: String,
    show_reads: bool,
    failed_only: bool,
    kinds: [bool; 3],
    toggled: HashSet<String>,
    /// The detail pages opened, in order: first a row of the list, then items of the pages.
    /// Empty while the list shows.
    trail: Vec<String>,
    /// The page the UI shows, to close its collapsed parts when another opens.
    page: Option<String>,
    visible: bool,
    /// Whether the shown run was going at the last refresh.
    live: bool,
    generation: u64,
}

pub struct ActivityView {
    ui: slint::Weak<AppWindow>,
    db: Rc<Db>,
    control: Rc<VmControl>,
    state: RefCell<State>,
    loader: Loader,
    rows: Rc<VecModel<ActivityRow>>,
    timer: Timer,
}

impl ActivityView {
    pub fn new(ui: &AppWindow, db: Rc<Db>, control: Rc<VmControl>) -> Rc<ActivityView> {
        let loader = Loader::new(|loaded| {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(view) = SHOWN.with_borrow(Weak::upgrade) {
                    view.show(loaded);
                }
            });
        });
        let rows = Rc::new(VecModel::default());
        let global = ui.global::<Activity>();
        global.set_rows(ModelRc::from(rows.clone()));
        let this = Rc::new(ActivityView {
            ui: ui.as_weak(),
            db,
            control,
            state: RefCell::new(State {
                instance: None,
                runs: Vec::new(),
                run: 0,
                view: 0,
                search: String::new(),
                show_reads: true,
                failed_only: false,
                kinds: [true; 3],
                toggled: HashSet::new(),
                trail: Vec::new(),
                page: None,
                visible: false,
                live: false,
                generation: 0,
            }),
            loader,
            rows,
            timer: Timer::default(),
        });
        SHOWN.with_borrow_mut(|shown| *shown = Rc::downgrade(&this));

        global.on_set_view(changing(&this, |state, index: i32| {
            state.view = index as usize;
            state.trail.clear();
        }));
        global.on_pick_run(changing(&this, |state, index: i32| {
            state.run = index as usize;
            state.toggled.clear();
            state.trail.clear();
        }));
        global.on_search_edited(changing(&this, |state, text: slint::SharedString| {
            state.search = text.trim().to_lowercase();
        }));
        let toggle = changing(&this, |state, ()| state.show_reads = !state.show_reads);
        global.on_toggle_reads(move || toggle(()));
        let toggle = changing(&this, |state, ()| state.failed_only = !state.failed_only);
        global.on_toggle_failed(move || toggle(()));
        global.on_toggle_kind(changing(&this, |state, kind: i32| {
            if let Some(on) = state.kinds.get_mut(kind as usize) {
                *on = !*on;
            }
        }));
        global.on_toggle_row(changing(&this, |state, key: slint::SharedString| {
            let key = key.to_string();
            if !state.toggled.remove(&key) {
                state.toggled.insert(key);
            }
        }));
        global.on_select_row(changing(&this, |state, key: slint::SharedString| {
            // "and N more" rows have nothing to show. From the list a row starts the trail;
            // from a page, an item goes one level deeper.
            if !key.contains("#more") {
                state.trail.push(key.into());
            }
        }));
        let back = changing(&this, |state, ()| {
            state.trail.pop();
        });
        global.on_back(move || back(()));

        let view = Rc::downgrade(&this);
        this.timer.start(TimerMode::Repeated, REFRESH, move || {
            if let Some(view) = view.upgrade() {
                view.tick();
            }
        });
        this
    }

    /// Shows the instance's runs: the running one, or else the newest.
    pub fn open(&self, id: &str) {
        {
            let mut state = self.state.borrow_mut();
            state.instance = Some(id.to_owned());
            state.toggled.clear();
            state.trail.clear();
        }
        self.load_runs(true);
        self.request();
    }

    /// The tab became visible or hidden. Only a visible tab refreshes.
    pub fn set_visible(&self, visible: bool) {
        self.state.borrow_mut().visible = visible;
        if visible {
            self.load_runs(false);
            self.request();
        }
    }

    fn change(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.state.borrow_mut());
        self.request();
    }

    /// Every two seconds: a new run shows up in the picker, and the shown run is read
    /// again while it goes (and once more when it ended, for the last exits).
    fn tick(&self) {
        let (visible, instance, shown, was_live) = {
            let state = self.state.borrow();
            (
                state.visible,
                state.instance.clone(),
                state.runs.get(state.run).copied(),
                state.live,
            )
        };
        let Some(id) = instance.filter(|_| visible) else {
            return;
        };
        let running = self.control.running_boot(&id);
        if running.is_some() && !self.state.borrow().runs.contains(&running.unwrap_or(0)) {
            // A new run started; follow it if the newest run was shown.
            let newest = self.state.borrow().run == 0;
            self.load_runs(newest);
            self.request();
        } else if running.is_some() && running == shown || was_live {
            self.request();
        }
    }

    /// Reads the instance's agent runs. `newest` selects the running run or the newest;
    /// otherwise the shown run stays selected.
    fn load_runs(&self, newest: bool) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let global = ui.global::<Activity>();
        let Some(id) = self.state.borrow().instance.clone() else {
            return;
        };
        let runs = self.db.agent_runs(&id).unwrap_or_else(|error| {
            eprintln!("Couldn't read the runs of {id}: {error}");
            Vec::new()
        });
        let running = self.control.running_boot(&id);
        let items: Vec<RunItem> = runs
            .iter()
            .map(|run| {
                let live = running == Some(run.id);
                let end = match (&run.ended, live) {
                    (_, true) => "now".to_owned(),
                    (Some(ended), false) => ended.clone(),
                    (None, false) => "unknown".to_owned(),
                };
                RunItem {
                    title: format!("Run {}, {} to {end}", run.number, run.started).into(),
                    detail: format!("Configuration revision {}", run.revision).into(),
                    live,
                }
            })
            .collect();
        global.set_placeholder(if runs.is_empty() {
            "No agent runs yet. Activity is recorded while the agent runs.".into()
        } else {
            "".into()
        });
        global.set_runs(ModelRc::new(VecModel::from(items)));

        let mut state = self.state.borrow_mut();
        let shown = state.runs.get(state.run).copied();
        state.runs = runs.iter().map(|run| run.id).collect();
        state.run = if newest {
            runs.iter()
                .position(|run| Some(run.id) == running)
                .unwrap_or(0)
        } else {
            shown
                .and_then(|shown| state.runs.iter().position(|&run| run == shown))
                .unwrap_or(0)
        };
    }

    /// Asks the loader for the current view and shows the choices right away.
    fn request(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let global = ui.global::<Activity>();
        let mut state = self.state.borrow_mut();
        global.set_run(state.run as i32);
        global.set_view(state.view as i32);
        global.set_show_reads(state.show_reads);
        global.set_failed_only(state.failed_only);
        global.set_show_processes(state.kinds[0]);
        global.set_show_files(state.kinds[1]);
        global.set_show_network(state.kinds[2]);
        global.set_selected(state.trail.first().cloned().unwrap_or_default().into());
        global.set_detail_open(!state.trail.is_empty());
        global.set_back_label(match state.trail.len() {
            1 => format!("← {}", VIEW_NAMES[state.view.min(VIEW_NAMES.len() - 1)]).into(),
            _ => "← Back".into(),
        });
        let page = state.trail.last().cloned();
        if page != state.page {
            global.set_args_open(false);
            global.set_record_open(false);
            state.page = page;
        }
        let (Some(id), Some(&boot)) = (state.instance.clone(), state.runs.get(state.run)) else {
            return;
        };
        state.live = self.control.running_boot(&id) == Some(boot);
        state.generation += 1;
        self.loader.load(Request {
            generation: state.generation,
            boot,
            live: state.live,
            view: VIEWS[state.view.min(VIEWS.len() - 1)],
            search: state.search.clone(),
            show_reads: state.show_reads,
            failed_only: state.failed_only,
            kinds: state.kinds,
            toggled: state.toggled.clone(),
            selected: state.trail.last().cloned(),
        });
    }

    fn show(&self, loaded: Loaded) {
        if loaded.generation != self.state.borrow().generation {
            return;
        }
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let global = ui.global::<Activity>();
        let rows: Vec<ActivityRow> = loaded
            .rows
            .into_iter()
            .map(|row| ActivityRow {
                key: row.key.into(),
                depth: row.depth,
                heading: row.heading,
                expandable: row.expandable,
                expanded: row.expanded,
                time: row.time.into(),
                label: row.label.into(),
                title: row.title.into(),
                subtitle: row.subtitle.into(),
                subtitle_tone: tone(row.subtitle_tone),
                meta: row.meta.into(),
                result: row.result.into(),
                tone: tone(row.tone),
                dim: row.dim,
            })
            .collect();
        self.rows.set_vec(rows);
        global.set_counts(ModelRc::new(VecModel::from(
            loaded
                .counts
                .into_iter()
                .map(Into::into)
                .collect::<Vec<slint::SharedString>>(),
        )));
        global.set_blocked(loaded.blocked.into());
        global.set_monitoring(loaded.monitoring.into());
        global.set_monitoring_tone(tone(loaded.monitoring_tone));
        global.set_note(loaded.note.into());
        global.set_empty(loaded.empty.into());

        let detail = loaded.detail.unwrap_or_default();
        global.set_detail_kind(detail.kind.into());
        global.set_detail_headline(detail.headline.into());
        global.set_detail_facts(ModelRc::new(VecModel::from(
            detail
                .facts
                .into_iter()
                .map(|fact| DetailField {
                    label: fact.label.into(),
                    value: fact.value.into(),
                    mono: fact.mono,
                })
                .collect::<Vec<_>>(),
        )));
        global.set_detail_script(detail.script.into());
        global.set_detail_argv(ModelRc::new(VecModel::from(
            detail
                .argv
                .into_iter()
                .map(Into::into)
                .collect::<Vec<slint::SharedString>>(),
        )));
        global.set_detail_argv_note(detail.argv_note.into());
        global.set_detail_sections(ModelRc::new(VecModel::from(
            detail
                .sections
                .into_iter()
                .map(|section| DetailSection {
                    title: section.title.into(),
                    note: section.note.into(),
                    items: ModelRc::new(VecModel::from(
                        section
                            .items
                            .into_iter()
                            .map(|item| DetailItem {
                                depth: item.depth,
                                text: item.text.into(),
                                detail: item.detail.into(),
                                tone: tone(item.tone),
                                key: item.key.into(),
                                mono: item.mono,
                            })
                            .collect::<Vec<_>>(),
                    )),
                })
                .collect::<Vec<_>>(),
        )));
        global.set_detail_record(ModelRc::new(VecModel::from(
            detail
                .record
                .into_iter()
                .map(|entry| DetailEntry {
                    time: entry.time.into(),
                    text: entry.text.into(),
                    tone: tone(entry.tone),
                })
                .collect::<Vec<_>>(),
        )));
        global.set_detail_note(detail.note.into());
    }
}

/// A callback that changes the state with its argument and asks for the view again.
fn changing<A>(
    view: &Rc<ActivityView>,
    change: impl Fn(&mut State, A) + 'static,
) -> impl Fn(A) + 'static {
    let view = Rc::downgrade(view);
    move |value| {
        if let Some(view) = view.upgrade() {
            view.change(|state| change(state, value));
        }
    }
}

fn tone(tone: activity_data::Tone) -> Tone {
    match tone {
        activity_data::Tone::Neutral => Tone::Neutral,
        activity_data::Tone::Muted => Tone::Muted,
        activity_data::Tone::Warn => Tone::Warn,
        activity_data::Tone::Bad => Tone::Bad,
    }
}
