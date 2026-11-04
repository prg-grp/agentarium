//! Temporary in-memory data until real storage exists.
//!
//! The app starts with no instances. Set `AGENT_TOOLBOX_MOCK=sample` to load the
//! sample instances from the design mockups.

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
}

const fn reading(value: &'static str, tone: Tone) -> Reading {
    Reading { value, tone }
}

pub fn instances() -> Vec<Instance> {
    if std::env::var("AGENT_TOOLBOX_MOCK").as_deref() != Ok("sample") {
        return Vec::new();
    }

    vec![
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
        },
    ]
}
