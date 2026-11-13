//! Creating an instance: a draft edited in five steps (harness, agent, shared folders,
//! network, review), then created and, unless the user opts out, started right away so it
//! provisions. See design/create-harness.md and the other design/create-*.md files.
//!
//! The draft lives here; the UI's `Creation` global shows it. Every edit goes through a
//! callback, updates the draft and refreshes the lists and checks in the UI.

use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::db::{self, Config, Db, NetworkAction};
use crate::editor::{
    self, FolderProblem, Host, Lists, command_error, display_path, env_errors, folder_problem,
    forward_label, guest_path_errors, pick_folder, remove, rule_rows, same_target, sync,
    working_directory_error,
};
use crate::preset::Preset;
use crate::{AppWindow, CreateIssue, Creation, InstanceItem, InstanceState, PresetRow};
use crate::{guest, vm};

// Steps, as numbered in the step indicator (from 0).
const HARNESS: i32 = 0;
const AGENT: i32 = 1;
const FOLDERS: i32 = 2;
const REVIEW: i32 = 4;

struct Draft {
    name: String,
    preset: Preset,
    config: Config,
    /// The custom agent's command and script, kept while another preset is selected.
    custom_command: String,
    custom_script: String,
}

pub struct Creator {
    ui: slint::Weak<AppWindow>,
    db: Rc<Db>,
    /// The sidebar rows; the draft gets a row once it has a name.
    instances: Rc<VecModel<InstanceItem>>,
    /// Set while a new instance is being drafted.
    draft: RefCell<Option<Draft>>,
    lists: Lists,
    issues: Rc<VecModel<CreateIssue>>,
    /// Takes a created instance and whether to start it.
    created: Box<dyn Fn(db::Instance, bool)>,
}

impl Creator {
    pub fn new(
        ui: &AppWindow,
        db: Rc<Db>,
        instances: Rc<VecModel<InstanceItem>>,
        created: impl Fn(db::Instance, bool) + 'static,
    ) -> Rc<Creator> {
        let this = Rc::new(Creator {
            ui: ui.as_weak(),
            db,
            instances,
            draft: RefCell::new(None),
            lists: Lists::default(),
            issues: Rc::default(),
            created: Box::new(created),
        });

        let g = ui.global::<Creation>();
        let lists = &this.lists;
        g.set_arguments(ModelRc::from(lists.arguments.clone()));
        g.set_environment(ModelRc::from(lists.environment.clone()));
        g.set_mounts(ModelRc::from(lists.mounts.clone()));
        g.set_layout(ModelRc::from(lists.layout.clone()));
        g.set_allowed(ModelRc::from(lists.allowed.clone()));
        g.set_denied(ModelRc::from(lists.denied.clone()));
        g.set_forwards(ModelRc::from(lists.forwards.clone()));
        g.set_issues(ModelRc::from(this.issues.clone()));
        let presets: Vec<PresetRow> = Preset::ALL
            .iter()
            .map(|preset| PresetRow {
                name: preset.name().into(),
                note: preset.note().into(),
            })
            .collect();
        g.set_presets(ModelRc::new(VecModel::from(presets)));

        let c = this.clone();
        g.on_go(move |step| c.go(step));
        let c = this.clone();
        g.on_cancel(move || c.cancel());
        let c = this.clone();
        g.on_create(move |start| c.create(start));

        let c = this.clone();
        g.on_set_name(move |name| c.edit(|d| d.name = name.into()));
        let c = this.clone();
        g.on_set_preset(move |index| c.set_preset(index));
        // A preset's script and command are fixed; only a custom agent's can be edited.
        let c = this.clone();
        g.on_set_script(move |script| {
            c.edit(|d| {
                if d.preset == Preset::Custom {
                    d.config.provision_script = script.into();
                }
            })
        });
        let c = this.clone();
        g.on_reset_script(move || {
            c.edit(|d| {
                if d.preset == Preset::Custom {
                    d.config.provision_script = Preset::Custom.provision_script().into();
                }
            });
            c.show_fields();
        });

        let c = this.clone();
        g.on_set_command(move |command| {
            c.edit(|d| {
                if d.preset == Preset::Custom {
                    d.config.command = command.into();
                }
            })
        });
        let c = this.clone();
        g.on_add_argument(move |argument| {
            c.edit(|d| editor::add_argument(&mut d.config, &argument))
        });
        let c = this.clone();
        g.on_remove_argument(move |index| c.edit(|d| remove(&mut d.config.arguments, index)));
        let c = this.clone();
        g.on_edit_argument(move |index, argument| {
            c.edit(|d| editor::edit_argument(&mut d.config, index, &argument))
        });
        let c = this.clone();
        g.on_move_argument(move |from, to| {
            c.edit(|d| editor::move_argument(&mut d.config, from, to))
        });
        let c = this.clone();
        g.on_set_working_directory(move |dir| c.edit(|d| d.config.working_directory = dir.into()));

        let c = this.clone();
        g.on_add_env(move || c.edit(|d| editor::add_env(&mut d.config)));
        let c = this.clone();
        g.on_edit_env(move |index, name, value| {
            c.edit(|d| editor::edit_env(&mut d.config, index, &name, &value))
        });
        let c = this.clone();
        g.on_remove_env(move |index| c.edit(|d| remove(&mut d.config.environment, index)));

        let c = this.clone();
        g.on_add_folder(move || {
            if let Some(folder) = pick_folder() {
                c.edit(|d| editor::add_folder(&mut d.config, folder));
            }
        });
        let c = this.clone();
        g.on_choose_folder(move |index| {
            if let Some(folder) = pick_folder() {
                c.edit(|d| editor::choose_folder(&mut d.config, index, folder));
            }
        });
        let c = this.clone();
        g.on_edit_guest_path(move |index, path| {
            c.edit(|d| editor::edit_guest_path(&mut d.config, index, &path))
        });
        let c = this.clone();
        g.on_set_writable(move |index, writable| {
            c.edit(|d| editor::set_writable(&mut d.config, index, writable))
        });
        let c = this.clone();
        g.on_remove_folder(move |index| c.edit(|d| remove(&mut d.config.mounts, index)));

        let c = this.clone();
        g.on_set_network_deny(move |deny| {
            c.edit(|d| d.config.network_default = editor::action(deny))
        });
        let c = this.clone();
        g.on_set_network_allow_local(move |allow| c.edit(|d| d.config.network_allow_local = allow));
        let c = this.clone();
        g.on_add_rule(move |text, deny| {
            let mut error = "";
            c.edit(|d| error = editor::add_rule(&mut d.config, &text, deny));
            error.into()
        });
        let c = this.clone();
        g.on_toggle_scope(move |deny, index| {
            c.edit(|d| editor::toggle_scope(&mut d.config, deny, index))
        });
        let c = this.clone();
        g.on_remove_rule(move |deny, index| {
            c.edit(|d| editor::remove_rule(&mut d.config, deny, index))
        });
        let c = this.clone();
        g.on_add_forward(move |guest, host| {
            let mut error = "";
            c.edit(|d| error = editor::add_forward(&mut d.config, &guest, &host));
            error.into()
        });
        let c = this.clone();
        g.on_toggle_forward(move |index| {
            c.edit(|d| editor::toggle_forward_scope(&mut d.config, index))
        });
        let c = this.clone();
        g.on_remove_forward(move |index| c.edit(|d| remove(&mut d.config.port_forwards, index)));

        let c = this.clone();
        g.on_set_cpus(move |cpus| c.edit(|d| d.config.cpus = cpus.max(1) as u32));
        let c = this.clone();
        g.on_set_memory(move |gb| c.edit(|d| d.config.memory_mib = gb.max(1) as u32 * 1024));
        let c = this.clone();
        g.on_set_disk(move |gb| {
            c.edit(|d| d.config.disk_gb = (gb.max(0) as u32).max(db::DISK_MIN_GB))
        });

        this
    }

    /// Shows the creation screen: a new draft, or the one in progress.
    pub fn open(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        if self.draft.borrow().is_none() {
            let host = Host::read();
            let preset = Preset::ClaudeCode;
            let mut config = Config {
                command: preset.command().into(),
                arguments: preset.arguments(),
                provision_script: preset.provision_script().into(),
                network_rules: preset.network_rules(),
                port_forwards: preset.port_forwards(),
                ..Config::default()
            };
            // At most half of the Mac, so the Mac stays usable.
            config.cpus = config.cpus.min((host.cpus / 2).max(1));
            config.memory_mib = config.memory_mib.min((host.memory_gb / 2).max(1) * 1024);
            // A random name the user can keep or replace. A taken one is re-rolled a few
            // times; if all are taken, the name validation reports it.
            let mut name = String::new();
            for _ in 0..10 {
                name = petname::petname(2, "-").unwrap_or_default();
                if self.name_error(&name).is_empty() {
                    break;
                }
            }
            *self.draft.borrow_mut() = Some(Draft {
                name,
                preset,
                config,
                custom_command: String::new(),
                custom_script: Preset::Custom.provision_script().into(),
            });

            let g = ui.global::<Creation>();
            g.set_step(HARNESS);
            g.set_reached(HARNESS);
            g.set_create_error("".into());
            g.set_max_cpus(host.cpus as i32);
            g.set_max_memory_gb(host.max_memory_gb() as i32);
            g.set_min_disk_gb(db::DISK_MIN_GB as i32);
            g.set_host_line(host.line().into());
            self.show_fields();
        }
        ui.set_creating(true);
        ui.set_selected_index(self.draft_row().map_or(-1, |row| row as i32));
        self.refresh();
    }

    fn go(&self, step: i32) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let g = ui.global::<Creation>();
        let step = step.clamp(HARNESS, REVIEW);
        g.set_step(step);
        g.set_reached(g.get_reached().max(step));
        self.refresh();
    }

    fn cancel(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        if let Some(row) = self.draft_row() {
            self.instances.remove(row);
        }
        self.draft.borrow_mut().take();
        ui.set_creating(false);
        ui.set_selected_index(0);
    }

    /// Creates the instance and, if `start` is set and nothing keeps it from starting, starts it.
    fn create(&self, start: bool) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let g = ui.global::<Creation>();
        let (name, config, issues) = {
            let draft = self.draft.borrow();
            let Some(d) = draft.as_ref() else {
                return;
            };
            let mut config = d.config.clone();
            // Rows left empty are dropped.
            config
                .environment
                .retain(|var| !var.name.is_empty() || !var.value.is_empty());
            (d.name.trim().to_owned(), config, self.issues(d))
        };
        if issues.iter().any(|issue| issue.blocks_create) {
            return self.refresh();
        }
        let start = start && issues.is_empty();

        let id = uuid::Uuid::now_v7().to_string();
        match self.create_instance(&id, &name, &config) {
            Ok(instance) => {
                if let Some(row) = self.draft_row() {
                    self.instances.remove(row);
                }
                self.draft.borrow_mut().take();
                ui.set_creating(false);
                (self.created)(instance, start);
            }
            Err(error) => {
                g.set_create_error(format!("Couldn't create the instance: {error}").into())
            }
        }
    }

    /// Clones the instance's disk and stores the instance; on failure nothing stays behind.
    fn create_instance(&self, id: &str, name: &str, config: &Config) -> db::Result<db::Instance> {
        let dir = db::instance_dir(id)?;
        let disk = dir.join("root.ext4");
        let created = guest::base_rootfs()
            .and_then(|base| vm::create_disk(&base, &disk))
            .map_err(Box::<dyn Error>::from)
            .and_then(|()| {
                let (kernel, base) = (guest::current_kernel(), guest::current_base());
                self.db
                    .create_instance(id, name, &disk, kernel, base, config)
            });
        if created.is_err() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        created
    }

    fn edit(&self, change: impl FnOnce(&mut Draft)) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            change(draft);
        }
        self.refresh();
    }

    /// Switches the draft to the preset at `index` in `Preset::ALL`. The preset sets the
    /// command and script; its arguments, network rules and forwarded ports replace the old
    /// preset's that the user left alone, so ones the user added or changed stay.
    fn set_preset(&self, index: i32) {
        let Some(&preset) = usize::try_from(index)
            .ok()
            .and_then(|index| Preset::ALL.get(index))
        else {
            return;
        };
        self.edit(|d| {
            let old = d.preset;
            if old == preset {
                return;
            }
            let c = &mut d.config;
            if old == Preset::Custom {
                d.custom_command = std::mem::take(&mut c.command);
                d.custom_script = std::mem::take(&mut c.provision_script);
            }
            if preset == Preset::Custom {
                c.command = d.custom_command.clone();
                c.provision_script = d.custom_script.clone();
            } else {
                c.command = preset.command().into();
                c.provision_script = preset.provision_script().into();
            }
            let old_rules = old.network_rules();
            c.network_rules.retain(|rule| !old_rules.contains(rule));
            for rule in preset.network_rules() {
                let present = c.network_rules.iter().any(|other| {
                    other.action == rule.action && same_target(&other.target, &rule.target)
                });
                if !present {
                    c.network_rules.push(rule);
                }
            }
            if c.arguments == old.arguments() {
                c.arguments = preset.arguments();
            }
            let old_forwards = old.port_forwards();
            c.port_forwards
                .retain(|forward| !old_forwards.contains(forward));
            for forward in preset.port_forwards() {
                let present = c.port_forwards.iter().any(|other| {
                    other.guest_port == forward.guest_port && other.host_port == forward.host_port
                });
                if !present {
                    c.port_forwards.push(forward);
                }
            }
            d.preset = preset;
        });
        self.show_fields();
    }

    /// Puts the draft's single values into their fields, which then keep them while the
    /// user types. Lists and checks follow in `refresh`.
    fn show_fields(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let draft = self.draft.borrow();
        let Some(d) = draft.as_ref() else {
            return;
        };
        let g = ui.global::<Creation>();
        let c = &d.config;
        g.set_name(d.name.as_str().into());
        g.set_script(c.provision_script.as_str().into());
        g.set_command(c.command.as_str().into());
        g.set_working_directory(c.working_directory.as_str().into());
        g.set_cpus(c.cpus as i32);
        g.set_memory_gb((c.memory_mib / 1024) as i32);
        g.set_disk_gb(c.disk_gb as i32);
    }

    /// Shows the draft's lists, checks and read-back, and keeps its sidebar row in sync.
    fn refresh(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let draft = self.draft.borrow();
        let Some(d) = draft.as_ref() else {
            return;
        };
        let g = ui.global::<Creation>();
        let c = &d.config;
        let name = d.name.trim();
        self.sync_draft_row(&ui, name, d.preset.name());

        let custom = d.preset == Preset::Custom;
        let template = c.provision_script == Preset::Custom.provision_script();
        g.set_preset(Preset::ALL.iter().position(|&p| p == d.preset).unwrap_or(0) as i32);
        g.set_is_custom(custom);
        g.set_preset_name(d.preset.name().into());
        g.set_script_changed(custom && !template);
        // An empty name is only reported on Review, not while the user starts typing.
        g.set_name_error(
            if name.is_empty() {
                ""
            } else {
                self.name_error(name)
            }
            .into(),
        );
        g.set_command_error(command_error(&c.command).into());
        g.set_working_directory_error(working_directory_error(&c.working_directory).into());

        g.set_network_deny(c.network_default == NetworkAction::Deny);
        g.set_network_allow_local(c.network_allow_local);
        self.lists.show(c);

        let issues = self.issues(d);
        g.set_can_create(!issues.iter().any(|issue| issue.blocks_create));
        g.set_can_start(issues.is_empty());
        sync(&self.issues, issues);

        g.set_review_runs(command_line(c).into());
        g.set_review_script(match (custom, template) {
            (true, true) => "Template, not filled in yet".into(),
            (true, false) => "Custom script".into(),
            (false, _) => format!("Script from the {} preset", d.preset.name()).into(),
        });
        let targets = |action| {
            let names: Vec<_> = rule_rows(&c.network_rules, action)
                .into_iter()
                .map(|row| row.target)
                .collect();
            list_or_none(&names)
        };
        g.set_review_allowed(targets(NetworkAction::Allow).into());
        g.set_review_denied(targets(NetworkAction::Deny).into());
        let forwards: Vec<_> = c
            .port_forwards
            .iter()
            .map(|forward| forward_label(forward.guest_port, forward.host_port))
            .collect();
        g.set_review_forwards(list_or_none(&forwards).into());
        let names: Vec<_> = c
            .environment
            .iter()
            .filter(|var| !var.name.is_empty())
            .map(|var| var.name.as_str())
            .collect();
        g.set_review_env(list_or_none(&names).into());
    }

    /// What keeps the draft from being started, or created at all.
    fn issues(&self, d: &Draft) -> Vec<CreateIssue> {
        let c = &d.config;
        let mut issues = Vec::new();
        let mut add = |step, text: &str, blocks_create| {
            issues.push(CreateIssue {
                step,
                text: text.into(),
                blocks_create,
            })
        };

        let name_error = self.name_error(&d.name);
        if !name_error.is_empty() {
            add(HARNESS, name_error, true);
        }
        // A preset's script is complete; only a custom agent's needs checking.
        if d.preset == Preset::Custom {
            if c.provision_script.trim().is_empty() {
                add(HARNESS, "The provisioning script is empty.", false);
            } else if c.provision_script == Preset::Custom.provision_script() {
                add(
                    HARNESS,
                    "Fill in the provisioning script so it installs your agent.",
                    false,
                );
            }
        }
        for error in [
            command_error(&c.command),
            working_directory_error(&c.working_directory),
        ] {
            if !error.is_empty() {
                add(AGENT, error, false);
            }
        }
        if env_errors(&c.environment)
            .iter()
            .any(|error| !error.is_empty())
        {
            add(AGENT, "Some environment variables need a fix.", false);
        }
        for (mount, guest_error) in c.mounts.iter().zip(guest_path_errors(&c.mounts)) {
            let path = display_path(&mount.host_path);
            match folder_problem(&mount.host_path) {
                Some(FolderProblem::Missing) => add(
                    FOLDERS,
                    &format!("{path} no longer exists on your Mac."),
                    false,
                ),
                Some(FolderProblem::Unreadable) => {
                    add(FOLDERS, &format!("The app can't read {path}."), false)
                }
                None => {}
            }
            if !guest_error.is_empty() {
                add(
                    FOLDERS,
                    &format!("{}: {guest_error}", mount.guest_path),
                    false,
                );
            }
        }
        issues
    }

    fn name_error(&self, name: &str) -> &'static str {
        // The draft has no id yet.
        editor::name_error(&self.instances, name, "")
    }

    fn draft_row(&self) -> Option<usize> {
        (0..self.instances.row_count()).find(|&row| {
            self.instances
                .row_data(row)
                .is_some_and(|item| item.state == InstanceState::Draft)
        })
    }

    /// The draft shows at the top of the sidebar once it has a name.
    fn sync_draft_row(&self, ui: &AppWindow, name: &str, agent: &str) {
        let selected = ui.get_selected_index();
        match (self.draft_row(), name.is_empty()) {
            (None, false) => {
                self.instances.insert(
                    0,
                    InstanceItem {
                        name: name.into(),
                        agent: agent.into(),
                        state: InstanceState::Draft,
                        summary: "Not created yet".into(),
                        ..Default::default()
                    },
                );
                // The row moves the selection down, or becomes it on the creation screen.
                ui.set_selected_index(if selected < 0 { 0 } else { selected + 1 });
            }
            (Some(row), true) => {
                self.instances.remove(row);
                let row = row as i32;
                if selected == row {
                    ui.set_selected_index(-1);
                } else if selected > row {
                    ui.set_selected_index(selected - 1);
                }
            }
            (Some(row), false) => {
                if let Some(mut item) = self.instances.row_data(row)
                    && (item.name != name || item.agent != agent)
                {
                    item.name = name.into();
                    item.agent = agent.into();
                    self.instances.set_row_data(row, item);
                }
            }
            (None, true) => {}
        }
    }
}

/// The command and its arguments as one line, with arguments that need it quoted.
fn command_line(config: &Config) -> String {
    std::iter::once(&config.command)
        .chain(&config.arguments)
        .map(|part| {
            if part.is_empty() || part.contains(char::is_whitespace) {
                format!("'{part}'")
            } else {
                part.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn list_or_none(items: &[impl AsRef<str>]) -> String {
    if items.is_empty() {
        "None".into()
    } else {
        items
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>()
            .join(", ")
    }
}
