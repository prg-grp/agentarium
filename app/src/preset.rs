//! The presets offered when creating an instance, compiled into the app.
//!
//! A preset decides the agent's command and its default arguments, its provisioning script,
//! the initial network rules and the forwarded ports. It only exists while an instance is
//! drafted: creating the instance copies these values into its first revision, and the
//! preset itself isn't stored.

use crate::db::{NetworkAction, NetworkRule, PortForward, RuleTarget};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Preset {
    ClaudeCode,
    Codex,
    GeminiCli,
    OpenCode,
    OpenClaw,
    /// The user's own provisioning script and command.
    Custom,
}

impl Preset {
    /// In the order the creation screen lists them.
    pub const ALL: [Preset; 6] = [
        Preset::ClaudeCode,
        Preset::Codex,
        Preset::GeminiCli,
        Preset::OpenCode,
        Preset::OpenClaw,
        Preset::Custom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Preset::ClaudeCode => "Claude Code",
            Preset::Codex => "Codex",
            Preset::GeminiCli => "Gemini CLI",
            Preset::OpenCode => "OpenCode",
            Preset::OpenClaw => "OpenClaw",
            Preset::Custom => "Custom agent",
        }
    }

    pub fn note(self) -> &'static str {
        match self {
            Preset::ClaudeCode => "Anthropic's coding agent",
            Preset::Codex => "OpenAI's coding agent",
            Preset::GeminiCli => "Google's coding agent",
            Preset::OpenCode => "Open-source agent; add your model provider's domain",
            Preset::OpenClaw => "Assistant gateway with a web UI; add your model provider's domain",
            Preset::Custom => "Bring your own provisioning script and command",
        }
    }

    /// The program the agent is launched with, without arguments; a custom agent has
    /// none until the user sets it.
    pub fn command(self) -> &'static str {
        match self {
            Preset::ClaudeCode => "claude",
            Preset::Codex => "codex",
            Preset::GeminiCli => "gemini",
            Preset::OpenCode => "opencode",
            Preset::OpenClaw => "openclaw",
            Preset::Custom => "",
        }
    }

    /// The arguments the command starts with; the user may change them.
    pub fn arguments(self) -> Vec<String> {
        let arguments: &[&str] = match self {
            // The gateway in the foreground, so the supervisor keeps it running. It
            // listens on the guest's loopback and serves the Control UI on the same port.
            Preset::OpenClaw => &["gateway", "run", "--port", "18789"],
            _ => &[],
        };
        arguments.iter().map(|&argument| argument.into()).collect()
    }

    /// The guest ports opened on the Mac in a new instance; the user may change them.
    pub fn port_forwards(self) -> Vec<PortForward> {
        let guest_ports: &[u16] = match self {
            Preset::OpenClaw => &[18789],
            _ => &[],
        };
        guest_ports
            .iter()
            .map(|&guest_port| PortForward {
                guest_port,
                host_port: None,
                all_interfaces: false,
            })
            .collect()
    }

    /// Installs the agent; for a custom agent, a template the user fills in.
    pub fn provision_script(self) -> &'static str {
        match self {
            Preset::ClaudeCode => include_str!("../../vms/provision/claude-code.sh"),
            Preset::Codex => include_str!("../../vms/provision/codex.sh"),
            Preset::GeminiCli => include_str!("../../vms/provision/gemini-cli.sh"),
            Preset::OpenCode => include_str!("../../vms/provision/opencode.sh"),
            Preset::OpenClaw => include_str!("../../vms/provision/openclaw.sh"),
            Preset::Custom => include_str!("../../vms/provision/custom.sh"),
        }
    }

    /// The domains the agent needs, allowed in a new instance; the user may change them.
    pub fn network_rules(self) -> Vec<NetworkRule> {
        // (domain, include subdomains)
        let domains: &[(&str, bool)] = match self {
            Preset::ClaudeCode => &[
                ("api.anthropic.com", false),
                ("claude.ai", false),
                ("downloads.claude.ai", false),
                ("platform.claude.com", false),
                ("console.anthropic.com", false),
                ("statsig.anthropic.com", false),
                ("mcp-proxy.anthropic.com", false),
            ],
            Preset::Codex => &[
                ("api.openai.com", false),
                ("auth.openai.com", false),
                ("chatgpt.com", false),
            ],
            Preset::GeminiCli => &[
                ("generativelanguage.googleapis.com", false),
                ("cloudcode-pa.googleapis.com", false),
                ("oauth2.googleapis.com", false),
                ("accounts.google.com", false),
                ("www.googleapis.com", false),
            ],
            Preset::OpenCode => &[("opencode.ai", true), ("models.dev", false)],
            Preset::OpenClaw | Preset::Custom => &[],
        };
        domains
            .iter()
            .map(|&(name, include_subdomains)| NetworkRule {
                target: RuleTarget::Domain {
                    name: name.into(),
                    include_subdomains,
                },
                action: NetworkAction::Allow,
            })
            .collect()
    }
}
