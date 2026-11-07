//! The presets offered when creating an instance, compiled into the app.
//!
//! A preset decides the agent's command, its provisioning script and the initial network
//! rules. It only exists while an instance is drafted: creating the instance copies these
//! values into its first revision, and the preset itself isn't stored.

use crate::db::{NetworkAction, NetworkRule, RuleTarget};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Preset {
    ClaudeCode,
    Codex,
    GeminiCli,
    OpenCode,
    /// The user's own provisioning script and command.
    Custom,
}

impl Preset {
    /// In the order the creation screen lists them.
    pub const ALL: [Preset; 5] = [
        Preset::ClaudeCode,
        Preset::Codex,
        Preset::GeminiCli,
        Preset::OpenCode,
        Preset::Custom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Preset::ClaudeCode => "Claude Code",
            Preset::Codex => "Codex",
            Preset::GeminiCli => "Gemini CLI",
            Preset::OpenCode => "OpenCode",
            Preset::Custom => "Custom agent",
        }
    }

    pub fn note(self) -> &'static str {
        match self {
            Preset::ClaudeCode => "Anthropic's coding agent",
            Preset::Codex => "OpenAI's coding agent",
            Preset::GeminiCli => "Google's coding agent",
            Preset::OpenCode => "Open-source agent; add your model provider's domain",
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
            Preset::Custom => "",
        }
    }

    /// Installs the agent; for a custom agent, a template the user fills in.
    pub fn provision_script(self) -> &'static str {
        match self {
            Preset::ClaudeCode => include_str!("../../vms/provision/claude-code.sh"),
            Preset::Codex => include_str!("../../vms/provision/codex.sh"),
            Preset::GeminiCli => include_str!("../../vms/provision/gemini-cli.sh"),
            Preset::OpenCode => include_str!("../../vms/provision/opencode.sh"),
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
            Preset::Custom => &[],
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
