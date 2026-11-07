//! Editing an instance's configuration, shared by the creation flow (`create.rs`) and the
//! Settings tab (`settings.rs`): the lists the UI shows, the checks, the edits that are more
//! than an assignment, and what the Mac has for the resource limits.

use std::ffi::{CString, c_void};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{Model, SharedString, VecModel};

use crate::db::{
    self, Config, EnvVar, Mount, NetworkAction, NetworkRule, RuleTarget, mask32, mask128,
    max_prefix, parse_network,
};
use crate::{EnvRow, InstanceItem, InstanceState, MountRow, RuleRow};

/// The agent user's ~/workspace inside the instance; shared folders go below it by default.
const WORKSPACE: &str = "/home/ubuntu/workspace";

/// The configuration's lists as the UI shows them, with their checks.
#[derive(Default)]
pub struct Lists {
    pub arguments: Rc<VecModel<SharedString>>,
    pub environment: Rc<VecModel<EnvRow>>,
    pub mounts: Rc<VecModel<MountRow>>,
    pub allowed: Rc<VecModel<RuleRow>>,
    pub denied: Rc<VecModel<RuleRow>>,
}

impl Lists {
    pub fn show(&self, c: &Config) {
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
        sync(
            &self.allowed,
            rule_rows(&c.network_rules, NetworkAction::Allow),
        );
        sync(
            &self.denied,
            rule_rows(&c.network_rules, NetworkAction::Deny),
        );
    }
}

pub fn add_argument(c: &mut Config, argument: &str) {
    if !argument.is_empty() {
        c.arguments.push(argument.into());
    }
}

pub fn add_env(c: &mut Config) {
    c.environment.push(EnvVar {
        name: String::new(),
        value: String::new(),
    });
}

pub fn edit_env(c: &mut Config, index: i32, name: &str, value: &str) {
    if let Some(var) = c.environment.get_mut(index as usize) {
        var.name = name.into();
        var.value = value.into();
    }
}

/// Shares `folder` read-only below ~/workspace.
pub fn add_folder(c: &mut Config, folder: PathBuf) {
    let guest_path = default_guest_path(&c.mounts, &folder);
    c.mounts.push(Mount {
        host_path: folder,
        guest_path,
        writable: false,
        enabled: true,
    });
}

pub fn choose_folder(c: &mut Config, index: i32, folder: PathBuf) {
    if let Some(mount) = c.mounts.get_mut(index as usize) {
        mount.host_path = folder;
    }
}

pub fn edit_guest_path(c: &mut Config, index: i32, path: &str) {
    if let Some(mount) = c.mounts.get_mut(index as usize) {
        mount.guest_path = path.into();
    }
}

pub fn set_writable(c: &mut Config, index: i32, writable: bool) {
    if let Some(mount) = c.mounts.get_mut(index as usize) {
        mount.writable = writable;
    }
}

/// Adds a rule from what the user typed; returns why it was refused, or "".
pub fn add_rule(c: &mut Config, text: &str, deny: bool) -> &'static str {
    if text.trim().is_empty() {
        return "";
    }
    let Some(target) = parse_target(text) else {
        return "Enter a domain like pypi.org, an address like 203.0.113.7, or a range like 10.0.0.0/8.";
    };
    let action = action(deny);
    let rules = &mut c.network_rules;
    if rules
        .iter()
        .any(|rule| rule.action == action && same_target(&rule.target, &target))
    {
        return "Already in this list.";
    }
    rules.push(NetworkRule { target, action });
    ""
}

pub fn toggle_scope(c: &mut Config, deny: bool, index: i32) {
    let rules = &mut c.network_rules;
    if let Some(i) = rule_index(rules, action(deny), index)
        && let RuleTarget::Domain {
            include_subdomains, ..
        } = &mut rules[i].target
    {
        *include_subdomains = !*include_subdomains;
    }
}

pub fn remove_rule(c: &mut Config, deny: bool, index: i32) {
    let rules = &mut c.network_rules;
    if let Some(i) = rule_index(rules, action(deny), index) {
        rules.remove(i);
    }
}

pub fn remove<T>(list: &mut Vec<T>, index: i32) {
    if let Ok(index) = usize::try_from(index)
        && index < list.len()
    {
        list.remove(index);
    }
}

pub fn action(deny: bool) -> NetworkAction {
    if deny {
        NetworkAction::Deny
    } else {
        NetworkAction::Allow
    }
}

/// Why `name` can't name an instance, or "". `except_id` is the instance being renamed.
pub fn name_error(instances: &VecModel<InstanceItem>, name: &str, except_id: &str) -> &'static str {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return "Enter a name for the instance.";
    }
    let taken = instances.iter().any(|item| {
        item.state != InstanceState::Draft
            && item.id != except_id
            && item.name.to_lowercase() == name
    });
    if taken {
        "Another instance already uses this name."
    } else {
        ""
    }
}

/// What the Mac has, for the resource defaults and limits.
pub struct Host {
    pub cpus: u32,
    pub memory_gb: u32,
    free_disk_gb: Option<u64>,
}

impl Host {
    pub fn read() -> Host {
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

    /// The most memory an instance may get; the Mac keeps some of its own.
    pub fn max_memory_gb(&self) -> u32 {
        self.memory_gb.saturating_sub(2).max(1)
    }

    /// The sentence under the resource fields.
    pub fn line(&self) -> String {
        let mut line = format!(
            "Your Mac has {} processors and {} GB of memory.",
            self.cpus, self.memory_gb
        );
        if let Some(free) = self.free_disk_gb {
            line += &format!(" {free} GB of disk is free.");
        }
        line
    }
}

#[derive(Clone, Copy)]
pub enum FolderProblem {
    Missing,
    Unreadable,
}

pub fn folder_problem(path: &Path) -> Option<FolderProblem> {
    match std::fs::read_dir(path) {
        Ok(_) => None,
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Some(FolderProblem::Missing)
        }
        Err(_) => Some(FolderProblem::Unreadable),
    }
}

pub fn command_error(command: &str) -> &'static str {
    if command.trim().is_empty() {
        "Enter the program to start, like claude."
    } else {
        ""
    }
}

pub fn working_directory_error(dir: &str) -> &'static str {
    if dir.starts_with('/') {
        ""
    } else {
        "Enter an absolute path inside the instance, like /home/ubuntu/workspace."
    }
}

pub fn env_errors(vars: &[EnvVar]) -> Vec<&'static str> {
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

pub fn guest_path_errors(mounts: &[Mount]) -> Vec<&'static str> {
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

pub fn pick_folder() -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title("Choose a folder to share");
    if let Some(home) = std::env::var_os("HOME") {
        dialog = dialog.set_directory(home);
    }
    dialog.pick_folder()
}

/// A host path with the home folder shortened to `~`.
pub fn display_path(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return Path::new("~").join(rest).display().to_string();
    }
    path.display().to_string()
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

pub fn rule_rows(rules: &[NetworkRule], action: NetworkAction) -> Vec<RuleRow> {
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

pub fn same_target(a: &RuleTarget, b: &RuleTarget) -> bool {
    match (a, b) {
        (RuleTarget::Domain { name: a, .. }, RuleTarget::Domain { name: b, .. }) => a == b,
        (RuleTarget::Cidr { network: a }, RuleTarget::Cidr { network: b }) => a == b,
        _ => false,
    }
}

/// Updates a UI list in place, so rows (and fields being typed in) stay put; only a change
/// in length replaces the rows.
pub fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
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
