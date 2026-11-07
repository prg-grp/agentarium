//! Creating an instance: a draft edited in five steps (harness, agent, shared folders,
//! network, review), then created and, unless the user opts out, started right away so it
//! provisions. See design/create-harness.md and the other design/create-*.md files.
//!
//! The draft lives here; the UI's `Creation` global shows it. Every edit goes through a
//! callback, updates the draft and refreshes the lists and checks in the UI.

use std::cell::RefCell;
use std::error::Error;
use std::ffi::{CString, c_void};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::db::{
    self, Config, Db, EnvVar, Mount, NetworkAction, NetworkRule, RuleTarget, mask32, mask128,
    max_prefix, parse_network,
};
use crate::preset::Preset;
use crate::vm;
use crate::{
    AppWindow, CreateIssue, Creation, EnvRow, InstanceItem, InstanceState, MountRow, PresetRow,
    RuleRow,
};

// Steps, as numbered in the step indicator (from 0).
const HARNESS: i32 = 0;
const AGENT: i32 = 1;
const FOLDERS: i32 = 2;
const REVIEW: i32 = 4;

/// The agent user's ~/workspace inside the instance; shared folders go below it by default.
const WORKSPACE: &str = "/home/ubuntu/workspace";

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
    arguments: Rc<VecModel<SharedString>>,
    environment: Rc<VecModel<EnvRow>>,
    mounts: Rc<VecModel<MountRow>>,
    allowed: Rc<VecModel<RuleRow>>,
    denied: Rc<VecModel<RuleRow>>,
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
            arguments: Rc::default(),
            environment: Rc::default(),
            mounts: Rc::default(),
            allowed: Rc::default(),
            denied: Rc::default(),
            issues: Rc::default(),
            created: Box::new(created),
        });

        let g = ui.global::<Creation>();
        g.set_arguments(ModelRc::from(this.arguments.clone()));
        g.set_environment(ModelRc::from(this.environment.clone()));
        g.set_mounts(ModelRc::from(this.mounts.clone()));
        g.set_allowed(ModelRc::from(this.allowed.clone()));
        g.set_denied(ModelRc::from(this.denied.clone()));
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
        g.on_create(move || c.create());

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
            if !argument.is_empty() {
                c.edit(|d| d.config.arguments.push(argument.into()));
            }
        });
        let c = this.clone();
        g.on_remove_argument(move |index| c.edit(|d| remove(&mut d.config.arguments, index)));
        let c = this.clone();
        g.on_set_working_directory(move |dir| c.edit(|d| d.config.working_directory = dir.into()));

        let c = this.clone();
        g.on_add_env(move || {
            c.edit(|d| {
                d.config.environment.push(EnvVar {
                    name: String::new(),
                    value: String::new(),
                })
            })
        });
        let c = this.clone();
        g.on_edit_env(move |index, name, value| {
            c.edit(|d| {
                if let Some(var) = d.config.environment.get_mut(index as usize) {
                    var.name = name.into();
                    var.value = value.into();
                }
            })
        });
        let c = this.clone();
        g.on_remove_env(move |index| c.edit(|d| remove(&mut d.config.environment, index)));

        let c = this.clone();
        g.on_add_folder(move || {
            if let Some(folder) = pick_folder() {
                c.edit(|d| {
                    let guest_path = default_guest_path(&d.config.mounts, &folder);
                    d.config.mounts.push(Mount {
                        host_path: folder,
                        guest_path,
                        writable: false,
                        enabled: true,
                    });
                });
            }
        });
        let c = this.clone();
        g.on_choose_folder(move |index| {
            if let Some(folder) = pick_folder() {
                c.edit(|d| {
                    if let Some(mount) = d.config.mounts.get_mut(index as usize) {
                        mount.host_path = folder;
                    }
                });
            }
        });
        let c = this.clone();
        g.on_edit_guest_path(move |index, path| {
            c.edit(|d| {
                if let Some(mount) = d.config.mounts.get_mut(index as usize) {
                    mount.guest_path = path.into();
                }
            })
        });
        let c = this.clone();
        g.on_set_writable(move |index, writable| {
            c.edit(|d| {
                if let Some(mount) = d.config.mounts.get_mut(index as usize) {
                    mount.writable = writable;
                }
            })
        });
        let c = this.clone();
        g.on_remove_folder(move |index| c.edit(|d| remove(&mut d.config.mounts, index)));

        let c = this.clone();
        g.on_set_network_deny(move |deny| c.edit(|d| d.config.network_default = action(deny)));
        let c = this.clone();
        g.on_add_rule(move |text, deny| c.add_rule(&text, deny).into());
        let c = this.clone();
        g.on_toggle_scope(move |deny, index| {
            c.edit(|d| {
                let rules = &mut d.config.network_rules;
                if let Some(i) = rule_index(rules, action(deny), index)
                    && let RuleTarget::Domain {
                        include_subdomains, ..
                    } = &mut rules[i].target
                {
                    *include_subdomains = !*include_subdomains;
                }
            })
        });
        let c = this.clone();
        g.on_remove_rule(move |deny, index| {
            c.edit(|d| {
                let rules = &mut d.config.network_rules;
                if let Some(i) = rule_index(rules, action(deny), index) {
                    rules.remove(i);
                }
            })
        });

        let c = this.clone();
        g.on_set_cpus(move |cpus| c.edit(|d| d.config.cpus = cpus.max(1) as u32));
        let c = this.clone();
        g.on_set_memory(move |gb| c.edit(|d| d.config.memory_mib = gb.max(1) as u32 * 1024));

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
                provision_script: preset.provision_script().into(),
                network_rules: preset.network_rules(),
                ..Config::default()
            };
            // At most half of the Mac, so the Mac stays usable.
            config.cpus = config.cpus.min((host.cpus / 2).max(1));
            config.memory_mib = config.memory_mib.min((host.memory_gb / 2).max(1) * 1024);
            *self.draft.borrow_mut() = Some(Draft {
                name: String::new(),
                preset,
                config,
                custom_command: String::new(),
                custom_script: Preset::Custom.provision_script().into(),
            });

            let g = ui.global::<Creation>();
            g.set_step(HARNESS);
            g.set_reached(HARNESS);
            g.set_dont_start(false);
            g.set_create_error("".into());
            g.set_max_cpus(host.cpus as i32);
            // Leave the Mac some memory of its own.
            g.set_max_memory_gb(host.memory_gb.saturating_sub(2).max(1) as i32);
            let mut line = format!(
                "Your Mac has {} processors and {} GB of memory.",
                host.cpus, host.memory_gb
            );
            if let Some(free) = host.free_disk_gb {
                line += &format!(" {free} GB of disk is free.");
            }
            g.set_host_line(line.into());
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

    fn create(&self) {
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
        let start = issues.is_empty() && !g.get_dont_start();

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
        let created = vm::create_disk(&disk)
            .map_err(Box::<dyn Error>::from)
            .and_then(|()| self.db.create_instance(id, name, &disk, config));
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
    /// command and script; its network rules replace the old preset's that the user left
    /// alone, so rules the user added or changed stay.
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
            d.preset = preset;
        });
        self.show_fields();
    }

    fn add_rule(&self, text: &str, deny: bool) -> &'static str {
        if text.trim().is_empty() {
            return "";
        }
        let Some(target) = parse_target(text) else {
            return "Enter a domain like pypi.org, an address like 203.0.113.7, or a range like 10.0.0.0/8.";
        };
        let action = action(deny);
        let mut error = "";
        self.edit(|d| {
            let rules = &mut d.config.network_rules;
            if rules
                .iter()
                .any(|rule| rule.action == action && same_target(&rule.target, &target))
            {
                error = "Already in this list.";
            } else {
                rules.push(NetworkRule { target, action });
            }
        });
        error
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

        sync(
            &self.arguments,
            c.arguments.iter().map(|arg| arg.as_str().into()).collect(),
        );
        sync(
            &self.environment,
            c.environment
                .iter()
                .zip(env_errors(&c.environment))
                .map(|(var, error)| EnvRow {
                    name: var.name.as_str().into(),
                    value: var.value.as_str().into(),
                    error: error.into(),
                })
                .collect(),
        );
        sync(
            &self.mounts,
            c.mounts
                .iter()
                .zip(guest_path_errors(&c.mounts))
                .map(|(mount, guest_error)| {
                    let problem = folder_problem(&mount.host_path);
                    MountRow {
                        host_path: display_path(&mount.host_path).into(),
                        guest_path: mount.guest_path.as_str().into(),
                        writable: mount.writable,
                        missing: problem.is_some(),
                        problem: match problem {
                            Some(FolderProblem::Missing) => "This folder no longer exists on your Mac. Choose another folder or remove it.",
                            Some(FolderProblem::Unreadable) => "The app can't read this folder. Allow access in System Settings, or choose another folder.",
                            None => "",
                        }
                        .into(),
                        guest_error: guest_error.into(),
                    }
                })
                .collect(),
        );
        g.set_network_deny(c.network_default == NetworkAction::Deny);
        sync(
            &self.allowed,
            rule_rows(&c.network_rules, NetworkAction::Allow),
        );
        sync(
            &self.denied,
            rule_rows(&c.network_rules, NetworkAction::Deny),
        );

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
        let name = name.trim().to_lowercase();
        if name.is_empty() {
            return "Enter a name for the instance.";
        }
        let taken = self
            .instances
            .iter()
            .any(|item| item.state != InstanceState::Draft && item.name.to_lowercase() == name);
        if taken {
            "Another instance already uses this name."
        } else {
            ""
        }
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

/// What the Mac has, for the resource defaults and limits.
struct Host {
    cpus: u32,
    memory_gb: u32,
    free_disk_gb: Option<u64>,
}

impl Host {
    fn read() -> Host {
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get() as u32);
        let mut memory: u64 = 0;
        let mut size = size_of::<u64>();
        let read = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&raw mut memory).cast::<c_void>(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        // Disk space as Finder counts it; it holds the instance disks.
        let free_disk_gb = db::app_dir().ok().and_then(|dir| {
            let path = CString::new(dir.as_os_str().as_bytes()).ok()?;
            let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
            (unsafe { libc::statfs(path.as_ptr(), &mut stat) } == 0)
                .then(|| stat.f_bavail * u64::from(stat.f_bsize) / 1_000_000_000)
        });
        Host {
            cpus,
            memory_gb: if read { (memory >> 30) as u32 } else { 8 },
            free_disk_gb,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum FolderProblem {
    Missing,
    Unreadable,
}

pub(crate) fn folder_problem(path: &Path) -> Option<FolderProblem> {
    match std::fs::read_dir(path) {
        Ok(_) => None,
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Some(FolderProblem::Missing)
        }
        Err(_) => Some(FolderProblem::Unreadable),
    }
}

fn command_error(command: &str) -> &'static str {
    if command.trim().is_empty() {
        "Enter the program to start, like claude."
    } else {
        ""
    }
}

fn working_directory_error(dir: &str) -> &'static str {
    if dir.starts_with('/') {
        ""
    } else {
        "Enter an absolute path inside the instance, like /home/ubuntu/workspace."
    }
}

pub(crate) fn env_errors(vars: &[EnvVar]) -> Vec<&'static str> {
    vars.iter()
        .enumerate()
        .map(|(i, var)| {
            if var.name.is_empty() {
                if var.value.is_empty() {
                    ""
                } else {
                    "Enter a name for this value."
                }
            } else if var
                .name
                .contains(|ch: char| ch == '=' || ch.is_whitespace())
            {
                "Names can't contain = or spaces."
            } else if vars[..i].iter().any(|other| other.name == var.name) {
                "This name is already used above."
            } else {
                ""
            }
        })
        .collect()
}

pub(crate) fn guest_path_errors(mounts: &[Mount]) -> Vec<&'static str> {
    let normalized = |path: &str| path.trim_end_matches('/').to_owned();
    mounts
        .iter()
        .enumerate()
        .map(|(i, mount)| {
            let path = normalized(&mount.guest_path);
            if !mount.guest_path.starts_with('/')
                || path.is_empty()
                || path.split('/').any(|part| part == "..")
            {
                "Enter an absolute path inside the instance, like /home/ubuntu/workspace/app."
            } else if mounts[..i]
                .iter()
                .any(|other| normalized(&other.guest_path) == path)
            {
                "Another folder already uses this path inside the instance."
            } else {
                ""
            }
        })
        .collect()
}

/// `~/workspace/<folder name>` inside the instance, numbered if another folder has it.
fn default_guest_path(mounts: &[Mount], folder: &Path) -> String {
    let name = folder
        .file_name()
        .map_or("folder".into(), |name| name.to_string_lossy());
    let base = format!("{WORKSPACE}/{name}");
    (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|path| !mounts.iter().any(|mount| &mount.guest_path == path))
        .unwrap_or(base)
}

fn pick_folder() -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title("Choose a folder to share");
    if let Some(home) = std::env::var_os("HOME") {
        dialog = dialog.set_directory(home);
    }
    dialog.pick_folder()
}

/// A host path with the home folder shortened to `~`.
pub(crate) fn display_path(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return Path::new("~").join(rest).display().to_string();
    }
    path.display().to_string()
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

fn action(deny: bool) -> NetworkAction {
    if deny {
        NetworkAction::Deny
    } else {
        NetworkAction::Allow
    }
}

/// The position in `rules` of the `index`th rule with `action`, as the lists show them.
fn rule_index(rules: &[NetworkRule], action: NetworkAction, index: i32) -> Option<usize> {
    rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule.action == action)
        .nth(usize::try_from(index).ok()?)
        .map(|(i, _)| i)
}

fn rule_rows(rules: &[NetworkRule], action: NetworkAction) -> Vec<RuleRow> {
    let denied = |target: &RuleTarget| {
        rules
            .iter()
            .any(|other| other.action == NetworkAction::Deny && other.target.covers(target))
    };
    rules
        .iter()
        .filter(|rule| rule.action == action)
        .map(|rule| {
            let conflict = action == NetworkAction::Allow && denied(&rule.target);
            match &rule.target {
                RuleTarget::Domain {
                    name,
                    include_subdomains,
                } => RuleRow {
                    target: name.into(),
                    scope: if *include_subdomains {
                        "Includes subdomains"
                    } else {
                        "Exact name"
                    }
                    .into(),
                    is_domain: true,
                    conflict: if conflict {
                        "Also denied. Deny rules win, so this domain stays blocked."
                    } else {
                        ""
                    }
                    .into(),
                },
                RuleTarget::Cidr { network } => RuleRow {
                    target: network.into(),
                    scope: match parse_network(network) {
                        Some((address, prefix)) if prefix == max_prefix(address) => {
                            "Single address"
                        }
                        _ => "Address range",
                    }
                    .into(),
                    is_domain: false,
                    conflict: if conflict {
                        "Overlaps a denied range. Deny rules win where they overlap."
                    } else {
                        ""
                    }
                    .into(),
                },
            }
        })
        .collect()
}

/// Parses a rule target: a domain, an IP address, or a CIDR range. Ranges are normalized
/// to their network address; a single address becomes a /32 or /128.
fn parse_target(text: &str) -> Option<RuleTarget> {
    let text = text.trim().to_ascii_lowercase();
    let text = text.strip_suffix('.').unwrap_or(&text);
    if let Some((address, prefix)) = text.split_once('/') {
        return cidr(address.parse().ok()?, prefix.parse().ok()?);
    }
    if let Ok(address) = text.parse::<IpAddr>() {
        return cidr(address, max_prefix(address));
    }
    let labels: Vec<&str> = text.split('.').collect();
    let valid_label = |label: &&str| {
        (1..=63).contains(&label.len())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    // The top-level label is never all digits, so 1.2.3 isn't taken for a domain.
    let valid = text.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(valid_label)
        && labels
            .last()
            .is_some_and(|tld| !tld.bytes().all(|b| b.is_ascii_digit()));
    valid.then(|| RuleTarget::Domain {
        name: text.into(),
        include_subdomains: false,
    })
}

fn cidr(address: IpAddr, prefix: u8) -> Option<RuleTarget> {
    let network = match address {
        IpAddr::V4(address) if prefix <= 32 => {
            IpAddr::V4(Ipv4Addr::from(u32::from(address) & mask32(prefix)))
        }
        IpAddr::V6(address) if prefix <= 128 => {
            IpAddr::V6(Ipv6Addr::from(u128::from(address) & mask128(prefix)))
        }
        _ => return None,
    };
    Some(RuleTarget::Cidr {
        network: format!("{network}/{prefix}"),
    })
}

fn same_target(a: &RuleTarget, b: &RuleTarget) -> bool {
    match (a, b) {
        (RuleTarget::Domain { name: a, .. }, RuleTarget::Domain { name: b, .. }) => a == b,
        (RuleTarget::Cidr { network: a }, RuleTarget::Cidr { network: b }) => a == b,
        _ => false,
    }
}

fn remove<T>(list: &mut Vec<T>, index: i32) {
    if let Ok(index) = usize::try_from(index)
        && index < list.len()
    {
        list.remove(index);
    }
}

/// Updates a UI list in place, so rows (and fields being typed in) stay put; only a change
/// in length replaces the rows.
fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (index, row) in rows.into_iter().enumerate() {
        if model.row_data(index).as_ref() != Some(&row) {
            model.set_row_data(index, row);
        }
    }
}
