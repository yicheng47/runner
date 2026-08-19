use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui::{font, Font, FontFallbacks};
use runner_terminal::palette::{self, TerminalPalette};
use serde::{Deserialize, Serialize};

use crate::theme::{DarkTheme, LightTheme, ThemeIntent};

pub const SIDEBAR_MIN: f32 = 200.;
pub const SIDEBAR_MAX: f32 = 480.;
pub const SIDEBAR_DEFAULT: f32 = 240.;
pub const ZOOM_STEPS: [f32; 8] = [0.8, 0.9, 1., 1.1, 1.2, 1.3, 1.4, 1.5];
pub const TERMINAL_FONT_SIZE_MIN: u16 = 10;
pub const TERMINAL_FONT_SIZE_MAX: u16 = 20;
pub const TERMINAL_FONT_SIZE_DEFAULT: u16 = 13;
pub const TERMINAL_SCROLLBACK_DEFAULT: usize = 10_000;
pub const TERMINAL_SCROLLBACK_OPTIONS: [usize; 4] = [1_000, 5_000, 10_000, 50_000];
pub const CHAT_PANEL_MIN: f32 = 200.;
pub const CHAT_PANEL_MAX: f32 = 480.;
pub const CHAT_PANEL_DEFAULT: f32 = 320.;
pub const WINDOW_WIDTH_DEFAULT: f32 = 1440.;
pub const WINDOW_HEIGHT_DEFAULT: f32 = 900.;
pub const WINDOW_WIDTH_MIN: f32 = 640.;
pub const WINDOW_HEIGHT_MIN: f32 = 480.;

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
            Self::Inter => ("Inter Variable", Some("Inter")),
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
    #[default]
    #[serde(rename = "System default")]
    SystemDefault,
    Menlo,
    Monaco,
    #[serde(rename = "SF Mono")]
    SfMono,
    #[serde(rename = "JetBrains Mono")]
    JetBrainsMono,
    #[serde(rename = "Fira Code")]
    FiraCode,
}

impl TerminalFontFamily {
    pub fn family(self) -> &'static str {
        match self {
            Self::SystemDefault | Self::Menlo => "Menlo",
            Self::Monaco => "Monaco",
            Self::SfMono => "SF Mono",
            Self::JetBrainsMono => "JetBrains Mono",
            Self::FiraCode => "Fira Code",
        }
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
    pub window_width: f32,
    pub window_height: f32,
    pub terminal_theme: TerminalTheme,
    pub terminal_font_family: TerminalFontFamily,
    pub terminal_font_size: u16,
    pub terminal_cursor_style: TerminalCursorStyle,
    pub terminal_scrollback: usize,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    pub sidebar_projects_open: bool,
    pub sidebar_chats_open: bool,
    pub sidebar_collapsed_projects: BTreeSet<String>,
    pub sidebar_collapsed_tabs: BTreeSet<String>,
    pub chat_panel_open: bool,
    pub chat_panel_width: f32,
    pub default_working_dir: String,
    pub default_runtime: String,
    pub disabled_agents: BTreeSet<String>,
    pub enabled_agents: BTreeSet<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            app_theme: ThemeIntent::Auto,
            light_app_theme: LightTheme::Codex,
            dark_app_theme: DarkTheme::Runner,
            app_font_family: AppFontFamily::Inter,
            app_zoom: 1.,
            window_width: WINDOW_WIDTH_DEFAULT,
            window_height: WINDOW_HEIGHT_DEFAULT,
            terminal_theme: TerminalTheme::Runner,
            terminal_font_family: TerminalFontFamily::SystemDefault,
            terminal_font_size: TERMINAL_FONT_SIZE_DEFAULT,
            terminal_cursor_style: TerminalCursorStyle::Block,
            terminal_scrollback: TERMINAL_SCROLLBACK_DEFAULT,
            sidebar_width: SIDEBAR_DEFAULT,
            sidebar_collapsed: false,
            sidebar_projects_open: true,
            sidebar_chats_open: true,
            sidebar_collapsed_projects: BTreeSet::new(),
            sidebar_collapsed_tabs: BTreeSet::new(),
            chat_panel_open: true,
            chat_panel_width: CHAT_PANEL_DEFAULT,
            default_working_dir: String::new(),
            default_runtime: String::new(),
            disabled_agents: BTreeSet::new(),
            enabled_agents: BTreeSet::new(),
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
        self.window_width =
            normalize_window_dimension(self.window_width, WINDOW_WIDTH_DEFAULT, WINDOW_WIDTH_MIN);
        self.window_height = normalize_window_dimension(
            self.window_height,
            WINDOW_HEIGHT_DEFAULT,
            WINDOW_HEIGHT_MIN,
        );
        self.terminal_font_size = normalize_terminal_font_size(self.terminal_font_size);
        self.terminal_scrollback = normalize_terminal_scrollback(self.terminal_scrollback);
        self.sidebar_width = normalize_sidebar_width(self.sidebar_width);
        self.chat_panel_width = normalize_chat_panel_width(self.chat_panel_width);
        self.default_working_dir = self.default_working_dir.trim().to_owned();
        self.default_runtime = self.default_runtime.trim().to_owned();
        self.sidebar_collapsed_projects =
            normalize_agent_set(std::mem::take(&mut self.sidebar_collapsed_projects));
        self.sidebar_collapsed_tabs =
            normalize_agent_set(std::mem::take(&mut self.sidebar_collapsed_tabs));
        self.disabled_agents = normalize_agent_set(std::mem::take(&mut self.disabled_agents));
        self.enabled_agents = normalize_agent_set(std::mem::take(&mut self.enabled_agents));
    }

    pub fn is_agent_enabled(&self, name: &str, default_enabled: bool) -> bool {
        if self.disabled_agents.contains(name) {
            return false;
        }
        default_enabled || self.enabled_agents.contains(name)
    }
}

fn normalize_window_dimension(value: f32, fallback: f32, minimum: f32) -> f32 {
    if value.is_finite() && value > 0. {
        value.max(minimum)
    } else {
        fallback
    }
}

pub fn clamp_window_size_to_display(
    width: f32,
    height: f32,
    display_width: f32,
    display_height: f32,
) -> (f32, f32) {
    (width.min(display_width), height.min(display_height))
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

pub fn normalize_terminal_scrollback(value: usize) -> usize {
    if TERMINAL_SCROLLBACK_OPTIONS.contains(&value) {
        value
    } else {
        TERMINAL_SCROLLBACK_DEFAULT
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
            window_width: 1280.,
            window_height: 720.,
            ..AppSettings::default()
        };
        settings.save(&path).unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.sidebar_width, 376.);
        assert!(loaded.sidebar_collapsed);
        assert_eq!(loaded.window_width, 1280.);
        assert_eq!(loaded.window_height, 720.);
    }

    #[test]
    fn terminal_and_chat_panel_settings_follow_the_shipped_domains() {
        assert_eq!(normalize_terminal_scrollback(1_000), 1_000);
        assert_eq!(normalize_terminal_scrollback(12_345), 10_000);
        assert_eq!(normalize_chat_panel_width(376.), 376.);
        assert_eq!(normalize_chat_panel_width(f32::NAN), 320.);
        assert_eq!(clamp_chat_panel_width(999.), 480.);

        let serialized = serde_json::to_value(AppSettings {
            terminal_cursor_style: TerminalCursorStyle::Bar,
            terminal_scrollback: 50_000,
            chat_panel_open: false,
            chat_panel_width: 440.,
            ..AppSettings::default()
        })
        .unwrap();
        assert_eq!(serialized["terminalCursorStyle"], "bar");
        assert_eq!(serialized["terminalScrollback"], 50_000);
        assert_eq!(serialized["chatPanelOpen"], false);
        assert_eq!(serialized["chatPanelWidth"], 440.);
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
        assert_eq!(loaded.window_width, WINDOW_WIDTH_DEFAULT);
        assert_eq!(loaded.window_height, WINDOW_HEIGHT_DEFAULT);
        assert_eq!(loaded.terminal_font_size, TERMINAL_FONT_SIZE_MAX);
        assert_eq!(loaded.sidebar_width, SIDEBAR_DEFAULT);
        assert!(!loaded.sidebar_collapsed);
        assert!(loaded.sidebar_projects_open);
        assert!(loaded.sidebar_chats_open);

        assert_eq!(normalize_terminal_font_size(0), TERMINAL_FONT_SIZE_DEFAULT);
        assert_eq!(normalize_terminal_font_size(1), TERMINAL_FONT_SIZE_MIN);
    }

    #[test]
    fn restored_window_size_has_a_floor_and_fits_the_current_display() {
        assert_eq!(
            normalize_window_dimension(1., WINDOW_WIDTH_DEFAULT, WINDOW_WIDTH_MIN),
            WINDOW_WIDTH_MIN
        );
        assert_eq!(
            normalize_window_dimension(f32::NAN, WINDOW_HEIGHT_DEFAULT, WINDOW_HEIGHT_MIN),
            WINDOW_HEIGHT_DEFAULT
        );
        assert_eq!(
            clamp_window_size_to_display(2400., 1400., 1440., 875.),
            (1440., 875.)
        );
        assert_eq!(
            clamp_window_size_to_display(1280., 720., 1440., 900.),
            (1280., 720.)
        );
    }

    #[test]
    fn persisted_labels_match_the_react_settings_contract() {
        let value = serde_json::to_value(AppSettings::default()).unwrap();
        assert_eq!(value["appTheme"], "auto");
        assert_eq!(value["lightAppTheme"], "codex");
        assert_eq!(value["darkAppTheme"], "carbon");
        assert_eq!(value["appFontFamily"], "Inter");
        assert_eq!(value["terminalTheme"], "runner");
        assert_eq!(value["terminalFontFamily"], "System default");
        assert_eq!(value["defaultWorkingDir"], "");
        assert_eq!(value["defaultRuntime"], "");
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
        assert_eq!(inter.family.as_ref(), "Inter Variable");
        assert_eq!(
            inter.fallbacks.unwrap().fallback_list(),
            [
                "Inter",
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
