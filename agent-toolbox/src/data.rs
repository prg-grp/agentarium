//! Temporary in-memory data until real storage exists.
//!
//! The app starts with one predefined instance, the demo VM. Set
//! `AGENT_TOOLBOX_MOCK=sample` to add the sample instances from the design
//! mockups, or `AGENT_TOOLBOX_MOCK=empty` to start without instances.

pub enum InstanceState {
    Running,
    Maintenance,
    StartupFailed,
    Stopped,
}

pub enum Tone {
    Ok,
    Bad,
    Neutral,
}

pub struct Reading {
    pub value: &'static str,
    pub tone: Tone,
}

pub struct Instance {
    pub id: &'static str,
    pub name: &'static str,
    pub agent: &'static str,
    pub state: InstanceState,
    pub summary: &'static str,
    pub vm: Reading,
    pub agent_status: Reading,
    pub monitoring: Reading,
    pub pending_changes: i32,
    pub has_terminal: bool,
}

pub const DEMO_VM_ID: &str = "demo-vm";

/// A statically defined VM, built by `make guest`.
pub struct VmSpec {
    pub cpus: usize,
    pub memory_mib: u64,
    pub kernel: &'static str,
    /// Raw ext4 image, attached read-write as the guest's /dev/vda.
    pub rootfs: &'static str,
    pub command_line: &'static str,
}

/// In agent mode the guest console logs in `ubuntu` and runs Claude Code; maintenance
/// mode logs in root without the agent. The guest reads the mode from the kernel flag.
pub fn demo_vm(maintenance: bool) -> VmSpec {
    VmSpec {
        cpus: 2,
        memory_mib: 2048,
        // The guest is built into the workspace target directory, one level above this crate.
        kernel: concat!(env!("CARGO_MANIFEST_DIR"), "/../target/guest/Image"),
        rootfs: concat!(env!("CARGO_MANIFEST_DIR"), "/../target/rootfs/claude.ext4"),
        command_line: if maintenance {
            "console=hvc0 root=/dev/vda rw agent_toolbox.mode=maintenance"
        } else {
            "console=hvc0 root=/dev/vda rw"
        },
    }
}

const fn reading(value: &'static str, tone: Tone) -> Reading {
    Reading { value, tone }
}

pub fn instances() -> Vec<Instance> {
    let mock = std::env::var("AGENT_TOOLBOX_MOCK").unwrap_or_default();
    if mock == "empty" {
        return Vec::new();
    }

    let mut instances = vec![Instance {
        id: DEMO_VM_ID,
        name: DEMO_VM_ID,
        agent: "Claude Code",
        state: InstanceState::Stopped,
        summary: "Stopped",
        vm: reading("Stopped", Tone::Neutral),
        agent_status: reading("Not running", Tone::Neutral),
        monitoring: reading("Not running", Tone::Neutral),
        pending_changes: 0,
        has_terminal: true,
    }];
    if mock != "sample" {
        return instances;
    }

    instances.extend([
        Instance {
            id: "api-refactor",
            name: "api-refactor",
            agent: "Claude Code",
            state: InstanceState::Running,
            summary: "Running",
            vm: reading("Running", Tone::Ok),
            agent_status: reading("Running", Tone::Ok),
            monitoring: reading("Running", Tone::Ok),
            pending_changes: 2,
            has_terminal: false,
        },
        Instance {
            id: "scraper-lab",
            name: "scraper-lab",
            agent: "Claude Code",
            state: InstanceState::Maintenance,
            summary: "Maintenance, internet open",
            vm: reading("Running", Tone::Ok),
            agent_status: reading("Not running in maintenance", Tone::Neutral),
            monitoring: reading("Running", Tone::Ok),
            pending_changes: 0,
            has_terminal: false,
        },
        Instance {
            id: "infra-audit",
            name: "infra-audit",
            agent: "Claude Code",
            state: InstanceState::StartupFailed,
            summary: "Startup failed",
            vm: reading("Running", Tone::Ok),
            agent_status: reading("Not started", Tone::Neutral),
            monitoring: reading("Failed to start", Tone::Bad),
            pending_changes: 0,
            has_terminal: false,
        },
        Instance {
            id: "docs-site",
            name: "docs-site",
            agent: "Claude Code",
            state: InstanceState::Stopped,
            summary: "Stopped 2 days ago",
            vm: reading("Stopped", Tone::Neutral),
            agent_status: reading("Not running", Tone::Neutral),
            monitoring: reading("Not running", Tone::Neutral),
            pending_changes: 0,
            has_terminal: false,
        },
    ]);
    instances
}
