use super::*;
pub struct Permissions {
    pub offered: &'static [PermissionMode],
    pub strip_flags: &'static [(&'static str, bool)],
    pub equals_on_bool: bool,
    pub variadic_flag: Option<&'static str>,
    pub args: fn(PermissionMode) -> Vec<String>,
    pub matches: fn(&[String], PermissionMode) -> bool,
    pub mission_bypass: Option<&'static [&'static str]>,
    pub strip_on_mission_resume: bool,
}
impl Permissions {
    pub fn mode_args(&self, mode: PermissionMode) -> Vec<String> {
        (self.args)(mode)
    }
    pub fn strip(&self, args: &[String]) -> Vec<String> {
        let keys = self.strip_flags;
        // Go's flag package also takes `=value` on a boolean flag.
        let equals_on_bool = self.equals_on_bool;
        if keys.is_empty() {
            return args.to_vec();
        }
        let mut out = Vec::with_capacity(args.len());
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if self.variadic_flag == Some(arg.as_str()) {
                i += 1;
                while i < args.len() && !args[i].starts_with('-') {
                    i += 1;
                }
                continue;
            }
            // Exact-match `--flag` form. For takes_value flags we also
            // skip the next token if present (it's the value).
            if let Some(&(_, takes_value)) = keys.iter().find(|(name, _)| name == arg) {
                i += if takes_value && i + 1 < args.len() {
                    2
                } else {
                    1
                };
                continue;
            }
            // `--flag=value` form: strip the whole token in one go.
            if keys.iter().any(|(name, takes_value)| {
                (*takes_value || equals_on_bool) && arg.starts_with(&format!("{name}="))
            }) {
                i += 1;
                continue;
            }
            out.push(arg.clone());
            i += 1;
        }
        out
    }
    pub fn apply(&self, args: &[String], mode: PermissionMode) -> Vec<String> {
        let mut out = self.strip(args);
        out.extend(self.mode_args(mode));
        out
    }
    pub fn mission_args(&self) -> Vec<String> {
        self.mission_bypass
            .map(strings)
            .unwrap_or_else(|| self.mode_args(PermissionMode::Bypass))
    }
    pub fn apply_mission(&self, args: &[String]) -> Vec<String> {
        let mut out = self.strip(args);
        out.extend(self.mission_args());
        out
    }
    pub fn infer(&self, args: &[String]) -> PermissionMode {
        [
            PermissionMode::Bypass,
            PermissionMode::Auto,
            PermissionMode::AcceptEdits,
        ]
        .into_iter()
        .find(|mode| (self.matches)(args, *mode))
        .unwrap_or(PermissionMode::Default)
    }
}
pub fn go_bool_flag_is_set(arg: &str, name: &str) -> bool {
    let Some(rest) = arg.strip_prefix("--").or_else(|| arg.strip_prefix('-')) else {
        return false;
    };
    match rest.strip_prefix(name) {
        Some("") => true,
        Some(value) => value
            .strip_prefix('=')
            .is_some_and(|value| matches!(value, "1" | "t" | "T" | "true" | "TRUE" | "True")),
        None => false,
    }
}
pub fn flag_value_matches(args: &[String], flag: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return args.iter().any(|a| a == flag);
    };
    let equals_token = format!("{flag}={expected}");
    for (i, arg) in args.iter().enumerate() {
        if arg == &equals_token {
            return true;
        }
        if arg == flag {
            if let Some(next) = args.get(i + 1) {
                if next == expected {
                    return true;
                }
            }
        }
    }
    false
}
pub fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).into()).collect()
}
pub mod codex {
    use super::super::runtime_options::*;
    use super::*;
    pub static PERMISSIONS: Permissions = Permissions {
        offered: &[
            PermissionMode::Default,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ],
        strip_flags: &[("--ask-for-approval", true), ("--sandbox", true)],
        equals_on_bool: false,
        variadic_flag: None,
        args: permission_args,
        matches: mode_matches,
        mission_bypass: Some(&[
            "--ask-for-approval",
            "never",
            "--sandbox",
            "danger-full-access",
        ]),
        strip_on_mission_resume: false,
    };
    fn permission_args(mode: PermissionMode) -> Vec<String> {
        match mode {
            PermissionMode::Auto => strings(&[
                "--ask-for-approval",
                "on-request",
                "--sandbox",
                "workspace-write",
            ]),
            PermissionMode::Bypass => strings(&[
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write",
            ]),
            _ => Vec::new(),
        }
    }
    fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
        let pairs: &[(&str, Option<&str>)] = match mode {
            PermissionMode::Auto => &[
                ("--ask-for-approval", Some("on-request")),
                ("--sandbox", Some("workspace-write")),
            ],
            PermissionMode::Bypass => &[
                ("--ask-for-approval", Some("never")),
                ("--sandbox", Some("workspace-write")),
            ],
            _ => &[],
        };
        !pairs.is_empty()
            && pairs
                .iter()
                .all(|&(flag, expected)| flag_value_matches(args, flag, expected))
    }
    pub fn capabilities() -> RuntimeCapabilities {
        RuntimeCapabilities {
            usage: true,
            global_skill_toggle: true,
            skill_toggle_requires_marker: true,
            codex_speed: true,
            ..Default::default()
        }
    }
    pub fn catalog() -> Option<RuntimeCatalog> {
        let mut codex_efforts = common_efforts();
        codex_efforts.push(option(
            "max",
            "Max",
            "Maximum reasoning depth for the hardest problems.",
        ));
        codex_efforts.push(option(
            "ultra",
            "Ultra",
            "Maximum reasoning with automatic task delegation.",
        ));
        Some(RuntimeCatalog {
            name: Runtime::Codex,
            min_version: Some("0.149.0"),
            display_name: Runtime::Codex.display_name(),
            command: Runtime::Codex.command().unwrap(),
            capabilities: capabilities(),
            native_fork: true,
            description: "OpenAI Codex CLI",
            install_url: "https://developers.openai.com/codex/cli",
            default_enabled: true,
            models: vec![
                default_model_option(),
                option(
                    "gpt-6-astra",
                    "gpt-6-astra",
                    "Our most capable model for complex, demanding work.",
                ),
                option(
                    "gpt-5.6-sol",
                    "gpt-5.6-sol",
                    "Reliable agentic workhorse for everyday tasks.",
                ),
                option(
                    "gpt-5.6-terra",
                    "gpt-5.6-terra",
                    "Balanced agentic coding model for everyday work.",
                ),
                option(
                    "gpt-5.6-luna",
                    "gpt-5.6-luna",
                    "Fast and affordable agentic coding model.",
                ),
                option(
                    "gpt-5.5",
                    "gpt-5.5",
                    "Frontier model for complex coding, research, and real-world work.",
                ),
                option("gpt-5.4", "gpt-5.4", "Strong model for everyday coding."),
                option(
                    "gpt-5.4-mini",
                    "gpt-5.4-mini",
                    "Small, fast, and cost-efficient model for simpler coding tasks.",
                ),
                option(
                    "gpt-5.3-codex-spark",
                    "gpt-5.3-codex-spark",
                    "Ultra-fast coding model.",
                ),
            ],
            efforts: codex_efforts,
            skills_dirs: SKILL_DIRS,
            update_args: &["update"],
            npm_package: Some("@openai/codex"),
        })
    }
    pub const SKILL_DIRS: &[&str] = &[".agents/skills", ".codex/skills"];
}
pub mod claude_code {
    use super::super::runtime_options::*;
    use super::*;
    pub static PERMISSIONS: Permissions = Permissions {
        offered: &[
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ],
        strip_flags: &[
            ("--dangerously-skip-permissions", false),
            ("--permission-mode", true),
        ],
        equals_on_bool: false,
        variadic_flag: None,
        args: permission_args,
        matches: mode_matches,
        mission_bypass: None,
        strip_on_mission_resume: false,
    };
    fn permission_args(mode: PermissionMode) -> Vec<String> {
        match mode {
            PermissionMode::AcceptEdits => strings(&["--permission-mode", "acceptEdits"]),
            PermissionMode::Auto => strings(&["--permission-mode", "auto"]),
            PermissionMode::Bypass => strings(&["--permission-mode", "bypassPermissions"]),
            _ => Vec::new(),
        }
    }
    fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
        if mode == PermissionMode::Bypass
            && args.iter().any(|a| a == "--dangerously-skip-permissions")
        {
            return true;
        }
        let pairs: &[(&str, Option<&str>)] = match mode {
            PermissionMode::AcceptEdits => &[("--permission-mode", Some("acceptEdits"))],
            PermissionMode::Auto => &[("--permission-mode", Some("auto"))],
            PermissionMode::Bypass => &[("--permission-mode", Some("bypassPermissions"))],
            _ => &[],
        };
        !pairs.is_empty()
            && pairs
                .iter()
                .all(|&(flag, expected)| flag_value_matches(args, flag, expected))
    }
    pub fn capabilities() -> RuntimeCapabilities {
        RuntimeCapabilities {
            usage: true,
            global_skill_toggle: true,
            ..Default::default()
        }
    }
    pub fn catalog() -> Option<RuntimeCatalog> {
        let claude_efforts = vec![
            default_effort(),
            plain_option("low", "low"),
            plain_option("medium", "medium"),
            plain_option("high", "high"),
            plain_option("xhigh", "xhigh"),
            plain_option("max", "max"),
        ];
        Some(RuntimeCatalog {
            name: Runtime::ClaudeCode,
            min_version: None,
            display_name: Runtime::ClaudeCode.display_name(),
            command: Runtime::ClaudeCode.command().unwrap(),
            capabilities: capabilities(),
            native_fork: true,
            description: "Anthropic Claude Code CLI",
            install_url: "https://code.claude.com/docs/en/setup",
            default_enabled: true,
            models: vec![
                default_model_option(),
                option("fable", "fable", "Latest Claude Fable."),
                option("opus", "opus", "Latest Claude Opus."),
                option("sonnet", "sonnet", "Latest Claude Sonnet."),
                option("haiku", "haiku", "Latest Claude Haiku."),
            ],
            efforts: claude_efforts,
            skills_dirs: SKILL_DIRS,
            update_args: &["update"],
            npm_package: Some("@anthropic-ai/claude-code"),
        })
    }
    pub const SKILL_DIRS: &[&str] = &[".claude/skills"];
}
pub const ANTIGRAVITY_MODELS: &[(&str, &[&str])] = &[
    ("gemini-4-carbon", &["low", "medium", "high"]),
    ("gemini-4-argon", &["low", "medium", "high"]),
    ("gemini-4-hydrogen", &["low", "medium", "high", "xhigh"]),
    ("gemini-pro-orchestrator", &[]),
    ("gemini-pro-orchestrator-argon", &[]),
    ("gemini-3.8-flash", &["low", "medium", "high"]),
    ("claude-opus-5-5", &["low", "medium", "high"]),
    ("claude-sonnet-5-5", &["low", "medium", "high"]),
    ("gpt-oss-120b", &["medium"]),
];
pub const ANTIGRAVITY_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
pub mod antigravity {
    use super::super::runtime_options::*;
    use super::*;

    /// Split an Antigravity effort-suffixed model ID such as
    /// `gemini-4-hydrogen-xhigh` into `(base_model, effort_level)`.
    pub fn split_leveled_model(id: &str) -> Option<(&str, &str)> {
        ANTIGRAVITY_EFFORTS.iter().find_map(|&level| {
            let base = id.strip_suffix(level)?.strip_suffix('-')?;
            (!base.is_empty()).then_some((base, level))
        })
    }

    /// Split an Antigravity `agy models` `(id, label)` pair when `id` ends with
    /// `-<level>` and `label` ends with ` (<level>)` (ASCII case-insensitive).
    pub fn split_leveled_model_with_label<'a>(
        id: &'a str,
        label: &'a str,
    ) -> Option<(&'a str, &'a str, &'a str)> {
        let (base_id, level) = split_leveled_model(id)?;
        let (base_label, tag) = label.strip_suffix(')')?.rsplit_once(" (")?;
        (!base_label.is_empty() && tag.eq_ignore_ascii_case(level))
            .then_some((base_id, base_label, level))
    }
    pub static PERMISSIONS: Permissions = Permissions {
        offered: &[
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Bypass,
        ],
        strip_flags: &[
            ("--mode", true),
            ("-mode", true),
            ("--dangerously-skip-permissions", false),
            ("-dangerously-skip-permissions", false),
        ],
        equals_on_bool: true,
        variadic_flag: None,
        args: permission_args,
        matches: mode_matches,
        mission_bypass: None,
        strip_on_mission_resume: false,
    };
    fn permission_args(mode: PermissionMode) -> Vec<String> {
        match mode {
            PermissionMode::AcceptEdits => strings(&["--mode", "accept-edits"]),
            PermissionMode::Bypass => strings(&["--dangerously-skip-permissions"]),
            _ => Vec::new(),
        }
    }
    fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
        match mode {
            PermissionMode::Bypass => args
                .iter()
                .any(|arg| go_bool_flag_is_set(arg, "dangerously-skip-permissions")),
            PermissionMode::AcceptEdits => ["--mode", "-mode"]
                .iter()
                .any(|flag| flag_value_matches(args, flag, Some("accept-edits"))),
            _ => false,
        }
    }
    pub fn capabilities() -> RuntimeCapabilities {
        RuntimeCapabilities {
            usage: true,
            effort_needs_launch_model: true,
            ..Default::default()
        }
    }
    pub fn catalog() -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Antigravity,
            min_version: None,
            display_name: Runtime::Antigravity.display_name(),
            command: Runtime::Antigravity.command().unwrap(),
            capabilities: capabilities(),
            native_fork: false,
            description: "Google Antigravity CLI (signs in with a Google account)",
            install_url: "https://antigravity.google/docs/cli/reference",
            default_enabled: true,
            models: std::iter::once(default_model_option())
                .chain(
                    ANTIGRAVITY_MODELS
                        .iter()
                        .map(|(model, efforts)| RuntimeCatalogOption {
                            supported_efforts: Some(
                                efforts.iter().map(|effort| (*effort).into()).collect(),
                            ),
                            ..plain_option(model, model)
                        }),
                )
                .collect(),
            efforts: std::iter::once(default_effort())
                .chain(
                    ANTIGRAVITY_EFFORTS
                        .iter()
                        .map(|effort| plain_option(effort, effort)),
                )
                .collect(),
            skills_dirs: SKILL_DIRS,
            update_args: &[],
            npm_package: None,
        })
    }
    pub const SKILL_DIRS: &[&str] = &[".gemini/antigravity-cli/skills", ".gemini/skills"];
}
pub mod copilot {
    use super::super::runtime_options::*;
    use super::*;
    pub static PERMISSIONS: Permissions = Permissions {
        offered: &[
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Bypass,
        ],
        strip_flags: &[
            ("--allow-tool", true),
            ("--yolo", false),
            ("--allow-all", false),
            ("--allow-all-tools", false),
            ("--allow-all-paths", false),
            ("--allow-all-urls", false),
        ],
        equals_on_bool: false,
        variadic_flag: Some("--allow-tool"),
        args: permission_args,
        matches: mode_matches,
        mission_bypass: None,
        strip_on_mission_resume: false,
    };
    fn permission_args(mode: PermissionMode) -> Vec<String> {
        match mode {
            PermissionMode::AcceptEdits => strings(&["--allow-tool=write"]),
            PermissionMode::Bypass => strings(&["--yolo"]),
            _ => Vec::new(),
        }
    }
    fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
        if mode == PermissionMode::Bypass {
            return ["--yolo", "--allow-all"]
                .iter()
                .any(|flag| flag_value_matches(args, flag, None));
        }
        let pairs: &[(&str, Option<&str>)] = match mode {
            PermissionMode::AcceptEdits => &[("--allow-tool", Some("write"))],
            PermissionMode::Bypass => &[("--yolo", None)],
            _ => &[],
        };
        !pairs.is_empty()
            && pairs
                .iter()
                .all(|&(flag, expected)| flag_value_matches(args, flag, expected))
    }
    pub fn capabilities() -> RuntimeCapabilities {
        RuntimeCapabilities {
            global_skill_toggle: true,
            ..Default::default()
        }
    }
    pub fn catalog() -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Copilot,
            min_version: None,
            display_name: Runtime::Copilot.display_name(),
            command: Runtime::Copilot.command().unwrap(),
            capabilities: capabilities(),
            native_fork: false,
            description: "GitHub Copilot CLI (requires a Copilot subscription)",
            install_url: "https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli",
            default_enabled: true,
            models: std::iter::once(default_model_option())
                .chain(
                    [
                        "auto",
                        "claude-sonnet-5",
                        "claude-fable-5.1",
                        "claude-fable-5",
                        "claude-opus-5",
                        "claude-opus-4.8",
                        "claude-opus-4.8-fast",
                        "claude-opus-4.7",
                        "claude-sonnet-4.6",
                        "claude-haiku-4.5",
                        "gpt-5.6-sol",
                        "gpt-5.6-terra",
                        "gpt-5.6-luna",
                        "gpt-5.5",
                        "gpt-5.4",
                        "gpt-5.4-mini",
                        "gpt-5.3-codex",
                        "gpt-5-mini",
                        "mai-code-1.1-flash",
                        "mai-code-1-flash-picker",
                        "gemini-3.8-flash",
                        "gemini-3.7-flash",
                        "gemini-3.6-flash",
                        "gemini-3.5-flash",
                        "grok-4.5",
                        "kimi-k3",
                        "kimi-k2.7-code",
                    ]
                    .into_iter()
                    .map(|model| plain_option(model, model)),
                )
                .collect(),
            efforts: std::iter::once(default_effort())
                .chain(
                    ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
                        .into_iter()
                        .map(|effort| plain_option(effort, effort)),
                )
                .collect(),
            skills_dirs: SKILL_DIRS,
    update_args: &["update"],
    npm_package: Some("@github/copilot"),
})
    }
    pub const SKILL_DIRS: &[&str] = &[".copilot/skills", ".agents/skills"];
}
pub mod pi {
    use super::super::runtime_options::*;
    use super::*;
    pub static PERMISSIONS: Permissions = Permissions {
        offered: &[],
        strip_flags: &[],
        equals_on_bool: false,
        variadic_flag: None,
        args: |_| Vec::new(),
        matches: |_, _| false,
        mission_bypass: None,
        strip_on_mission_resume: false,
    };
    pub fn catalog() -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Pi,
            min_version: None,
            display_name: Runtime::Pi.display_name(),
            command: Runtime::Pi.command().unwrap(),
            capabilities: capabilities(),
            native_fork: true,
            description: "pi coding agent (bring your own model provider)",
            install_url: "https://github.com/earendil-works/pi",
            default_enabled: true,
            models: vec![default_model_option()],
            efforts: std::iter::once(default_effort())
                .chain(
                    ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                        .into_iter()
                        .map(|effort| plain_option(effort, effort)),
                )
                .collect(),
            skills_dirs: SKILL_DIRS,
            update_args: &["update"],
            npm_package: Some("@earendil-works/pi-coding-agent"),
        })
    }
    pub const SKILL_DIRS: &[&str] = &[".pi/agent/skills", ".agents/skills"];
    pub fn capabilities() -> RuntimeCapabilities {
        RuntimeCapabilities::default()
    }
}
pub mod trae {
    use super::super::runtime_options::*;
    use super::*;
    pub static PERMISSIONS: Permissions = Permissions {
        offered: &[PermissionMode::Default, PermissionMode::Bypass],
        strip_flags: &[("--permission-mode", true)],
        equals_on_bool: false,
        variadic_flag: None,
        args: permission_args,
        matches: mode_matches,
        mission_bypass: None,
        strip_on_mission_resume: true,
    };
    fn permission_args(mode: PermissionMode) -> Vec<String> {
        match mode {
            PermissionMode::Bypass => strings(&["--permission-mode", "bypass_permissions"]),
            _ => Vec::new(),
        }
    }
    fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
        let pairs: &[(&str, Option<&str>)] = match mode {
            PermissionMode::Bypass => &[("--permission-mode", Some("bypass_permissions"))],
            PermissionMode::Auto => &[("--permission-mode", Some("auto"))],
            _ => &[],
        };
        !pairs.is_empty()
            && pairs
                .iter()
                .all(|&(flag, expected)| flag_value_matches(args, flag, expected))
    }
    pub fn catalog() -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Trae,
            min_version: None,
            display_name: Runtime::Trae.display_name(),
            command: Runtime::Trae.command().unwrap(),
            capabilities: capabilities(),
            native_fork: false,
            description: "",
            install_url: "",
            default_enabled: false,
            models: vec![default_model_option()],
            efforts: common_efforts(),
            skills_dirs: SKILL_DIRS,
            update_args: &[],
            npm_package: None,
        })
    }
    pub const SKILL_DIRS: &[&str] = &[".trae/skills"];
    pub fn capabilities() -> RuntimeCapabilities {
        RuntimeCapabilities::default()
    }
}
pub mod muse {
    use super::super::runtime_options::*;
    use super::*;
    pub static PERMISSIONS: Permissions = Permissions {
        offered: &[PermissionMode::Default, PermissionMode::Bypass],
        strip_flags: &[
            ("--approval-mode", true),
            ("--disable-approval", false),
            ("--yolo", false),
        ],
        equals_on_bool: false,
        variadic_flag: None,
        args: permission_args,
        matches: mode_matches,
        mission_bypass: Some(&["--yolo"]),
        strip_on_mission_resume: false,
    };
    fn permission_args(mode: PermissionMode) -> Vec<String> {
        match mode {
            PermissionMode::Bypass => strings(&["--approval-mode", "never"]),
            _ => Vec::new(),
        }
    }
    fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
        match mode {
            PermissionMode::Bypass => {
                args.iter()
                    .any(|a| a == "--yolo" || a == "--disable-approval")
                    || flag_value_matches(args, "--approval-mode", Some("never"))
            }
            _ => false,
        }
    }
    pub fn catalog() -> Option<RuntimeCatalog> {
        // Effort values verified against `muse exec --help`
        // (none|minimal|low|medium|high|xhigh|max|ultra), 2026-11-07.
        // Model IDs stay default-only until verified (follow-up).
        let efforts = vec![
            default_effort(),
            option("none", "None", "No reasoning effort; fastest responses."),
            option("minimal", "Minimal", "Minimal reasoning for trivial tasks."),
            option("low", "Low", "Fast responses with lighter reasoning."),
            option("medium", "Medium", "Balances speed and reasoning depth."),
            option(
                "high",
                "High",
                "Greater reasoning depth for complex problems.",
            ),
            option(
                "xhigh",
                "Extra high",
                "Extra reasoning depth for complex problems.",
            ),
            option(
                "max",
                "Max",
                "Maximum reasoning depth for the hardest problems.",
            ),
            option(
                "ultra",
                "Ultra",
                "Maximum reasoning with automatic task delegation.",
            ),
        ];
        Some(RuntimeCatalog {
            name: Runtime::Muse,
            min_version: None,
            display_name: Runtime::Muse.display_name(),
            command: Runtime::Muse.command().unwrap(),
            capabilities: capabilities(),
            native_fork: false,
            description: "Meta Muse Code",
            install_url: "",
            default_enabled: true,
            models: vec![default_model_option()],
            efforts,
            skills_dirs: SKILL_DIRS,
            update_args: &[],
            npm_package: None,
            min_version: None,
        })
    }
    pub const SKILL_DIRS: &[&str] = &[".agents/skills"];
    pub fn capabilities() -> RuntimeCapabilities {
        RuntimeCapabilities {
            usage: true,
            ..Default::default()
        }
    }
}
pub mod cursor_agent {
    use super::super::runtime_options::*;
    use super::*;

    pub const SKILL_DIRS: &[&str] = &[".cursor/skills", ".agents/skills"];
    pub static PERMISSIONS: Permissions = Permissions {
        offered: &[],
        strip_flags: &[
            ("--force", false),
            ("-f", false),
            ("--yolo", false),
            ("--sandbox", true),
            ("--approve-mcps", false),
            ("--trust", false),
            ("--mode", true),
            ("--plan", false),
        ],
        equals_on_bool: false,
        variadic_flag: None,
        args: |_| Vec::new(),
        matches: |_, _| false,
        mission_bypass: Some(&[
            "--force",
            "--sandbox",
            "disabled",
            "--approve-mcps",
            "--trust",
        ]),
        strip_on_mission_resume: false,
    };

    pub fn catalog() -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Cursor,
            min_version: None,
            display_name: Runtime::Cursor.display_name(),
            command: Runtime::Cursor.command().unwrap(),
            capabilities: RuntimeCapabilities::default(),
            native_fork: false,
            description: "Cursor Agent CLI",
            install_url: "https://cursor.com/docs/cli/installation",
            default_enabled: true,
            models: vec![default_model_option()],
            efforts: vec![default_effort()],
            skills_dirs: SKILL_DIRS,
            update_args: &[],
            npm_package: None,
        })
    }
}
pub mod grok {
    use super::super::runtime_options::*;
    use super::*;
    pub const SKILL_DIRS: &[&str] = &[".grok/skills", ".agents/skills"];
    pub static PERMISSIONS: Permissions = Permissions {
        offered: &[
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ],
        strip_flags: &[
            ("--permission-mode", true),
            ("--always-approve", false),
            ("--yolo", false),
            ("--trust", false),
            ("--sandbox", true),
        ],
        equals_on_bool: false,
        variadic_flag: None,
        args: |mode| {
            strings(&[
                "--permission-mode",
                match mode {
                    PermissionMode::Default => "default",
                    PermissionMode::AcceptEdits => "acceptEdits",
                    PermissionMode::Auto => "auto",
                    PermissionMode::Bypass => "bypassPermissions",
                },
            ])
        },
        matches: |args, mode| {
            (mode == PermissionMode::Bypass
                && args
                    .iter()
                    .any(|arg| matches!(arg.as_str(), "--always-approve" | "--yolo")))
                || flag_value_matches(
                    args,
                    "--permission-mode",
                    Some(match mode {
                        PermissionMode::Default => "default",
                        PermissionMode::AcceptEdits => "acceptEdits",
                        PermissionMode::Auto => "auto",
                        PermissionMode::Bypass => "bypassPermissions",
                    }),
                )
        },
        mission_bypass: None,
        strip_on_mission_resume: false,
    };
    pub fn catalog() -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Grok,
            min_version: None,
            display_name: Runtime::Grok.display_name(),
            command: Runtime::Grok.command().unwrap(),
            capabilities: RuntimeCapabilities::default(),
            native_fork: true,
            description: "xAI Grok Build CLI",
            install_url: "https://docs.x.ai/build/overview",
            default_enabled: true,
            models: vec![default_model_option()],
            efforts: vec![default_effort()],
            skills_dirs: SKILL_DIRS,
            update_args: &[],
            npm_package: None,
        })
    }
}
pub mod opencode {
    use super::super::runtime_options::*;
    use super::*;
    pub const SKILL_DIRS: &[&str] = &[
        ".config/opencode/skills",
        ".agents/skills",
        ".claude/skills",
    ];
    pub static PERMISSIONS: Permissions = Permissions {
        offered: &[PermissionMode::Default, PermissionMode::Bypass],
        strip_flags: &[("--auto", false)],
        equals_on_bool: true,
        variadic_flag: None,
        args: |mode| {
            if mode == PermissionMode::Bypass {
                strings(&["--auto"])
            } else {
                Vec::new()
            }
        },
        matches: |args, mode| {
            mode == PermissionMode::Bypass
                && args.iter().any(|arg| go_bool_flag_is_set(arg, "auto"))
        },
        mission_bypass: Some(&["--auto"]),
        strip_on_mission_resume: false,
    };
    pub fn catalog() -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::OpenCode,
            min_version: None,
            display_name: "OpenCode",
            command: "opencode",
            capabilities: RuntimeCapabilities {
                effort_needs_launch_model: true,
                ..RuntimeCapabilities::default()
            },
            native_fork: false,
            description: "OpenCode v2 CLI (native smoke pending)",
            install_url: "https://opencode.ai",
            default_enabled: true,
            models: vec![default_model_option()],
            efforts: vec![default_effort()],
            skills_dirs: SKILL_DIRS,
            update_args: &[],
            npm_package: None,
        })
    }
}
pub fn catalog_for(runtime: Runtime) -> Option<RuntimeCatalog> {
    match runtime {
        Runtime::Codex => codex::catalog(),
        Runtime::ClaudeCode => claude_code::catalog(),
        Runtime::Antigravity => antigravity::catalog(),
        Runtime::Copilot => copilot::catalog(),
        Runtime::Pi => pi::catalog(),
        Runtime::Trae => trae::catalog(),
        Runtime::Cursor => cursor_agent::catalog(),
        Runtime::Grok => grok::catalog(),
        Runtime::OpenCode => opencode::catalog(),
        Runtime::Muse => muse::catalog(),
        Runtime::Shell => None,
    }
}
static NO_PERMISSIONS: Permissions = Permissions {
    offered: &[],
    strip_flags: &[],
    equals_on_bool: false,
    variadic_flag: None,
    args: |_| Vec::new(),
    matches: |_, _| false,
    mission_bypass: None,
    strip_on_mission_resume: false,
};
pub struct RuntimeMetadata(Runtime);
impl RuntimeMetadata {
    pub fn image_paste_shortcut(&self, windows: bool) -> &'static [u8] {
        if windows && self.0 == Runtime::ClaudeCode {
            b"\x1bv"
        } else {
            b"\x16"
        }
    }
    pub fn catalog(&self) -> Option<RuntimeCatalog> {
        catalog_for(self.0)
    }
    /// Whether the composer reliably accepts and submits input typed during a turn, which the
    /// agent then reads at its next tool call. Only a runtime shown to do so may be sent a
    /// mission notice while it works; every other one waits for idle, because a notice typed
    /// into a busy composer can sit unsubmitted there (#753, #1075).
    pub fn queues_input_mid_turn(&self) -> bool {
        match self.0 {
            Runtime::ClaudeCode => true,
            Runtime::Codex
            | Runtime::Antigravity
            | Runtime::Copilot
            | Runtime::Pi
            | Runtime::Trae
            | Runtime::Cursor
            | Runtime::Grok
            | Runtime::OpenCode
            | Runtime::Muse
            | Runtime::Shell => false,
        }
    }
    pub fn permissions(&self) -> &'static Permissions {
        match self.0 {
            Runtime::Codex => &codex::PERMISSIONS,
            Runtime::ClaudeCode => &claude_code::PERMISSIONS,
            Runtime::Antigravity => &antigravity::PERMISSIONS,
            Runtime::Copilot => &copilot::PERMISSIONS,
            Runtime::Pi => &pi::PERMISSIONS,
            Runtime::Trae => &trae::PERMISSIONS,
            Runtime::Cursor => &cursor_agent::PERMISSIONS,
            Runtime::Grok => &grok::PERMISSIONS,
            Runtime::OpenCode => &opencode::PERMISSIONS,
            Runtime::Muse => &muse::PERMISSIONS,
            Runtime::Shell => &NO_PERMISSIONS,
        }
    }
}
pub fn for_key(key: &str) -> RuntimeMetadata {
    RuntimeMetadata(Runtime::parse(key).unwrap_or(Runtime::Shell))
}
pub fn runtime_list() -> Vec<RuntimeDefinition> {
    Runtime::ALL
        .into_iter()
        .filter_map(catalog_for)
        .map(|catalog| RuntimeDefinition {
            name: catalog.name,
            display_name: catalog.display_name.into(),
            command: catalog.command.into(),
            native_fork: catalog.native_fork,
        })
        .collect()
}
pub fn runtime_default_enabled(runtime: Runtime) -> bool {
    catalog_for(runtime).is_some_and(|catalog| catalog.default_enabled)
}
pub fn model_discovery_runtimes() -> Vec<Runtime> {
    vec![
        Runtime::Codex,
        Runtime::ClaudeCode,
        Runtime::Pi,
        Runtime::Antigravity,
        Runtime::Cursor,
        Runtime::Grok,
        Runtime::OpenCode,
    ]
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn mission_args_use_runtime_bypass() {
        for (runtime, expected) in [
            (
                "codex",
                strings(&[
                    "--ask-for-approval",
                    "never",
                    "--sandbox",
                    "danger-full-access",
                ]),
            ),
            (
                "cursor",
                strings(&[
                    "--force",
                    "--sandbox",
                    "disabled",
                    "--approve-mcps",
                    "--trust",
                ]),
            ),
            (
                "claude-code",
                strings(&["--permission-mode", "bypassPermissions"]),
            ),
            ("shell", vec![]),
            ("unknown", vec![]),
        ] {
            assert_eq!(
                for_key(runtime).permissions().mission_args(),
                expected,
                "{runtime}"
            );
        }
    }

    #[test]
    fn apply_mission_replaces_role_permissions_and_preserves_other_args() {
        let args = strings(&[
            "--sandbox=workspace-write",
            "--model",
            "gpt-5.5",
            "--ask-for-approval",
            "on-request",
        ]);
        assert_eq!(
            for_key("codex").permissions().apply_mission(&args),
            strings(&[
                "--model",
                "gpt-5.5",
                "--ask-for-approval",
                "never",
                "--sandbox",
                "danger-full-access"
            ]),
        );
        assert_eq!(for_key("unknown").permissions().apply_mission(&args), args);
    }
}

#[cfg(test)]
mod input_tests {
    #[test]
    fn image_paste_shortcut_covers_both_platforms_and_unknown_runtime() {
        assert_eq!(
            super::for_key("claude-code").image_paste_shortcut(true),
            b"\x1bv"
        );
        assert_eq!(
            super::for_key("claude-code").image_paste_shortcut(false),
            b"\x16"
        );
        for key in ["codex", "shell", "unknown"] {
            for windows in [true, false] {
                assert_eq!(super::for_key(key).image_paste_shortcut(windows), b"\x16");
            }
        }
    }
}

#[cfg(test)]
mod mid_turn_tests {
    use super::*;

    #[test]
    fn only_claude_code_is_marked_for_mid_turn_delivery() {
        for runtime in Runtime::ALL {
            assert_eq!(
                for_key(runtime.key()).queues_input_mid_turn(),
                runtime == Runtime::ClaudeCode,
                "{runtime:?}"
            );
        }
        for key in ["shell", "unknown", ""] {
            assert!(!for_key(key).queues_input_mid_turn(), "{key}");
        }
    }
}
