use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui::{font, Font, FontFallbacks};
use runner_terminal::palette::{self, TerminalPalette};
use serde::{Deserialize, Serialize};

use crate::keymap::{self, KeymapOverrides};
use crate::theme::{DarkTheme, LightTheme, ThemeIntent};

pub const SIDEBAR_MIN: f32 = 200.;
pub const SIDEBAR_MAX: f32 = 480.;
pub const SIDEBAR_DEFAULT: f32 = 240.;
pub const ZOOM_STEPS: [f32; 8] = [0.8, 0.9, 1., 1.1, 1.2, 1.3, 1.4, 1.5];
pub const TERMINAL_FONT_SIZE_MIN: u16 = 10;
pub const TERMINAL_FONT_SIZE_MAX: u16 = 20;
pub const TERMINAL_FONT_SIZE_DEFAULT: u16 = 13;
pub const TERMINAL_SCROLLBACK_LINES: usize = 10_000;
pub const CHAT_PANEL_MIN: f32 = 200.;
pub const CHAT_PANEL_MAX: f32 = 480.;
pub const CHAT_PANEL_DEFAULT: f32 = 320.;
pub const MISSION_RAIL_MIN: f32 = 200.;
pub const MISSION_RAIL_MAX: f32 = 480.;
pub const MISSION_RAIL_DEFAULT: f32 = 288.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalTheme {
    #[default]
    Runner,
    CatppuccinMocha,
    SolarizedDark,
}

impl TerminalTheme {
    pub fn palette(self) -> TerminalPalette {
        match self {
            Self::Runner => palette::RUNNER,
            Self::CatppuccinMocha => palette::CATPPUCCIN_MOCHA,
            Self::SolarizedDark => palette::SOLARIZED_DARK,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppFontFamily {
    #[default]
    Inter,
    Geist,
    Roboto,
    #[serde(rename = "System UI")]
    SystemUi,
}

impl AppFontFamily {
    pub fn font(self) -> Font {
        let (family, named_fallback) = match self {
            Self::Inter => ("Inter", Some("Inter Variable")),
            Self::Geist => ("Geist Variable", Some("Geist")),
            Self::Roboto => ("Roboto Variable", Some("Roboto")),
            Self::SystemUi => (".SystemUIFont", None),
        };
        let mut fallbacks = Vec::with_capacity(6);
        if let Some(named_fallback) = named_fallback {
            fallbacks.push(named_fallback.to_owned());
            fallbacks.push(".SystemUIFont".to_owned());
        }
        fallbacks.extend(
            ["Segoe UI", "PingFang SC", "Microsoft YaHei", "sans-serif"].map(str::to_owned),
        );
        let mut font = font(family);
        font.fallbacks = Some(FontFallbacks::from_fonts(fallbacks));
        font
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalFontFamily {
    #[serde(
        alias = "System default",
        alias = "Monaco",
        alias = "SF Mono",
        alias = "JetBrains Mono",
        alias = "Fira Code"
    )]
    Menlo,
    #[default]
    #[serde(rename = "Meslo Nerd Font")]
    MesloNerdFont,
}

impl TerminalFontFamily {
    pub fn family(self) -> &'static str {
        match self {
            Self::Menlo => "Menlo",
            Self::MesloNerdFont => "MesloLGS NF",
        }
    }

    pub fn font(self) -> Font {
        let mut font = font(self.family());
        font.fallbacks = Some(FontFallbacks::from_fonts(
            ["PingFang SC", "Microsoft YaHei", "sans-serif"]
                .map(str::to_owned)
                .into(),
        ));
        font
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalCursorStyle {
    #[default]
    Block,
    Underline,
    Bar,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub app_theme: ThemeIntent,
    pub light_app_theme: LightTheme,
    pub dark_app_theme: DarkTheme,
    pub app_font_family: AppFontFamily,
    pub app_zoom: f32,
    pub terminal_theme: TerminalTheme,
    pub terminal_font_family: TerminalFontFamily,
    pub terminal_font_size: u16,
    pub terminal_cursor_style: TerminalCursorStyle,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    pub sidebar_projects_open: bool,
    pub sidebar_chats_open: bool,
    pub sidebar_collapsed_projects: BTreeSet<String>,
    pub chat_panel_open: bool,
    pub chat_panel_width: f32,
    pub mission_rail_open: bool,
    pub mission_rail_width: f32,
    pub mission_rail_view: String,
    pub last_mission_terminal_ids: BTreeMap<String, String>,
    pub default_crew_id: String,
    pub default_working_dir: String,
    pub resume_on_launch: bool,
    pub automatically_check_for_updates: bool,
    pub default_runtime: String,
    pub disabled_agents: BTreeSet<String>,
    pub enabled_agents: BTreeSet<String>,
    #[serde(default, deserialize_with = "keymap::deserialize_overrides")]
    pub keymap_overrides: KeymapOverrides,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            app_theme: ThemeIntent::Auto,
            light_app_theme: LightTheme::Codex,
            dark_app_theme: DarkTheme::Runner,
            app_font_family: AppFontFamily::Inter,
            app_zoom: 1.,
            terminal_theme: TerminalTheme::Runner,
            terminal_font_family: TerminalFontFamily::MesloNerdFont,
            terminal_font_size: TERMINAL_FONT_SIZE_DEFAULT,
            terminal_cursor_style: TerminalCursorStyle::Block,
            sidebar_width: SIDEBAR_DEFAULT,
            sidebar_collapsed: false,
            sidebar_projects_open: true,
            sidebar_chats_open: true,
            sidebar_collapsed_projects: BTreeSet::new(),
            chat_panel_open: true,
            chat_panel_width: CHAT_PANEL_DEFAULT,
            mission_rail_open: true,
            mission_rail_width: MISSION_RAIL_DEFAULT,
            mission_rail_view: "runners".into(),
            last_mission_terminal_ids: BTreeMap::new(),
            default_crew_id: String::new(),
            default_working_dir: String::new(),
            resume_on_launch: false,
            automatically_check_for_updates: true,
            default_runtime: String::new(),
            disabled_agents: BTreeSet::new(),
            enabled_agents: BTreeSet::new(),
            keymap_overrides: KeymapOverrides::new(),
        }
    }
}

impl AppSettings {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let mut settings: Self =
            serde_json::from_str(&contents).with_context(|| format!("parse {}", path.display()))?;
        settings.normalize();
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let contents = serde_json::to_string_pretty(self).context("serialize app settings")?;
        fs::write(path, contents).with_context(|| format!("write {}", path.display()))
    }

    pub fn normalize(&mut self) {
        self.app_zoom = normalize_zoom(self.app_zoom);
        self.terminal_font_size = normalize_terminal_font_size(self.terminal_font_size);
        self.sidebar_width = normalize_sidebar_width(self.sidebar_width);
        self.chat_panel_width = normalize_chat_panel_width(self.chat_panel_width);
        self.mission_rail_width = normalize_mission_rail_width(self.mission_rail_width);
        if !matches!(self.mission_rail_view.as_str(), "runners" | "meta") {
            self.mission_rail_view = "runners".into();
        }
        self.last_mission_terminal_ids
            .retain(|mission_id, session_id| !mission_id.is_empty() && !session_id.is_empty());
        self.default_crew_id = self.default_crew_id.trim().to_owned();
        self.default_working_dir = self.default_working_dir.trim().to_owned();
        self.default_runtime = self.default_runtime.trim().to_owned();
        self.sidebar_collapsed_projects =
            normalize_agent_set(std::mem::take(&mut self.sidebar_collapsed_projects));
        self.disabled_agents = normalize_agent_set(std::mem::take(&mut self.disabled_agents));
        self.enabled_agents = normalize_agent_set(std::mem::take(&mut self.enabled_agents));
        keymap::normalize_overrides(&mut self.keymap_overrides);
    }

    pub fn is_agent_enabled(&self, name: &str, default_enabled: bool) -> bool {
        if self.disabled_agents.contains(name) {
            return false;
        }
        default_enabled || self.enabled_agents.contains(name)
    }
}

fn normalize_agent_set(values: BTreeSet<String>) -> BTreeSet<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

pub fn settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("ui-settings.json")
}

pub fn normalize_zoom(value: f32) -> f32 {
    if !value.is_finite() || value <= 0. {
        return 1.;
    }
    let mut nearest = ZOOM_STEPS[0];
    let mut distance = (nearest - value).abs();
    for step in &ZOOM_STEPS[1..] {
        let next_distance = (*step - value).abs();
        if next_distance < distance {
            nearest = *step;
            distance = next_distance;
        }
    }
    nearest
}

pub fn nudge_zoom(current: f32, direction: i8) -> f32 {
    let normalized = normalize_zoom(current);
    let index = ZOOM_STEPS
        .iter()
        .position(|step| *step == normalized)
        .unwrap_or(2);
    match direction.cmp(&0) {
        std::cmp::Ordering::Greater => ZOOM_STEPS[(index + 1).min(ZOOM_STEPS.len() - 1)],
        std::cmp::Ordering::Less => ZOOM_STEPS[index.saturating_sub(1)],
        std::cmp::Ordering::Equal => 1.,
    }
}

pub fn normalize_terminal_font_size(value: u16) -> u16 {
    if value == 0 {
        TERMINAL_FONT_SIZE_DEFAULT
    } else {
        value.clamp(TERMINAL_FONT_SIZE_MIN, TERMINAL_FONT_SIZE_MAX)
    }
}

pub fn normalize_sidebar_width(value: f32) -> f32 {
    if value.is_finite() && (SIDEBAR_MIN..=SIDEBAR_MAX).contains(&value) {
        value
    } else {
        SIDEBAR_DEFAULT
    }
}

pub fn clamp_sidebar_width(value: f32) -> f32 {
    value.clamp(SIDEBAR_MIN, SIDEBAR_MAX)
}

pub fn normalize_chat_panel_width(value: f32) -> f32 {
    if value.is_finite() && (CHAT_PANEL_MIN..=CHAT_PANEL_MAX).contains(&value) {
        value
    } else {
        CHAT_PANEL_DEFAULT
    }
}

pub fn clamp_chat_panel_width(value: f32) -> f32 {
    value.clamp(CHAT_PANEL_MIN, CHAT_PANEL_MAX)
}

pub fn normalize_mission_rail_width(value: f32) -> f32 {
    if value.is_finite() && (MISSION_RAIL_MIN..=MISSION_RAIL_MAX).contains(&value) {
        value
    } else {
        MISSION_RAIL_DEFAULT
    }
}

pub fn clamp_mission_rail_width(value: f32) -> f32 {
    value.clamp(MISSION_RAIL_MIN, MISSION_RAIL_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_values_snap_and_nudge_within_the_shipped_domain() {
        assert_eq!(normalize_zoom(1.16), 1.2);
        assert_eq!(normalize_zoom(99.), 1.5);
        assert_eq!(normalize_zoom(-1.), 1.);
        assert_eq!(normalize_zoom(f32::NAN), 1.);
        assert_eq!(nudge_zoom(1., 1), 1.1);
        assert_eq!(nudge_zoom(0.8, -1), 0.8);
        assert_eq!(nudge_zoom(1.5, 1), 1.5);
        assert_eq!(nudge_zoom(1.3, 0), 1.);
    }

    #[test]
    fn sidebar_width_and_collapse_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        let settings = AppSettings {
            sidebar_width: 376.,
            sidebar_collapsed: true,
            ..AppSettings::default()
        };
        settings.save(&path).unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.sidebar_width, 376.);
        assert!(loaded.sidebar_collapsed);
    }

    #[test]
    fn terminal_and_chat_panel_settings_follow_the_shipped_domains() {
        assert_eq!(normalize_chat_panel_width(376.), 376.);
        assert_eq!(normalize_chat_panel_width(f32::NAN), 320.);
        assert_eq!(clamp_chat_panel_width(999.), 480.);

        let serialized = serde_json::to_value(AppSettings {
            terminal_cursor_style: TerminalCursorStyle::Bar,
            chat_panel_open: false,
            chat_panel_width: 440.,
            ..AppSettings::default()
        })
        .unwrap();
        assert_eq!(serialized["terminalCursorStyle"], "bar");
        assert!(serialized.get("terminalScrollback").is_none());
        assert_eq!(serialized["chatPanelOpen"], false);
        assert_eq!(serialized["chatPanelWidth"], 440.);
    }

    #[test]
    fn mission_workspace_preferences_round_trip_and_normalize() {
        let mut settings = AppSettings {
            mission_rail_open: false,
            mission_rail_width: 412.,
            mission_rail_view: "meta".into(),
            ..AppSettings::default()
        };
        settings
            .last_mission_terminal_ids
            .insert("mission".into(), "session".into());
        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(value["missionRailOpen"], false);
        assert_eq!(value["missionRailWidth"], 412.);
        assert_eq!(value["missionRailView"], "meta");
        assert_eq!(value["lastMissionTerminalIds"]["mission"], "session");

        settings.mission_rail_width = f32::NAN;
        settings.mission_rail_view = "unknown".into();
        settings
            .last_mission_terminal_ids
            .insert(String::new(), String::new());
        settings.normalize();
        assert_eq!(settings.mission_rail_width, MISSION_RAIL_DEFAULT);
        assert_eq!(settings.mission_rail_view, "runners");
        assert_eq!(settings.last_mission_terminal_ids.len(), 1);
    }

    #[test]
    fn invalid_persisted_values_follow_react_fallbacks() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        fs::write(
            &path,
            r#"{"appZoom":1.26,"windowWidth":0,"windowHeight":-1,"terminalFontSize":99,"sidebarWidth":900}"#,
        )
        .unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.app_zoom, 1.3);
        assert_eq!(loaded.terminal_font_size, TERMINAL_FONT_SIZE_MAX);
        assert_eq!(loaded.sidebar_width, SIDEBAR_DEFAULT);
        assert!(!loaded.sidebar_collapsed);
        assert!(loaded.sidebar_projects_open);
        assert!(loaded.sidebar_chats_open);

        assert_eq!(normalize_terminal_font_size(0), TERMINAL_FONT_SIZE_DEFAULT);
        assert_eq!(normalize_terminal_font_size(1), TERMINAL_FONT_SIZE_MIN);
    }

    #[test]
    fn persisted_labels_match_the_react_settings_contract() {
        let value = serde_json::to_value(AppSettings::default()).unwrap();
        assert_eq!(value["appTheme"], "auto");
        assert_eq!(value["lightAppTheme"], "codex");
        assert_eq!(value["darkAppTheme"], "carbon");
        assert_eq!(value["appFontFamily"], "Inter");
        assert_eq!(value["terminalTheme"], "runner");
        assert_eq!(value["terminalFontFamily"], "Meslo Nerd Font");
        assert_eq!(value["defaultCrewId"], "");
        assert_eq!(value["defaultWorkingDir"], "");
        assert_eq!(value["resumeOnLaunch"], false);
        assert_eq!(value["automaticallyCheckForUpdates"], true);
        assert_eq!(value["defaultRuntime"], "");
        assert_eq!(value["keymapOverrides"], serde_json::json!({}));
    }

    #[test]
    fn keymap_overrides_round_trip_null_and_normalize_bad_entries() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        fs::write(
            &path,
            r#"{
                "keymapOverrides": {
                    "command-palette": null,
                    "toggle-sidebar": {
                        "meta": true,
                        "ctrl": false,
                        "alt": false,
                        "shift": false,
                        "code": "KeyP"
                    },
                    "zoom-in": { "meta": "yes", "code": "Equal" },
                    "future-action": null
                }
            }"#,
        )
        .unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.keymap_overrides.len(), 2);
        assert!(loaded.keymap_overrides["command-palette"].is_none());
        assert_eq!(
            loaded.keymap_overrides["toggle-sidebar"]
                .as_ref()
                .unwrap()
                .code,
            "KeyP"
        );
        loaded.save(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(saved["keymapOverrides"]["command-palette"].is_null());
        assert!(saved["keymapOverrides"].get("future-action").is_none());

        fs::write(&path, r#"{"keymapOverrides": ["bad"]}"#).unwrap();
        assert!(AppSettings::load(&path)
            .unwrap()
            .keymap_overrides
            .is_empty());
    }

    #[test]
    fn terminal_fonts_default_to_meslo_and_normalize_legacy_choices_to_menlo() {
        assert_eq!(
            AppSettings::default().terminal_font_family,
            TerminalFontFamily::MesloNerdFont
        );
        for legacy in [
            "System default",
            "Menlo",
            "Monaco",
            "SF Mono",
            "JetBrains Mono",
            "Fira Code",
        ] {
            let parsed: TerminalFontFamily =
                serde_json::from_str(&format!("\"{legacy}\"")).unwrap();
            assert_eq!(parsed, TerminalFontFamily::Menlo);
        }
    }

    #[test]
    fn terminal_fonts_use_an_explicit_cjk_fallback_chain() {
        for family in [TerminalFontFamily::MesloNerdFont, TerminalFontFamily::Menlo] {
            let font = family.font();
            assert_eq!(font.family.as_ref(), family.family());
            assert_eq!(
                font.fallbacks.unwrap().fallback_list(),
                ["PingFang SC", "Microsoft YaHei", "sans-serif"]
            );
        }
    }

    #[test]
    fn agent_enablement_preserves_shipped_defaults_and_explicit_overrides() {
        let mut settings = AppSettings::default();
        assert!(settings.is_agent_enabled("codex", true));
        assert!(!settings.is_agent_enabled("trae", false));
        settings.enabled_agents.insert("trae".into());
        assert!(settings.is_agent_enabled("trae", false));
        settings.disabled_agents.insert("codex".into());
        assert!(!settings.is_agent_enabled("codex", true));
    }

    #[test]
    fn app_fonts_preserve_the_react_fallback_chain() {
        let inter = AppFontFamily::Inter.font();
        assert_eq!(inter.family.as_ref(), "Inter");
        assert_eq!(
            inter.fallbacks.unwrap().fallback_list(),
            [
                "Inter Variable",
                ".SystemUIFont",
                "Segoe UI",
                "PingFang SC",
                "Microsoft YaHei",
                "sans-serif"
            ]
        );

        let system = AppFontFamily::SystemUi.font();
        assert_eq!(system.family.as_ref(), ".SystemUIFont");
        assert_eq!(
            system.fallbacks.unwrap().fallback_list(),
            ["Segoe UI", "PingFang SC", "Microsoft YaHei", "sans-serif"]
        );
    }
}
