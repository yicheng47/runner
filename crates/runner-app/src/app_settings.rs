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
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
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
            terminal_theme: TerminalTheme::Runner,
            terminal_font_family: TerminalFontFamily::SystemDefault,
            terminal_font_size: TERMINAL_FONT_SIZE_DEFAULT,
            sidebar_width: SIDEBAR_DEFAULT,
            sidebar_collapsed: false,
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
        self.terminal_font_size = normalize_terminal_font_size(self.terminal_font_size);
        self.sidebar_width = normalize_sidebar_width(self.sidebar_width);
        self.default_working_dir = self.default_working_dir.trim().to_owned();
        self.default_runtime = self.default_runtime.trim().to_owned();
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
    fn invalid_persisted_values_follow_react_fallbacks() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        fs::write(
            &path,
            r#"{"appZoom":1.26,"terminalFontSize":99,"sidebarWidth":900}"#,
        )
        .unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.app_zoom, 1.3);
        assert_eq!(loaded.terminal_font_size, TERMINAL_FONT_SIZE_MAX);
        assert_eq!(loaded.sidebar_width, SIDEBAR_DEFAULT);
        assert!(!loaded.sidebar_collapsed);

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
