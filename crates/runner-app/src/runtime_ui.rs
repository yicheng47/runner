use crate::theme;
use gpui::Hsla;
use runner_core::protocol::model::Runtime;
use runner_core::protocol::runtime::RuntimeCapabilities;
use runner_core::protocol::runtime::RuntimeCatalogEntry;

pub(crate) struct RuntimeUi {
    pub icon: &'static str,
    pub tint: Hsla,
    pub skills_caption: &'static str,
    pub skill_toggle_hint: &'static str,
    pub usage_title: &'static str,
    pub usage_refresh_spinner: &'static str,
    pub usage_checking_spinner: &'static str,
    pub usage_week: &'static str,
    pub usage_invalid: &'static str,
    pub usage_groups: bool,
    pub usage_order: u8,
}

pub(crate) fn catalog_capabilities(
    catalog: &[RuntimeCatalogEntry],
    key: &str,
) -> RuntimeCapabilities {
    if let Some(entry) = catalog.iter().find(|entry| entry.name.key() == key) {
        return entry.capabilities;
    }
    Runtime::parse(key)
        .and_then(RuntimeCatalogEntry::for_runtime)
        .map(|entry| entry.capabilities)
        .unwrap_or_default()
}

const CLAUDE_CAPTION: &str =
    "ui.toggles_hide_a_skill_from_every_new_claude_code_session_inside_runner_or_not_the_only_writ";
const CODEX_CAPTION: &str =
    "ui.toggles_hide_a_skill_from_every_new_codex_session_inside_runner_or_not_the_only_write_is_a";
const COPILOT_CAPTION: &str =
    "ui.toggles_hide_a_skill_from_every_new_github_copilot_cli_session_inside_runner_or_not_the_on";
const PI_CAPTION: &str =
    "ui.every_skill_in_pi_agent_skills_and_agents_skills_loads_in_every_new_pi_session_runner_does";
const ANTIGRAVITY_CAPTION: &str =
    "ui.every_skill_in_gemini_antigravity_cli_skills_and_gemini_skills_loads_in_every_new_antigrav";
const TRAE_CAPTION: &str =
    "ui.every_skill_in_trae_skills_loads_in_every_new_trae_cli_session_runner_does_not_toggle_skil";
const CURSOR_CAPTION: &str =
    "ui.skills_in_cursor_skills_and_agents_skills_are_available_to_cursor_runner_does_not_toggle_t";
const GROK_CAPTION: &str =
    "ui.skills_in_grok_skills_and_agents_skills_are_available_to_grok_runner_does_not_toggle_them";
const READ_ONLY_CAPTION: &str =
    "ui.every_skill_in_these_roots_loads_in_every_new_session_runner_does_not_toggle_skills_for_th";

pub(crate) fn runtime_ui(runtime: Runtime) -> RuntimeUi {
    let mut ui = RuntimeUi {
        icon: "message-square.svg", tint: theme::text(), skills_caption: crate::i18n::text(READ_ONLY_CAPTION),
        skill_toggle_hint: crate::i18n::text("ui.applies_to_every_new_claude_code_session_inside_runner_or_not_writes_only_skilloverrides_i"),
        usage_title: "Codex", usage_refresh_spinner: "usage-codex-refreshing", usage_checking_spinner: "usage-codex-checking",
        usage_week: "Week", usage_invalid: crate::i18n::text("ui.couldn_t_reach_anthropic"), usage_groups: false, usage_order: u8::MAX,
    };
    match runtime {
        Runtime::ClaudeCode => {
            ui.icon = "claude.svg";
            ui.tint = gpui::rgb(0xd97757).into();
            ui.skills_caption = crate::i18n::text(CLAUDE_CAPTION);
            ui.usage_title = "Claude Code";
            ui.usage_refresh_spinner = "usage-claude-refreshing";
            ui.usage_checking_spinner = "usage-claude-checking";
            ui.usage_order = 0;
        }
        Runtime::Codex => {
            ui.icon = "openai.svg";
            ui.skills_caption = crate::i18n::text(CODEX_CAPTION);
            ui.usage_order = 1;
            ui.usage_invalid = crate::i18n::text("ui.codex_didn_t_answer");
            ui.skill_toggle_hint = crate::i18n::text("ui.applies_to_new_codex_sessions_writes_only_this_skill_s_skills_config_entry_in_codex_config");
        }
        Runtime::Antigravity => {
            ui.icon = "antigravity-icon.png";
            ui.skills_caption = crate::i18n::text(ANTIGRAVITY_CAPTION);
            ui.usage_title = "Antigravity CLI";
            ui.usage_refresh_spinner = "usage-antigravity-refreshing";
            ui.usage_checking_spinner = "usage-antigravity-checking";
            ui.usage_week = "Gemini Models · Week used";
            ui.usage_invalid = crate::i18n::text("ui.antigravity_cli_returned_invalid_usage_data");
            ui.usage_groups = true;
            ui.usage_order = 2;
        }
        Runtime::Pi => {
            ui.icon = "pi.svg";
            ui.skills_caption = crate::i18n::text(PI_CAPTION);
        }
        Runtime::Copilot => {
            ui.icon = "copilot.svg";
            ui.tint = gpui::rgb(0x8534f3).into();
            ui.skills_caption = crate::i18n::text(COPILOT_CAPTION);
            ui.skill_toggle_hint = crate::i18n::text("ui.applies_to_every_new_github_copilot_cli_session_inside_runner_or_not_writes_only_the_disab");
        }
        Runtime::Trae => {
            ui.icon = "trae.svg";
            ui.tint = gpui::rgb(0x32f08c).into();
            ui.skills_caption = crate::i18n::text(TRAE_CAPTION);
        }
        Runtime::Cursor => {
            ui.icon = "cursor.svg";
            ui.skills_caption = crate::i18n::text(CURSOR_CAPTION);
        }
        Runtime::Grok => {
            ui.icon = "grok.svg";
            ui.skills_caption = crate::i18n::text(GROK_CAPTION);
        }
        Runtime::OpenCode => {
            ui.icon = "opencode-icon.png";
            ui.skills_caption = crate::i18n::text("ui.opencode_reads_config_opencode_skills_agents_skills_and_claude_skills_runner_does_not_togg");
        }
        Runtime::Shell => {
            ui.icon = "square-terminal.svg";
            ui.tint = theme::accent();
        }
    }
    ui
}
