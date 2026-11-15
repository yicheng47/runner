use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(inline))]
pub enum Runtime {
    Codex,
    ClaudeCode,
    Antigravity,
    Pi,
    Copilot,
    Trae,
    Cursor,
    Grok,
    #[serde(rename = "opencode")]
    OpenCode,
    Shell,
}

impl Runtime {
    pub const ALL: [Self; 10] = [
        Self::Codex,
        Self::ClaudeCode,
        Self::Antigravity,
        Self::Pi,
        Self::Copilot,
        Self::Trae,
        Self::Cursor,
        Self::Grok,
        Self::OpenCode,
        Self::Shell,
    ];

    /// Priority order used in onboarding and starter-crew ranking.
    pub const SETUP_ORDER: [Self; 9] = [
        Self::ClaudeCode,
        Self::Codex,
        Self::Antigravity,
        Self::Pi,
        Self::Copilot,
        Self::Cursor,
        Self::OpenCode,
        Self::Grok,
        Self::Trae,
    ];

    pub fn setup_rank(self) -> usize {
        Self::SETUP_ORDER
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or(Self::SETUP_ORDER.len())
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Trae => "trae",
            Self::Copilot => "copilot",
            Self::Pi => "pi",
            Self::Antigravity => "antigravity",
            Self::Cursor => "cursor",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
            Self::Shell => "shell",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
            Self::Antigravity => "Antigravity CLI",
            Self::Pi => "pi",
            Self::Copilot => "GitHub Copilot CLI",
            Self::Trae => "TRAE CLI",
            Self::Cursor => "Cursor",
            Self::Grok => "Grok",
            Self::OpenCode => "OpenCode v2",
            Self::Shell => "Shell",
        }
    }
    pub fn command(self) -> Option<&'static str> {
        match self {
            Self::Codex => Some("codex"),
            Self::ClaudeCode => Some("claude"),
            Self::Antigravity => Some("agy"),
            Self::Pi => Some("pi"),
            Self::Copilot => Some("copilot"),
            Self::Trae => Some("traecli"),
            Self::Cursor => Some("cursor-agent"),
            Self::Grok => Some("grok"),
            Self::OpenCode => Some("opencode"),
            Self::Shell => None,
        }
    }
    pub fn managed_skill_root(self) -> Option<&'static str> {
        match self {
            Self::ClaudeCode => Some(".claude/skills"),
            Self::Codex | Self::Pi | Self::Copilot | Self::Cursor | Self::Grok | Self::OpenCode => {
                Some(".agents/skills")
            }
            Self::Trae => Some(".trae/skills"),
            Self::Antigravity => Some(".gemini/antigravity-cli/skills"),
            Self::Shell => None,
        }
    }
    pub fn is_shell(self) -> bool {
        self == Self::Shell
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|runtime| runtime.key() == value)
    }
}

impl std::fmt::Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.key())
    }
}

#[cfg(test)]
mod tests {
    use super::Runtime;

    #[test]
    fn runtime_wire_names_round_trip() {
        for (runtime, key) in [
            (Runtime::ClaudeCode, "claude-code"),
            (Runtime::Codex, "codex"),
            (Runtime::Trae, "trae"),
            (Runtime::Copilot, "copilot"),
            (Runtime::Pi, "pi"),
            (Runtime::Antigravity, "antigravity"),
            (Runtime::Cursor, "cursor"),
            (Runtime::Grok, "grok"),
            (Runtime::OpenCode, "opencode"),
            (Runtime::Shell, "shell"),
        ] {
            let json = format!("\"{key}\"");
            assert_eq!(serde_json::to_string(&runtime).unwrap(), json);
            assert_eq!(serde_json::from_str::<Runtime>(&json).unwrap(), runtime);
            assert_eq!(Runtime::parse(key), Some(runtime));
            assert_eq!(runtime.key(), key);
            assert_eq!(runtime.to_string(), key);
        }
        let roots: std::collections::BTreeSet<_> = Runtime::ALL
            .into_iter()
            .filter_map(Runtime::managed_skill_root)
            .collect();
        assert_eq!(roots, crate::RUNNER_SKILL_ROOTS.iter().copied().collect());
        assert_eq!(roots.len(), crate::RUNNER_SKILL_ROOTS.len());
        assert_eq!(Runtime::parse("aider-future"), None);
        assert!(serde_json::from_str::<Runtime>("\"aider-future\"").is_err());
    }

    #[test]
    fn opencode_v2_display_and_catalog_metadata_keep_wire_ids_intact() {
        assert_eq!(Runtime::OpenCode.display_name(), "OpenCode v2");
        assert_eq!(Runtime::OpenCode.key(), "opencode");
        let catalog = crate::protocol::runtime_metadata::opencode::catalog().unwrap();
        assert_eq!(catalog.display_name, "OpenCode v2");
        assert_eq!(catalog.min_version, Some("2.0.0"));
        assert_eq!(catalog.install_url, "https://opencode.ai/v2/docs");
        assert!(catalog.update_args.is_empty());
        let client = crate::protocol::McpClientId::OpenCode;
        assert_eq!(client.key(), "opencode");
        assert_eq!(client.label(), "OpenCode v2");
        assert_eq!(serde_json::to_string(&client).unwrap(), "\"OpenCode\"");
    }
}
