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

const CLAUDE_CAPTION: &str = "Toggles hide a skill from every new Claude Code session, inside Runner or not; the only write is the skillOverrides key in ~/.claude/settings.json. Click a row to read a skill, hover it to edit. Bundled skills (code-review, loop, …) and project skills always load and are not listed.";
const CODEX_CAPTION: &str = "Toggles hide a skill from every new Codex session, inside Runner or not; the only write is a [[skills.config]] entry in ~/.codex/config.toml. Click a row to read a skill, hover it to edit. Codex's system skills (~/.codex/skills/.system) and plugin skills always load and are not listed.";
const COPILOT_CAPTION: &str = "Toggles hide a skill from every new GitHub Copilot CLI session, inside Runner or not; the only write is the disabledSkills list in ~/.copilot/settings.json, the list `copilot plugins disable --skill` keeps. Click a row to read a skill, hover it to edit. Project skills (.github/skills, .agents/skills) and plugin skills always load and are not listed.";
const PI_CAPTION: &str = "Every skill in ~/.pi/agent/skills and ~/.agents/skills loads in every new pi session; Runner does not toggle skills for pi. Click a row to read a skill, hover it to edit.";
const ANTIGRAVITY_CAPTION: &str = "Every skill in ~/.gemini/antigravity-cli/skills and ~/.gemini/skills loads in every new Antigravity CLI session; Runner does not toggle skills for agy. Built-in and plugin skills are not listed. Click a row to read a skill, hover it to edit.";
const TRAE_CAPTION: &str = "Every skill in ~/.trae/skills loads in every new TRAE CLI session; Runner does not toggle skills for TRAE. TRAE's per-skill switch is disable-model-invocation in the skill frontmatter. Click a row to read a skill, hover it to edit.";
const READ_ONLY_CAPTION: &str = "Every skill in these roots loads in every new session; Runner does not toggle skills for this agent. Click a row to read a skill, hover it to edit.";

pub(crate) fn runtime_ui(runtime: Runtime) -> RuntimeUi {
    let mut ui = RuntimeUi {
        icon: "message-square.svg", tint: theme::text(), skills_caption: READ_ONLY_CAPTION,
        skill_toggle_hint: "Applies to every new Claude Code session, inside Runner or not. Writes only skillOverrides in ~/.claude/settings.json.",
        usage_title: "Codex", usage_refresh_spinner: "usage-codex-refreshing", usage_checking_spinner: "usage-codex-checking",
        usage_week: "Week", usage_invalid: "Couldn't reach Anthropic.", usage_groups: false, usage_order: u8::MAX,
    };
    match runtime {
        Runtime::ClaudeCode => {
            ui.icon = "claude.svg";
            ui.tint = gpui::rgb(0xd97757).into();
            ui.skills_caption = CLAUDE_CAPTION;
            ui.usage_title = "Claude Code";
            ui.usage_refresh_spinner = "usage-claude-refreshing";
            ui.usage_checking_spinner = "usage-claude-checking";
            ui.usage_order = 0;
        }
        Runtime::Codex => {
            ui.icon = "openai.svg";
            ui.skills_caption = CODEX_CAPTION;
            ui.usage_order = 1;
            ui.usage_invalid = "Codex didn't answer.";
            ui.skill_toggle_hint = "Applies to new Codex sessions. Writes only this skill’s [[skills.config]] entry in Codex config.toml; Claude Code is unchanged.";
        }
        Runtime::Antigravity => {
            ui.icon = "antigravity-icon.png";
            ui.skills_caption = ANTIGRAVITY_CAPTION;
            ui.usage_title = "Antigravity CLI";
            ui.usage_refresh_spinner = "usage-antigravity-refreshing";
            ui.usage_week = "Gemini Models · Week used";
            ui.usage_invalid = "Antigravity CLI returned invalid usage data.";
            ui.usage_groups = true;
            ui.usage_order = 2;
        }
        Runtime::Pi => {
            ui.icon = "pi.svg";
            ui.skills_caption = PI_CAPTION;
        }
        Runtime::Copilot => {
            ui.icon = "copilot.svg";
            ui.tint = gpui::rgb(0x8534f3).into();
            ui.skills_caption = COPILOT_CAPTION;
            ui.skill_toggle_hint = "Applies to every new GitHub Copilot CLI session, inside Runner or not. Writes only the disabledSkills list in ~/.copilot/settings.json.";
        }
        Runtime::Trae => {
            ui.icon = "trae.svg";
            ui.tint = gpui::rgb(0x32f08c).into();
            ui.skills_caption = TRAE_CAPTION;
        }
        Runtime::Shell => {
            ui.icon = "square-terminal.svg";
            ui.tint = theme::accent();
        }
    }
    ui
}
