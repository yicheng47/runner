use runner_core::protocol::model::Runtime;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui::{font, Font, FontFallbacks};
use runner_terminal::palette::{self, TerminalPalette};
use serde::{Deserialize, Serialize};

use crate::keymap::{self, KeymapOverrides};
use crate::theme::{DarkTheme, LightTheme, ThemeIntent, ThemeVariant};

pub const SIDEBAR_MIN: f32 = 200.;
pub const SIDEBAR_MAX: f32 = 480.;
pub const SIDEBAR_DEFAULT: f32 = 240.;
pub const ZOOM_STEPS: [f32; 15] = [
    0.6, 0.7, 0.8, 0.9, 1., 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9, 2.,
];
pub const TERMINAL_FONT_SIZE_MIN: u16 = 8;
pub const TERMINAL_FONT_SIZE_MAX: u16 = 32;
pub const TERMINAL_FONT_SIZE_DEFAULT: u16 = 13;
pub const TERMINAL_LINE_HEIGHT_MIN: f32 = 1.0;
pub const TERMINAL_LINE_HEIGHT_MAX: f32 = 1.6;
pub const TERMINAL_LINE_HEIGHT_DEFAULT: f32 = 1.4;
pub const TERMINAL_SCROLLBACK_PRESETS: [usize; 5] = [1_000, 5_000, 10_000, 25_000, 50_000];
pub const TERMINAL_SCROLLBACK_DEFAULT: usize = 10_000;
pub const CHAT_PANEL_MIN: f32 = 200.;
pub const CHAT_PANEL_MAX: f32 = 480.;
pub const CHAT_PANEL_DEFAULT: f32 = 320.;
pub const MISSION_RAIL_MIN: f32 = 200.;
pub const MISSION_RAIL_MAX: f32 = 480.;
pub const MISSION_RAIL_DEFAULT: f32 = 288.;
pub const FILE_VIEWER_MIN: f32 = 200.;
pub const FILE_VIEWER_MAX: f32 = 1200.;
pub const FILE_VIEWER_DEFAULT: f32 = 440.;
pub const FILE_TREE_MIN: f32 = 160.;
pub const FILE_TREE_MAX: f32 = 480.;
pub const FILE_TREE_DEFAULT: f32 = 240.;

/// The terminal palette used while the app resolves to a light variant.
/// Unknown keys (including the retired `match-app` and `monokai`) load as
/// the default, which serde requires to be the last variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LightTerminalTheme {
    Runner,
    CatppuccinMocha,
    #[default]
    #[serde(other)]
    RosePineDawn,
}

impl LightTerminalTheme {
    pub const ALL: [Self; 3] = [Self::RosePineDawn, Self::Runner, Self::CatppuccinMocha];

    pub fn palette(self) -> TerminalPalette {
        match self {
            Self::RosePineDawn => palette::ROSE_PINE_DAWN,
            Self::Runner => palette::RUNNER,
            Self::CatppuccinMocha => palette::CATPPUCCIN_MOCHA,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::RosePineDawn => "rose-pine-dawn",
            Self::Runner => "runner",
            Self::CatppuccinMocha => "catppuccin-mocha",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.key() == value)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::RosePineDawn => "Rosé Pine Dawn",
            Self::Runner => "Runner",
            Self::CatppuccinMocha => "Catppuccin Mocha",
        }
    }
}

/// The terminal palette used while the app resolves to a dark variant. The
/// pre-0.8.7 single `terminalTheme` key loads into this pick: `runner` is
/// still the Runner palette, and the retired `match-app`, `monokai`,
/// `runner-dark` and `runner-light` land on the default like any unknown key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DarkTerminalTheme {
    CatppuccinMocha,
    RosePineDawn,
    #[default]
    #[serde(other)]
    Runner,
}

impl DarkTerminalTheme {
    pub const ALL: [Self; 3] = [Self::Runner, Self::CatppuccinMocha, Self::RosePineDawn];

    pub fn palette(self) -> TerminalPalette {
        match self {
            Self::Runner => palette::RUNNER,
            Self::CatppuccinMocha => palette::CATPPUCCIN_MOCHA,
            Self::RosePineDawn => palette::ROSE_PINE_DAWN,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Runner => "runner",
            Self::CatppuccinMocha => "catppuccin-mocha",
            Self::RosePineDawn => "rose-pine-dawn",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.key() == value)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Runner => "Runner",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::RosePineDawn => "Rosé Pine Dawn",
        }
    }
}

/// The terminal palette for the resolved app variant: the light pick under a
/// light variant, the dark pick otherwise.
pub fn terminal_palette(settings: &AppSettings, variant: ThemeVariant) -> TerminalPalette {
    if variant.is_light() {
        settings.light_terminal_theme.palette()
    } else {
        settings.dark_terminal_theme.palette()
    }
}

pub fn configure_terminal(
    terminal: &runner_terminal::terminal::TerminalMirror,
    settings: &AppSettings,
) {
    use alacritty_terminal::vte::ansi::CursorShape;
    let cursor = match settings.terminal_cursor_style {
        TerminalCursorStyle::Block => CursorShape::Block,
        TerminalCursorStyle::Underline => CursorShape::Underline,
        TerminalCursorStyle::Bar => CursorShape::Beam,
    };
    terminal.configure(
        settings.terminal_scrollback_lines,
        cursor,
        settings.terminal_cursor_blink,
    );
}

/// The UI typeface is pinned to the bundled Inter (`assets/fonts/Inter-*.ttf`);
/// a legacy `appFontFamily` key in `ui-settings.json` is ignored on load.
pub fn app_font() -> Font {
    let mut font = font("Inter");
    font.fallbacks = Some(FontFallbacks::from_fonts(
        runner_app::platform_fonts::APP_FONT_FALLBACKS
            .iter()
            .map(|family| (*family).to_owned())
            .collect(),
    ));
    font
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct TerminalFontFamily(String);

impl Default for TerminalFontFamily {
    fn default() -> Self {
        Self(crate::terminal::fonts::BUNDLED_FONT.to_owned())
    }
}

impl<'de> Deserialize<'de> for TerminalFontFamily {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let family = String::deserialize(deserializer)?;
        Ok(Self::new(match family.as_str() {
            "JetBrains Mono" | "Meslo Nerd Font" => crate::terminal::fonts::BUNDLED_FONT,
            "Menlo" => runner_app::theme::SYSTEM_MONOSPACE_FONT,
            _ => &family,
        }))
    }
}

impl TerminalFontFamily {
    pub fn new(family: impl Into<String>) -> Self {
        Self(family.into())
    }

    pub fn family(&self) -> &str {
        &self.0
    }

    pub fn font(&self, cx: &gpui::App) -> Font {
        let available = crate::terminal::fonts::FontFamilyCache::loaded(cx)
            .is_some_and(|families| families.all.contains(&self.0));
        self.resolved_font(available)
    }

    fn resolved_font(&self, available: bool) -> Font {
        let mut font = font(if available {
            self.0.clone()
        } else {
            crate::terminal::fonts::BUNDLED_FONT.to_owned()
        });
        font.fallbacks = Some(FontFallbacks::from_fonts(
            std::iter::once(crate::terminal::fonts::BUNDLED_FONT)
                .chain(
                    runner_app::platform_fonts::TERMINAL_FONT_FALLBACKS
                        .iter()
                        .copied(),
                )
                .map(str::to_owned)
                .collect(),
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

/// How a Markdown file opens in the viewer; the `</>` button switches a tab
/// between the two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkdownOpenAs {
    Source,
    #[default]
    #[serde(other)]
    Rendered,
}

impl MarkdownOpenAs {
    pub const ALL: [Self; 2] = [Self::Rendered, Self::Source];

    pub fn key(self) -> &'static str {
        match self {
            Self::Rendered => "rendered",
            Self::Source => "source",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileLinkEditor {
    #[default]
    DefaultApp,
    Zed,
    #[serde(rename = "vscode")]
    VsCode,
    Cursor,
}

impl FileLinkEditor {
    pub const ALL: [Self; 4] = [Self::DefaultApp, Self::Zed, Self::VsCode, Self::Cursor];

    pub fn label(self) -> &'static str {
        match self {
            Self::DefaultApp => crate::i18n::text("ui.default_app"),
            Self::Zed => "Zed",
            Self::VsCode => "VS Code",
            Self::Cursor => "Cursor",
        }
    }

    /// The command-line launcher the editor ships, when it opens through one.
    pub fn cli(self) -> Option<&'static str> {
        match self {
            Self::Zed => Some("zed"),
            Self::VsCode => Some("code"),
            Self::Cursor => Some("cursor"),
            Self::DefaultApp => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotificationSettings {
    pub chats: ChatNotifications,
    pub missions: MissionNotifications,
    pub usage: UsageNotifications,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChatNotifications {
    pub input: bool,
    pub finished: bool,
    pub failed: bool,
}

impl Default for ChatNotifications {
    fn default() -> Self {
        Self {
            input: true,
            finished: true,
            failed: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MissionNotifications {
    pub question: bool,
    pub input: bool,
    pub idle: bool,
    pub failed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageNotifications {
    pub near_limit: bool,
    pub threshold: u8,
    pub alerted: Vec<UsageAlertPeriod>,
}
impl Default for UsageNotifications {
    fn default() -> Self {
        Self {
            near_limit: true,
            threshold: 80,
            alerted: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAlertPeriod {
    pub runtime: runner_core::Runtime,
    pub account: String,
    pub window: String,
    pub resets_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageSettings {
    pub sidebar: bool,
    pub codex: bool,
    pub claude_code: bool,
    pub antigravity: bool,
    pub muse: bool,
    pub five_hour: bool,
}
impl Default for UsageSettings {
    fn default() -> Self {
        Self {
            sidebar: true,
            codex: true,
            claude_code: true,
            antigravity: true,
            muse: true,
            five_hour: true,
        }
    }
}
impl UsageSettings {
    pub fn badge(&self, runtime: runner_core::Runtime) -> bool {
        self.sidebar
            && match runtime {
                runner_core::Runtime::Codex => self.codex,
                runner_core::Runtime::ClaudeCode => self.claude_code,
                runner_core::Runtime::Antigravity => self.antigravity,
                runner_core::Runtime::Muse => self.muse,
                _ => false,
            }
    }
    pub fn checks(&self, runtime: runner_core::Runtime, alert: bool) -> bool {
        self.badge(runtime) || alert
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteSidebarSettings {
    pub active_only: bool,
    pub projects_open: bool,
    pub chats_open: bool,
    pub collapsed_projects: BTreeSet<String>,
    pub collapsed_active_groups: BTreeSet<String>,
    pub collapsed_link_groups: BTreeSet<String>,
}

impl Default for RemoteSidebarSettings {
    fn default() -> Self {
        Self {
            active_only: false,
            projects_open: true,
            chats_open: true,
            collapsed_projects: BTreeSet::new(),
            collapsed_active_groups: BTreeSet::new(),
            collapsed_link_groups: BTreeSet::new(),
        }
    }
}

impl RemoteSidebarSettings {
    pub(crate) fn from_settings(settings: &AppSettings) -> Self {
        Self {
            active_only: settings.sidebar_active_only,
            projects_open: settings.sidebar_projects_open,
            chats_open: settings.sidebar_chats_open,
            collapsed_projects: settings.sidebar_collapsed_projects.clone(),
            collapsed_active_groups: settings.sidebar_collapsed_active_groups.clone(),
            collapsed_link_groups: settings.sidebar_collapsed_link_groups.clone(),
        }
    }

    pub(crate) fn apply_to(self, settings: &mut AppSettings) {
        settings.sidebar_active_only = self.active_only;
        settings.sidebar_projects_open = self.projects_open;
        settings.sidebar_chats_open = self.chats_open;
        settings.sidebar_collapsed_projects = self.collapsed_projects;
        settings.sidebar_collapsed_active_groups = self.collapsed_active_groups;
        settings.sidebar_collapsed_link_groups = self.collapsed_link_groups;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub language: runner_app::i18n::Language,
    pub notifications: NotificationSettings,
    pub usage: UsageSettings,
    pub app_theme: ThemeIntent,
    pub window_material: runner_app::appearance::WindowMaterial,
    pub light_app_theme: LightTheme,
    pub dark_app_theme: DarkTheme,
    pub app_zoom: f32,
    pub light_terminal_theme: LightTerminalTheme,
    #[serde(alias = "terminalTheme")]
    pub dark_terminal_theme: DarkTerminalTheme,
    pub terminal_font_family: TerminalFontFamily,
    pub terminal_font_size: u16,
    pub terminal_line_height: f32,
    pub terminal_cursor_style: TerminalCursorStyle,
    pub terminal_cursor_blink: bool,
    pub terminal_scrollback_lines: usize,
    pub terminal_copy_on_select: bool,
    pub option_as_meta: runner_app::terminal_ime::OptionAsMeta,
    pub sidebar_width: f32,
    pub roles_page_size: usize,
    pub crews_page_size: usize,
    pub sidebar_collapsed: bool,
    pub sidebar_active_only: bool,
    pub sidebar_projects_open: bool,
    pub sidebar_chats_open: bool,
    pub sidebar_collapsed_projects: BTreeSet<String>,
    pub sidebar_collapsed_active_groups: BTreeSet<String>,
    /// Collapsed linked groups (#748), by the node id of the tab heading them.
    pub sidebar_collapsed_link_groups: BTreeSet<String>,
    pub remote_sidebars: BTreeMap<String, RemoteSidebarSettings>,
    pub chat_panel_open: bool,
    pub chat_panel_width: f32,
    pub mission_rail_open: bool,
    pub mission_rail_width: f32,
    /// The file's width in a side panel widened by an open file, beside the tree.
    pub file_viewer_width: f32,
    /// The tree's width on the outer edge of a panel widened by an open file.
    pub file_tree_width: f32,
    /// Settings → Side panel → Panel: off hides every tab's panel and its
    /// toggle, keeping the open state and the view switches below.
    pub show_side_panel: bool,
    /// Whether the `+` menu offers Info and Files.
    pub info_view: bool,
    pub files_view: bool,
    pub file_icons: crate::file_icons::IconSet,
    /// `None` follows the terminal's Font family.
    pub files_font_family: Option<TerminalFontFamily>,
    /// `None` follows the terminal's font size.
    pub files_font_size: Option<u16>,
    pub markdown_open_as: MarkdownOpenAs,
    pub last_mission_terminal_ids: BTreeMap<String, String>,
    pub default_crew_id: String,
    pub default_working_dir: String,
    pub file_link_editor: FileLinkEditor,
    pub resume_on_launch: bool,
    pub quit_behavior: runner_app::lifecycle::QuitBehavior,
    pub last_quit_choice: runner_app::lifecycle::QuitChoice,
    pub automatically_check_for_updates: bool,
    #[cfg(any(windows, target_os = "linux"))]
    pub automatically_download_updates: bool,
    pub skipped_update_version: Option<String>,
    pub default_runtime: String,
    pub disabled_agents: BTreeSet<String>,
    pub enabled_agents: BTreeSet<String>,
    pub initialized_mcp_clients: BTreeSet<String>,
    pub mcp_registrations_removed: bool,
    pub initialized_skill_roots: BTreeSet<String>,
    pub initialized_command_install: bool,
    pub runner_skill_enabled: bool,
    #[serde(default, deserialize_with = "keymap::deserialize_overrides")]
    pub keymap_overrides: KeymapOverrides,
    #[cfg(any(windows, target_os = "linux"))]
    pub terminal_ctrl_keys: keymap::TerminalCtrlKeys,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: runner_app::i18n::Language::System,
            notifications: NotificationSettings::default(),
            usage: UsageSettings::default(),
            app_theme: ThemeIntent::Auto,
            window_material: Default::default(),
            light_app_theme: LightTheme::RunnerLight,
            dark_app_theme: DarkTheme::Runner,
            app_zoom: 1.,
            light_terminal_theme: LightTerminalTheme::RosePineDawn,
            dark_terminal_theme: DarkTerminalTheme::Runner,
            terminal_font_family: TerminalFontFamily::default(),
            terminal_font_size: TERMINAL_FONT_SIZE_DEFAULT,
            terminal_line_height: TERMINAL_LINE_HEIGHT_DEFAULT,
            terminal_cursor_style: TerminalCursorStyle::Block,
            terminal_cursor_blink: false,
            terminal_scrollback_lines: TERMINAL_SCROLLBACK_DEFAULT,
            terminal_copy_on_select: false,
            option_as_meta: Default::default(),
            sidebar_width: SIDEBAR_DEFAULT,
            roles_page_size: runner_app::ui::DEFAULT_PAGE_SIZE,
            crews_page_size: runner_app::ui::DEFAULT_PAGE_SIZE,
            sidebar_collapsed: false,
            sidebar_active_only: false,
            sidebar_projects_open: true,
            sidebar_chats_open: true,
            sidebar_collapsed_projects: BTreeSet::new(),
            sidebar_collapsed_active_groups: BTreeSet::new(),
            sidebar_collapsed_link_groups: BTreeSet::new(),
            remote_sidebars: BTreeMap::new(),
            chat_panel_open: true,
            chat_panel_width: CHAT_PANEL_DEFAULT,
            mission_rail_open: true,
            mission_rail_width: MISSION_RAIL_DEFAULT,
            file_viewer_width: FILE_VIEWER_DEFAULT,
            file_tree_width: FILE_TREE_DEFAULT,
            show_side_panel: true,
            info_view: true,
            files_view: true,
            file_icons: Default::default(),
            files_font_family: None,
            files_font_size: None,
            markdown_open_as: MarkdownOpenAs::Rendered,
            last_mission_terminal_ids: BTreeMap::new(),
            default_crew_id: String::new(),
            default_working_dir: String::new(),
            file_link_editor: FileLinkEditor::DefaultApp,
            resume_on_launch: true,
            quit_behavior: Default::default(),
            last_quit_choice: Default::default(),
            automatically_check_for_updates: true,
            #[cfg(any(windows, target_os = "linux"))]
            automatically_download_updates: true,
            skipped_update_version: None,
            default_runtime: String::new(),
            disabled_agents: BTreeSet::new(),
            enabled_agents: BTreeSet::new(),
            initialized_mcp_clients: BTreeSet::new(),
            mcp_registrations_removed: false,
            initialized_skill_roots: BTreeSet::new(),
            initialized_command_install: false,
            runner_skill_enabled: true,
            keymap_overrides: KeymapOverrides::new(),
            #[cfg(any(windows, target_os = "linux"))]
            terminal_ctrl_keys: Default::default(),
        }
    }
}

impl AppSettings {
    pub(crate) fn terminal_ctrl_keys(&self) -> keymap::TerminalCtrlKeys {
        #[cfg(any(windows, target_os = "linux"))]
        {
            self.terminal_ctrl_keys
        }
        #[cfg(target_os = "macos")]
        {
            keymap::TerminalCtrlKeys::RunnerFirst
        }
    }

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
        self.terminal_line_height = normalize_terminal_line_height(self.terminal_line_height);
        self.terminal_scrollback_lines =
            normalize_terminal_scrollback(self.terminal_scrollback_lines);
        self.sidebar_width = normalize_sidebar_width(self.sidebar_width);
        self.roles_page_size = runner_app::ui::list::normalize_page_size(self.roles_page_size);
        self.crews_page_size = runner_app::ui::list::normalize_page_size(self.crews_page_size);
        self.chat_panel_width = normalize_chat_panel_width(self.chat_panel_width);
        self.mission_rail_width = normalize_mission_rail_width(self.mission_rail_width);
        self.file_viewer_width = normalize_file_viewer_width(self.file_viewer_width);
        self.file_tree_width = normalize_file_tree_width(self.file_tree_width);
        self.files_font_family = self
            .files_font_family
            .take()
            .filter(|family| !family.family().trim().is_empty());
        self.files_font_size = self
            .files_font_size
            .filter(|size| *size != 0)
            .map(normalize_terminal_font_size);
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

    /// Whether a chat's or a terminal tab's panel is open: shown at all and
    /// not collapsed.
    pub fn chat_panel_shown(&self) -> bool {
        self.show_side_panel && self.chat_panel_open
    }

    pub fn mission_rail_shown(&self) -> bool {
        self.show_side_panel && self.mission_rail_open
    }

    /// The file viewer's code font: its own pick, or the terminal's; a font
    /// that is not installed draws as the bundled one, the pick kept.
    pub fn files_font(&self, cx: &gpui::App) -> Font {
        self.files_font_family
            .as_ref()
            .unwrap_or(&self.terminal_font_family)
            .font(cx)
    }

    pub fn files_font_size(&self) -> u16 {
        self.files_font_size.unwrap_or(self.terminal_font_size)
    }

    pub fn model_runtimes(&self) -> Vec<Runtime> {
        runner_core::protocol::runtime_metadata::model_discovery_runtimes()
            .into_iter()
            .filter(|runtime| {
                self.is_agent_enabled(
                    *runtime,
                    runner_core::protocol::runtime_metadata::runtime_default_enabled(*runtime),
                )
            })
            .collect()
    }

    /// The enabled runtimes that report a usage allowance. Not `model_runtimes`:
    /// that list is the runtimes that discover models, and a runtime can report
    /// usage without discovering models.
    pub fn usage_runtimes(&self) -> Vec<Runtime> {
        Runtime::ALL
            .into_iter()
            .filter(|runtime| {
                runner_core::protocol::RuntimeCatalogEntry::for_runtime(*runtime)
                    .is_some_and(|entry| entry.capabilities.usage)
                    && self.is_agent_enabled(
                        *runtime,
                        runner_core::protocol::runtime_metadata::runtime_default_enabled(*runtime),
                    )
            })
            .collect()
    }

    pub fn is_agent_enabled(&self, name: Runtime, default_enabled: bool) -> bool {
        if self.disabled_agents.contains(name.key()) {
            return false;
        }
        default_enabled || self.enabled_agents.contains(name.key())
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
        .unwrap_or(4);
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

pub fn normalize_terminal_line_height(value: f32) -> f32 {
    if !value.is_finite() {
        return TERMINAL_LINE_HEIGHT_DEFAULT;
    }
    let tenths = (value * 10.).round() as i32;
    tenths.clamp(
        (TERMINAL_LINE_HEIGHT_MIN * 10.) as i32,
        (TERMINAL_LINE_HEIGHT_MAX * 10.) as i32,
    ) as f32
        / 10.
}

pub fn normalize_terminal_scrollback(value: usize) -> usize {
    value.clamp(
        TERMINAL_SCROLLBACK_PRESETS[0],
        TERMINAL_SCROLLBACK_PRESETS[TERMINAL_SCROLLBACK_PRESETS.len() - 1],
    )
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

pub fn normalize_file_viewer_width(value: f32) -> f32 {
    if value.is_finite() && (FILE_VIEWER_MIN..=FILE_VIEWER_MAX).contains(&value) {
        value
    } else {
        FILE_VIEWER_DEFAULT
    }
}

pub fn clamp_file_viewer_width(value: f32) -> f32 {
    value.clamp(FILE_VIEWER_MIN, FILE_VIEWER_MAX)
}

pub fn normalize_file_tree_width(value: f32) -> f32 {
    if value.is_finite() && (FILE_TREE_MIN..=FILE_TREE_MAX).contains(&value) {
        value
    } else {
        FILE_TREE_DEFAULT
    }
}

pub fn clamp_file_tree_width(value: f32) -> f32 {
    value.clamp(FILE_TREE_MIN, FILE_TREE_MAX)
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
    #[test]
    fn saved_local_only_and_legacy_remote_keys_are_ignored_and_dropped() {
        for key in ["localOnly", "remoteAccessEnabled", "remote_access_enabled"] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("settings.json");
            std::fs::write(
                &path,
                serde_json::json!({(key):true,"appZoom":1.2}).to_string(),
            )
            .unwrap();
            let settings = super::AppSettings::load(&path).unwrap();
            assert_eq!(settings.app_zoom, 1.2);
            settings.save(&path).unwrap();
            let saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert!(saved.get(key).is_none());
        }
    }

    use super::*;

    #[test]
    fn remote_sidebars_default_for_older_settings_and_partial_views() {
        let old: AppSettings = serde_json::from_str(r#"{"sidebarProjectsOpen":false}"#).unwrap();
        assert!(old.remote_sidebars.is_empty());
        assert!(!old.sidebar_projects_open);
        let partial: AppSettings =
            serde_json::from_str(r#"{"remoteSidebars":{"host":{"activeOnly":true}}}"#).unwrap();
        assert_eq!(
            partial.remote_sidebars["host"],
            RemoteSidebarSettings {
                active_only: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn language_defaults_migrates_and_persists_intent() {
        use runner_app::i18n::{Language, Locale};
        let old: AppSettings = serde_json::from_str(r#"{"appZoom":1.2}"#).unwrap();
        assert_eq!(old.language, Language::System);
        assert_eq!(old.app_zoom, 1.2);
        let unknown: AppSettings = serde_json::from_str(r#"{"language":"future"}"#).unwrap();
        assert_eq!(unknown.language, Language::System);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        for (intent, saved) in [
            (Language::System, "system"),
            (Language::English, "en"),
            (Language::SimplifiedChinese, "zh-hans"),
        ] {
            let settings = AppSettings {
                language: intent,
                ..old.clone()
            };
            settings.save(&path).unwrap();
            let json: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(json["language"], saved);
            assert_eq!(AppSettings::load(&path).unwrap(), settings);
        }
        assert_eq!(
            Language::System.resolve(Some("zh-Hans")),
            Locale::SimplifiedChinese
        );
        assert_eq!(serde_json::to_value(Language::System).unwrap(), "system");
    }

    #[test]
    fn option_as_meta_defaults_and_persists_each_mode() {
        use runner_app::terminal_ime::OptionAsMeta;

        for json in [r#"{"appZoom":1.2}"#, r#"{"optionAsMeta":"future"}"#] {
            let settings: AppSettings = serde_json::from_str(json).unwrap();
            assert_eq!(settings.option_as_meta, OptionAsMeta::Automatic);
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        for mode in OptionAsMeta::ALL {
            let settings = AppSettings {
                option_as_meta: mode,
                ..AppSettings::default()
            };
            settings.save(&path).unwrap();
            let json: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(json["optionAsMeta"], mode.key());
            assert_eq!(AppSettings::load(&path).unwrap(), settings);
        }
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn terminal_ctrl_keys_default_to_runner_first_and_persist_each_mode() {
        use crate::keymap::TerminalCtrlKeys;

        for json in [r#"{"appZoom":1.2}"#, r#"{"terminalCtrlKeys":"future"}"#] {
            let settings: AppSettings = serde_json::from_str(json).unwrap();
            assert_eq!(settings.terminal_ctrl_keys(), TerminalCtrlKeys::RunnerFirst);
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        for (mode, saved) in [
            (TerminalCtrlKeys::RunnerFirst, "runner-first"),
            (TerminalCtrlKeys::TerminalFirst, "terminal-first"),
        ] {
            let settings = AppSettings {
                terminal_ctrl_keys: mode,
                ..AppSettings::default()
            };
            settings.save(&path).unwrap();
            let json: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(json["terminalCtrlKeys"], saved);
            let loaded = AppSettings::load(&path).unwrap();
            assert_eq!(loaded, settings);
            assert_eq!(loaded.terminal_ctrl_keys(), mode);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn terminal_ctrl_keys_do_not_exist_on_macos() {
        let settings: AppSettings =
            serde_json::from_str(r#"{"terminalCtrlKeys":"terminal-first"}"#).unwrap();
        assert_eq!(
            settings.terminal_ctrl_keys(),
            crate::keymap::TerminalCtrlKeys::RunnerFirst
        );
        let json = serde_json::to_value(&settings).unwrap();
        assert!(json.get("terminalCtrlKeys").is_none());
    }

    #[test]
    fn file_link_editor_defaults_to_the_default_app_for_pre_feature_settings() {
        let settings: AppSettings = serde_json::from_str(r#"{"appZoom":1.0}"#).unwrap();
        assert_eq!(settings.file_link_editor, FileLinkEditor::DefaultApp);

        let json = serde_json::to_string(&AppSettings {
            file_link_editor: FileLinkEditor::VsCode,
            ..AppSettings::default()
        })
        .unwrap();
        assert!(json.contains(r#""fileLinkEditor":"vscode""#), "{json}");
        let reloaded: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(reloaded.file_link_editor, FileLinkEditor::VsCode);
    }

    #[test]
    fn side_panel_settings_default_to_todays_behaviour_and_round_trip() {
        use crate::file_icons::IconSet;
        let old: AppSettings = serde_json::from_str(r#"{"appZoom":1.0}"#).unwrap();
        assert!(old.show_side_panel && old.info_view && old.files_view);
        assert_eq!(old.file_icons, IconSet::Catppuccin);
        assert_eq!(old.files_font_family, None);
        assert_eq!(old.files_font_size, None);
        assert_eq!(old.files_font_size(), TERMINAL_FONT_SIZE_DEFAULT);
        assert_eq!(old.markdown_open_as, MarkdownOpenAs::Rendered);
        assert_eq!(old, AppSettings::default());

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        let settings = AppSettings {
            show_side_panel: false,
            info_view: false,
            files_view: false,
            file_icons: IconSet::Minimal,
            files_font_family: Some(TerminalFontFamily::new("Berkeley Mono")),
            files_font_size: Some(17),
            markdown_open_as: MarkdownOpenAs::Source,
            ..AppSettings::default()
        };
        settings.save(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["showSidePanel"], false);
        assert_eq!(json["fileIcons"], "minimal");
        assert_eq!(json["filesFontFamily"], "Berkeley Mono");
        assert_eq!(json["filesFontSize"], 17);
        assert_eq!(json["markdownOpenAs"], "source");
        assert_eq!(AppSettings::load(&path).unwrap(), settings);
        // The removed Open with a single click, A new chat's panel opens and
        // Syntax highlighting are ignored on load.
        let saved: AppSettings = serde_json::from_str(
            r#"{"appZoom":1.0,"filesSingleClick":false,"newChatPanel":"info","filesSyntaxHighlighting":false}"#,
        )
        .unwrap();
        assert_eq!(saved, AppSettings::default());
        for set in IconSet::ALL {
            assert_eq!(IconSet::from_key(set.key()), Some(set));
        }
        for mode in MarkdownOpenAs::ALL {
            assert_eq!(MarkdownOpenAs::parse(mode.key()), Some(mode));
        }
    }

    #[test]
    fn bad_side_panel_values_normalize_to_their_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        for (json, size) in [
            (
                r#"{"fileIcons":"seti","markdownOpenAs":"preview","newChatPanel":"changes","filesFontFamily":"  ","filesFontSize":0}"#,
                None,
            ),
            (
                r#"{"filesFontFamily":"","filesFontSize":99}"#,
                Some(TERMINAL_FONT_SIZE_MAX),
            ),
            (
                r#"{"filesFontFamily":null,"filesFontSize":3}"#,
                Some(TERMINAL_FONT_SIZE_MIN),
            ),
        ] {
            fs::write(&path, json).unwrap();
            let settings = AppSettings::load(&path).unwrap();
            assert_eq!(settings.file_icons, crate::file_icons::IconSet::Catppuccin);
            assert_eq!(settings.markdown_open_as, MarkdownOpenAs::Rendered);
            assert_eq!(settings.files_font_family, None, "{json}");
            assert_eq!(settings.files_font_size, size, "{json}");
        }
    }

    #[test]
    fn the_viewer_font_follows_the_terminal_and_a_missing_one_draws_bundled() {
        let terminal = AppSettings {
            terminal_font_size: 15,
            ..AppSettings::default()
        };
        assert_eq!(terminal.files_font_size(), 15, "follows the terminal");
        let own = AppSettings {
            files_font_size: Some(11),
            ..terminal.clone()
        };
        assert_eq!(own.files_font_size(), 11);
        assert_eq!(
            AppSettings {
                files_font_size: None,
                ..own
            }
            .files_font_size(),
            15,
            "Reset follows the terminal again"
        );

        // A saved font that is not installed draws as the bundled font,
        // and the choice survives a save.
        let missing = TerminalFontFamily::new("Gone Mono");
        assert_eq!(
            missing.resolved_font(false).family.as_ref(),
            crate::terminal::fonts::BUNDLED_FONT
        );
        assert_eq!(missing.resolved_font(true).family.as_ref(), "Gone Mono");
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        AppSettings {
            files_font_family: Some(missing.clone()),
            ..AppSettings::default()
        }
        .save(&path)
        .unwrap();
        assert_eq!(
            AppSettings::load(&path).unwrap().files_font_family,
            Some(missing)
        );
    }

    #[test]
    fn old_mission_permission_setting_is_ignored() {
        let settings: AppSettings =
            serde_json::from_str(r#"{"missionPermissionMode":"auto"}"#).unwrap();
        let saved = serde_json::to_string(&settings).unwrap();
        assert!(!saved.contains("missionPermissionMode"));
    }

    #[test]
    fn zoom_values_snap_and_nudge_within_the_shipped_domain() {
        assert_eq!(normalize_zoom(1.16), 1.2);
        assert_eq!(normalize_zoom(99.), 2.);
        assert_eq!(normalize_zoom(0.01), 0.6);
        assert_eq!(normalize_zoom(-1.), 1.);
        assert_eq!(normalize_zoom(f32::NAN), 1.);
        assert_eq!(nudge_zoom(1., 1), 1.1);
        assert_eq!(nudge_zoom(0.6, -1), 0.6);
        assert_eq!(nudge_zoom(2., 1), 2.);
        assert_eq!(nudge_zoom(1.3, 0), 1.);
    }

    #[test]
    fn sidebar_width_and_collapse_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        let settings = AppSettings {
            sidebar_width: 376.,
            sidebar_active_only: true,
            sidebar_collapsed: true,
            ..AppSettings::default()
        };
        settings.save(&path).unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.sidebar_width, 376.);
        assert!(loaded.sidebar_collapsed);
        assert!(loaded.sidebar_active_only);
    }

    #[test]
    fn list_page_sizes_default_and_persist_independently() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        std::fs::write(&path, "{}").unwrap();
        let old = AppSettings::load(&path).unwrap();
        assert_eq!((old.roles_page_size, old.crews_page_size), (10, 10));
        let settings = AppSettings {
            roles_page_size: 25,
            crews_page_size: 50,
            ..old
        };
        settings.save(&path).unwrap();
        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!((loaded.roles_page_size, loaded.crews_page_size), (25, 50));
        std::fs::write(&path, r#"{"rolesPageSize":0,"crewsPageSize":8}"#).unwrap();
        let invalid = AppSettings::load(&path).unwrap();
        assert_eq!((invalid.roles_page_size, invalid.crews_page_size), (10, 10));
    }

    #[test]
    fn terminal_and_chat_panel_settings_follow_the_shipped_domains() {
        assert_eq!(normalize_chat_panel_width(376.), 376.);
        assert_eq!(normalize_chat_panel_width(f32::NAN), 320.);
        assert_eq!(clamp_chat_panel_width(999.), 480.);
        assert_eq!(normalize_file_viewer_width(f32::NAN), 440.);
        assert_eq!(normalize_file_viewer_width(9999.), 440.);
        assert_eq!(clamp_file_viewer_width(9999.), 1200.);
        assert_eq!(normalize_file_tree_width(320.), 320.);
        assert_eq!(normalize_file_tree_width(f32::NAN), 240.);
        assert_eq!(normalize_file_tree_width(100.), 240.);
        assert_eq!(normalize_file_tree_width(481.), 240.);
        assert_eq!(clamp_file_tree_width(100.), 160.);
        assert_eq!(clamp_file_tree_width(9999.), 480.);

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
            ..AppSettings::default()
        };
        settings
            .last_mission_terminal_ids
            .insert("mission".into(), "session".into());
        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(value["missionRailOpen"], false);
        assert_eq!(value["missionRailWidth"], 412.);
        assert_eq!(value["lastMissionTerminalIds"]["mission"], "session");

        settings.mission_rail_width = f32::NAN;
        settings
            .last_mission_terminal_ids
            .insert(String::new(), String::new());
        settings.normalize();
        assert_eq!(settings.mission_rail_width, MISSION_RAIL_DEFAULT);
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
        assert!(!loaded.sidebar_active_only);
        assert!(loaded.sidebar_projects_open);
        assert!(loaded.sidebar_chats_open);

        assert_eq!(normalize_terminal_font_size(0), TERMINAL_FONT_SIZE_DEFAULT);
        assert_eq!(normalize_terminal_font_size(1), TERMINAL_FONT_SIZE_MIN);
    }

    #[test]
    fn terminal_settings_default_round_trip_and_normalize() {
        let old: AppSettings =
            serde_json::from_str(r#"{"terminalFontSize":14,"terminalCursorStyle":"bar"}"#).unwrap();
        assert_eq!(old.terminal_font_size, 14);
        assert_eq!(old.terminal_line_height, 1.4);
        assert!(!old.terminal_cursor_blink);
        assert_eq!(old.terminal_scrollback_lines, 10_000);
        assert!(!old.terminal_copy_on_select);

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        let settings = AppSettings {
            terminal_font_size: 8,
            terminal_line_height: 1.1,
            terminal_cursor_blink: true,
            terminal_scrollback_lines: 50_000,
            terminal_copy_on_select: true,
            ..AppSettings::default()
        };
        settings.save(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["terminalLineHeight"], serde_json::json!(1.1));
        assert_eq!(json["terminalCursorBlink"], true);
        assert_eq!(json["terminalScrollbackLines"], 50_000);
        assert_eq!(json["terminalCopyOnSelect"], true);
        assert_eq!(AppSettings::load(&path).unwrap(), settings);

        fs::write(
            &path,
            r#"{"terminalFontSize":40,"terminalLineHeight":1.94,"terminalScrollbackLines":7000}"#,
        )
        .unwrap();
        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.terminal_font_size, 32);
        assert_eq!(loaded.terminal_line_height, 1.6);
        assert_eq!(loaded.terminal_scrollback_lines, 7_000);

        assert_eq!(normalize_terminal_font_size(7), 8);
        assert_eq!(normalize_terminal_font_size(20), 20);
        assert_eq!(normalize_terminal_line_height(1.24), 1.2);
        assert_eq!(normalize_terminal_line_height(1.25), 1.3);
        assert_eq!(normalize_terminal_line_height(0.5), 1.0);
        assert_eq!(normalize_terminal_line_height(f32::NAN), 1.4);
        assert_eq!(normalize_terminal_scrollback(0), 1_000);
        assert_eq!(normalize_terminal_scrollback(1_000_000), 50_000);
        for preset in TERMINAL_SCROLLBACK_PRESETS {
            assert_eq!(normalize_terminal_scrollback(preset), preset);
        }
    }

    #[test]
    fn a_skipped_update_version_survives_a_restart() {
        let settings = AppSettings {
            skipped_update_version: Some("0.19.5".into()),
            ..AppSettings::default()
        };
        let saved = serde_json::to_string(&settings).unwrap();
        let loaded: AppSettings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.skipped_update_version.as_deref(), Some("0.19.5"));
        let older: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(older.skipped_update_version, None);
    }

    #[test]
    fn persisted_settings_use_the_current_contract() {
        let value = serde_json::to_value(AppSettings::default()).unwrap();
        assert_eq!(value["appTheme"], "auto");
        assert_eq!(value["lightAppTheme"], "runner-light");
        assert_eq!(value["darkAppTheme"], "carbon");
        assert_eq!(value["lightTerminalTheme"], "rose-pine-dawn");
        assert_eq!(value["darkTerminalTheme"], "runner");
        assert!(value.get("terminalTheme").is_none());
        assert_eq!(
            value["terminalFontFamily"],
            crate::terminal::fonts::BUNDLED_FONT
        );
        assert_eq!(value["defaultCrewId"], "");
        assert_eq!(value["defaultWorkingDir"], "");
        assert_eq!(value["resumeOnLaunch"], true);
        assert_eq!(value["automaticallyCheckForUpdates"], true);
        assert!(value["skippedUpdateVersion"].is_null());
        assert_eq!(value["defaultRuntime"], "");
        assert_eq!(value["initializedCommandInstall"], false);
        assert_eq!(value["runnerSkillEnabled"], true);
        assert_eq!(value["keymapOverrides"], serde_json::json!({}));
    }

    #[test]
    fn legacy_theme_settings_migrate_without_resetting_other_preferences() {
        for json in [
            r#"{"terminalTheme":"runner","lightAppTheme":"codex","sidebarWidth":376}"#,
            r#"{"sidebarWidth":376}"#,
        ] {
            let settings: AppSettings = serde_json::from_str(json).unwrap();
            assert_eq!(settings.dark_terminal_theme, DarkTerminalTheme::Runner);
            assert_eq!(
                settings.light_terminal_theme,
                LightTerminalTheme::RosePineDawn
            );
            assert_eq!(settings.light_app_theme, LightTheme::RunnerLight);
            assert_eq!(settings.sidebar_width, 376.);
        }
        let settings: AppSettings =
            serde_json::from_str(r#"{"lightAppTheme":"catppuccin-latte"}"#).unwrap();
        assert_eq!(settings.light_app_theme, LightTheme::CatppuccinLatte);
    }

    #[test]
    fn legacy_terminal_theme_becomes_the_dark_pick_and_is_rewritten_on_save() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        fs::write(&path, r#"{"terminalTheme":"runner","sidebarWidth":376}"#).unwrap();

        let loaded = AppSettings::load(&path).unwrap();
        assert_eq!(loaded.dark_terminal_theme, DarkTerminalTheme::Runner);
        assert_eq!(
            loaded.light_terminal_theme,
            LightTerminalTheme::RosePineDawn
        );
        assert_eq!(loaded.sidebar_width, 376.);

        loaded.save(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["darkTerminalTheme"], "runner");
        assert_eq!(saved["lightTerminalTheme"], "rose-pine-dawn");
        assert!(saved.get("terminalTheme").is_none());

        let settings: AppSettings =
            serde_json::from_str(r#"{"terminalTheme":"catppuccin-mocha"}"#).unwrap();
        assert_eq!(
            settings.dark_terminal_theme,
            DarkTerminalTheme::CatppuccinMocha
        );
        assert_eq!(
            settings.light_terminal_theme,
            LightTerminalTheme::RosePineDawn
        );
    }

    #[test]
    fn retired_and_unknown_terminal_themes_load_as_defaults_without_resetting_settings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ui-settings.json");
        // "match-app" and "monokai" shipped as choices through 0.8.6; both
        // land on the defaults like any unknown key.
        for (light, dark) in [
            ("match-app", "match-app"),
            ("monokai", "monokai"),
            ("unknown-theme", "unknown-theme"),
        ] {
            fs::write(
                &path,
                format!(
                    r#"{{"lightTerminalTheme":"{light}","darkTerminalTheme":"{dark}","sidebarWidth":376}}"#
                ),
            )
            .unwrap();

            let loaded = AppSettings::load(&path).unwrap();
            assert_eq!(
                loaded.light_terminal_theme,
                LightTerminalTheme::RosePineDawn,
                "{light}"
            );
            assert_eq!(
                loaded.dark_terminal_theme,
                DarkTerminalTheme::Runner,
                "{dark}"
            );
            assert_eq!(loaded.sidebar_width, 376.);
        }
        for key in ["match-app", "monokai", "runner-dark", "runner-light"] {
            let settings: AppSettings =
                serde_json::from_str(&format!(r#"{{"terminalTheme":"{key}"}}"#)).unwrap();
            assert_eq!(settings, AppSettings::default(), "{key}");
        }
        let settings: AppSettings = serde_json::from_str(r#"{"sidebarWidth":376}"#).unwrap();
        assert_eq!(settings.light_terminal_theme, LightTerminalTheme::default());
        assert_eq!(settings.dark_terminal_theme, DarkTerminalTheme::default());
    }

    #[test]
    fn per_mode_terminal_themes_round_trip_every_palette() {
        for (light, dark) in LightTerminalTheme::ALL
            .into_iter()
            .zip(DarkTerminalTheme::ALL)
        {
            let json = serde_json::to_value(AppSettings {
                light_terminal_theme: light,
                dark_terminal_theme: dark,
                ..AppSettings::default()
            })
            .unwrap();
            assert_eq!(json["lightTerminalTheme"], light.key());
            assert_eq!(json["darkTerminalTheme"], dark.key());
            assert!(json.get("terminalTheme").is_none());
            let reloaded: AppSettings = serde_json::from_value(json).unwrap();
            assert_eq!(reloaded.light_terminal_theme, light);
            assert_eq!(reloaded.dark_terminal_theme, dark);
            assert_eq!(LightTerminalTheme::parse(light.key()), Some(light));
            assert_eq!(DarkTerminalTheme::parse(dark.key()), Some(dark));
        }
        assert_eq!(LightTerminalTheme::parse("match-app"), None);
        assert_eq!(DarkTerminalTheme::parse("runner-dark"), None);
    }

    #[test]
    fn terminal_palette_follows_the_resolved_mode() {
        let settings = AppSettings {
            light_terminal_theme: LightTerminalTheme::Runner,
            dark_terminal_theme: DarkTerminalTheme::CatppuccinMocha,
            ..AppSettings::default()
        };
        for variant in [ThemeVariant::RunnerLight, ThemeVariant::CatppuccinLatte] {
            assert_eq!(
                terminal_palette(&settings, variant),
                palette::RUNNER,
                "{variant:?}"
            );
        }
        for variant in [ThemeVariant::Carbon, ThemeVariant::CatppuccinMocha] {
            assert_eq!(
                terminal_palette(&settings, variant),
                palette::CATPPUCCIN_MOCHA,
                "{variant:?}"
            );
        }
        assert_eq!(
            terminal_palette(&AppSettings::default(), ThemeVariant::RunnerLight),
            palette::ROSE_PINE_DAWN
        );
        assert_eq!(
            terminal_palette(&AppSettings::default(), ThemeVariant::Carbon),
            palette::RUNNER
        );
        for (light, dark) in LightTerminalTheme::ALL
            .into_iter()
            .zip(DarkTerminalTheme::ALL)
        {
            assert_eq!(
                light.palette(),
                LightTerminalTheme::parse(light.key()).unwrap().palette()
            );
            assert_eq!(
                DarkTerminalTheme::parse(light.key()).unwrap().palette(),
                light.palette(),
                "{light:?}"
            );
            assert_eq!(
                LightTerminalTheme::parse(dark.key()).unwrap().palette(),
                dark.palette(),
                "{dark:?}"
            );
        }
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
    fn terminal_fonts_migrate_each_legacy_choice() {
        assert_eq!(
            AppSettings::default().terminal_font_family.family(),
            crate::terminal::fonts::BUNDLED_FONT
        );
        for (saved, expected) in [
            ("JetBrains Mono", crate::terminal::fonts::BUNDLED_FONT),
            ("Meslo Nerd Font", crate::terminal::fonts::BUNDLED_FONT),
            ("Menlo", runner_app::theme::SYSTEM_MONOSPACE_FONT),
            ("System default", "System default"),
            ("Monaco", "Monaco"),
            ("SF Mono", "SF Mono"),
            ("Fira Code", "Fira Code"),
        ] {
            let parsed: TerminalFontFamily =
                serde_json::from_value(serde_json::json!(saved)).unwrap();
            assert_eq!(parsed.family(), expected, "{saved}");
        }
    }

    #[test]
    fn missing_terminal_font_falls_back_without_overwriting_saved_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-settings.json");
        fs::write(&path, r#"{"terminalFontFamily":"Runner Missing Font 931"}"#).unwrap();
        let settings = AppSettings::load(&path).unwrap();
        assert_eq!(
            settings
                .terminal_font_family
                .resolved_font(false)
                .family
                .as_ref(),
            crate::terminal::fonts::BUNDLED_FONT
        );
        settings.save(&path).unwrap();
        assert_eq!(
            AppSettings::load(&path)
                .unwrap()
                .terminal_font_family
                .family(),
            "Runner Missing Font 931"
        );
    }

    #[test]
    fn terminal_fonts_use_bundled_icons_and_platform_fallbacks() {
        let family = TerminalFontFamily::new("SF Mono");
        let font = family.resolved_font(true);
        assert_eq!(font.family.as_ref(), "SF Mono");
        let expected = std::iter::once(crate::terminal::fonts::BUNDLED_FONT)
            .chain(
                runner_app::platform_fonts::TERMINAL_FONT_FALLBACKS
                    .iter()
                    .copied(),
            )
            .collect::<Vec<_>>();
        assert_eq!(font.fallbacks.unwrap().fallback_list(), expected);
    }

    #[test]
    fn agent_enablement_preserves_shipped_defaults_and_explicit_overrides() {
        let default_enabled = runner_core::protocol::runtime_metadata::runtime_default_enabled;
        let mut settings = AppSettings::default();
        assert_eq!(
            Runtime::ALL
                .into_iter()
                .filter(|runtime| settings.is_agent_enabled(*runtime, default_enabled(*runtime)))
                .collect::<Vec<_>>(),
            [
                Runtime::Codex,
                Runtime::ClaudeCode,
                Runtime::Antigravity,
                Runtime::Pi,
                Runtime::Copilot,
                Runtime::Cursor,
                Runtime::Grok,
                Runtime::OpenCode,
            ]
        );
        settings.disabled_agents.extend([
            "codex".into(),
            "copilot".into(),
            "cursor".into(),
            "grok".into(),
            "opencode".into(),
        ]);
        settings.enabled_agents.insert("trae".into());
        let settings: AppSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        for runtime in [
            Runtime::Codex,
            Runtime::Copilot,
            Runtime::Cursor,
            Runtime::Grok,
            Runtime::OpenCode,
        ] {
            assert!(!settings.is_agent_enabled(runtime, default_enabled(runtime)));
        }
        assert!(settings.is_agent_enabled(Runtime::Trae, default_enabled(Runtime::Trae)));
    }

    #[test]
    fn usage_runtimes_are_the_enabled_runtimes_that_report_usage() {
        let mut settings = AppSettings::default();
        let mut defaults = settings.usage_runtimes();
        defaults.sort_by_key(|runtime| runtime.key());
        assert_eq!(defaults, [Runtime::ClaudeCode, Runtime::Codex, Runtime::Muse]);
        assert!(
            !settings.model_runtimes().contains(&Runtime::Muse),
            "Muse reports usage without discovering models"
        );
        settings.disabled_agents.insert(Runtime::Muse.key().into());
        assert!(!settings.usage_runtimes().contains(&Runtime::Muse));
        assert!(settings.usage_runtimes().contains(&Runtime::Codex));
    }

    #[test]
    fn usage_badges_include_muse_by_default_and_allow_it_to_be_hidden() {
        let mut settings = AppSettings::default();
        assert!(settings.usage.badge(Runtime::Muse));
        settings.usage.muse = false;
        assert!(!settings.usage.badge(Runtime::Muse));
    }

    #[test]
    fn cursor_and_grok_model_discovery_follow_defaults_and_explicit_disablement() {
        for runtime in [Runtime::Cursor, Runtime::Grok, Runtime::OpenCode] {
            let mut settings = AppSettings::default();
            assert!(settings.model_runtimes().contains(&runtime));
            settings.disabled_agents.insert(runtime.key().into());
            assert!(!settings.model_runtimes().contains(&runtime));
        }
    }

    #[test]
    fn app_font_is_inter_with_the_platform_fallback_chain() {
        let inter = app_font();
        assert_eq!(inter.family.as_ref(), "Inter");
        #[cfg(windows)]
        assert_eq!(
            inter.fallbacks.unwrap().fallback_list(),
            ["Segoe UI", "Microsoft YaHei"]
        );
        #[cfg(target_os = "linux")]
        assert_eq!(
            inter.fallbacks.clone().unwrap().fallback_list(),
            runner_app::platform_fonts::APP_FONT_FALLBACKS
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            inter.fallbacks.unwrap().fallback_list(),
            [
                "Inter Variable",
                "Segoe UI",
                "Microsoft YaHei",
                "sans-serif"
            ]
        );
    }

    #[test]
    fn legacy_app_font_choices_still_load() {
        for legacy in ["Inter", "Geist", "Roboto", "System UI"] {
            let json = format!(r#"{{"appFontFamily":"{legacy}","appZoom":1.25}}"#);
            let settings: AppSettings = serde_json::from_str(&json).unwrap();
            assert_eq!(settings.app_zoom, 1.25, "{legacy}");
        }
        let value = serde_json::to_value(AppSettings::default()).unwrap();
        assert!(value.get("appFontFamily").is_none());
    }

    #[test]
    fn settings_without_initialized_mcp_clients_decode_to_empty() {
        let settings: AppSettings = serde_json::from_str(r#"{"appZoom":1.25}"#).unwrap();
        assert!(settings.initialized_mcp_clients.is_empty());
        let settings: AppSettings =
            serde_json::from_str(r#"{"initializedMcpClients":["codex"]}"#).unwrap();
        assert_eq!(
            settings.initialized_mcp_clients,
            ["codex".to_owned()].into_iter().collect()
        );
        assert!(!settings.mcp_registrations_removed);
    }

    #[test]
    fn settings_without_initialized_skill_roots_decode_to_empty() {
        let settings: AppSettings = serde_json::from_str(r#"{"appZoom":1.25}"#).unwrap();
        assert!(settings.initialized_skill_roots.is_empty());
        let settings: AppSettings =
            serde_json::from_str(r#"{"initializedSkillRoots":[".agents/skills"]}"#).unwrap();
        assert_eq!(
            settings.initialized_skill_roots,
            [".agents/skills".to_owned()].into_iter().collect()
        );
        assert!(settings.runner_skill_enabled);
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn automatic_downloads_default_on_and_persist_independently_of_checks() {
        let mut settings: AppSettings =
            serde_json::from_str(r#"{"automaticallyCheckForUpdates":false}"#).unwrap();
        assert!(settings.automatically_download_updates);
        settings.automatically_download_updates = false;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        settings.save(&path).unwrap();
        let loaded = AppSettings::load(&path).unwrap();
        assert!(!loaded.automatically_download_updates);
        assert!(!loaded.automatically_check_for_updates);
    }
}

#[cfg(test)]
mod quit_settings_tests {
    use super::*;
    use runner_app::lifecycle::{QuitBehavior, QuitChoice};

    #[test]
    fn quit_settings_default_and_round_trip_without_migration() {
        let old: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.quit_behavior, QuitBehavior::Ask);
        assert_eq!(old.last_quit_choice, QuitChoice::Keep);
        let unknown: AppSettings = serde_json::from_str(r#"{"quitBehavior":"future"}"#).unwrap();
        assert_eq!(unknown.quit_behavior, QuitBehavior::Ask);
        let temp = tempfile::tempdir().unwrap();
        for behavior in QuitBehavior::ALL {
            let settings = AppSettings {
                quit_behavior: behavior,
                last_quit_choice: QuitChoice::Stop,
                ..AppSettings::default()
            };
            let path = temp.path().join("ui-settings.json");
            settings.save(&path).unwrap();
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(value["quitBehavior"], behavior.key());
            assert_eq!(AppSettings::load(&path).unwrap(), settings);
        }
    }
}

#[cfg(test)]
mod material_tests {
    use super::*;
    use runner_app::appearance::WindowMaterial;

    #[test]
    fn existing_settings_get_the_platform_material_and_both_choices_round_trip() {
        let old: AppSettings = serde_json::from_str(r#"{"appZoom":1.25}"#).unwrap();
        assert_eq!(old.window_material, WindowMaterial::default());
        for material in [WindowMaterial::Glass, WindowMaterial::Solid] {
            let settings = AppSettings {
                window_material: material,
                ..old.clone()
            };
            let encoded = serde_json::to_string(&settings).unwrap();
            assert_eq!(
                serde_json::from_str::<AppSettings>(&encoded).unwrap(),
                settings
            );
        }
    }
}

#[cfg(test)]
mod notification_tests {
    use super::*;

    #[test]
    fn old_settings_load_notification_defaults_and_round_trip_switches() {
        let mut settings: AppSettings = serde_json::from_str(r#"{"appZoom":1.0}"#).unwrap();
        assert_eq!(settings.notifications, NotificationSettings::default());
        assert!(settings.notifications.chats.input);
        assert!(settings.notifications.chats.finished);
        assert!(settings.notifications.chats.failed);
        assert!(!settings.notifications.missions.question);
        assert!(!settings.notifications.missions.input);
        assert!(!settings.notifications.missions.idle);
        assert!(!settings.notifications.missions.failed);
        settings.notifications.chats.input = false;
        settings.notifications.chats.finished = false;
        settings.notifications.missions.input = true;
        settings.notifications.missions.idle = true;
        let saved = serde_json::to_string(&settings).unwrap();
        let json: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert!(json["notifications"]["missions"].get("enabled").is_none());
        let loaded: AppSettings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.notifications, settings.notifications);
    }
}

#[test]
fn usage_preferences_defaults_roundtrip_and_fetching_rules() {
    let defaults: AppSettings = serde_json::from_str("{}").unwrap();
    assert!(
        defaults.usage.sidebar
            && defaults.usage.codex
            && defaults.usage.claude_code
            && defaults.usage.antigravity
            && defaults.usage.five_hour
    );
    let legacy: UsageSettings = serde_json::from_str(
        r#"{"sidebar":true,"codex":false,"claudeCode":true,"fiveHour":false}"#,
    )
    .unwrap();
    assert!(legacy.antigravity);
    assert!(!legacy.codex);
    assert!(!legacy.five_hour);
    assert!(legacy.badge(Runtime::Antigravity));
    assert!(defaults.notifications.usage.near_limit);
    assert_eq!(defaults.notifications.usage.threshold, 80);
    let mut settings = defaults;
    settings.usage.sidebar = false;
    settings.usage.codex = false;
    settings.usage.antigravity = false;
    settings.usage.five_hour = false;
    settings.notifications.usage.near_limit = false;
    settings.notifications.usage.threshold = 60;
    let saved: AppSettings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert_eq!(saved, settings);
    for badges in [false, true] {
        for agent in [false, true] {
            for alert in [false, true] {
                let usage = UsageSettings {
                    sidebar: badges,
                    codex: agent,
                    claude_code: agent,
                    antigravity: agent,
                    ..Default::default()
                };
                for runtime in [Runtime::Codex, Runtime::ClaudeCode, Runtime::Antigravity] {
                    assert_eq!(usage.checks(runtime, alert), (badges && agent) || alert);
                }
            }
        }
    }
}
