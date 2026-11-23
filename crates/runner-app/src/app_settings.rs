use runner_core::protocol::model::Runtime;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui::{font, Font, FontFallbacks};
use runner_terminal::palette::TerminalPalette;
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
pub const CHAT_PANEL_DEFAULT: f32 = 300.;
pub const MISSION_RAIL_MIN: f32 = 200.;
pub const MISSION_RAIL_MAX: f32 = 480.;
pub const MISSION_RAIL_DEFAULT: f32 = 300.;
pub const FILE_VIEWER_MIN: f32 = 200.;
pub const FILE_VIEWER_MAX: f32 = 1200.;
pub const FILE_VIEWER_DEFAULT: f32 = 460.;
pub const FILE_TREE_MIN: f32 = 160.;
pub const FILE_TREE_MAX: f32 = 480.;
pub const FILE_TREE_DEFAULT: f32 = 240.;

/// The terminal palette for the resolved app variant.
pub fn terminal_palette(_settings: &AppSettings, variant: ThemeVariant) -> TerminalPalette {
    variant.terminal_palette()
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

/// How changed files render in Changes and Files' in-place diff; remembered in
/// `ui-settings.json` and shared between both surfaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiffViewMode {
    Split,
    #[default]
    #[serde(other)]
    Unified,
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
    pub five_hour: bool,
}
impl Default for UsageSettings {
    fn default() -> Self {
        Self {
            sidebar: true,
            codex: true,
            claude_code: true,
            antigravity: true,
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
    pub chat_panel_width: f32,
    pub mission_rail_width: f32,
    /// The unified viewer's width in a side panel widened by an open file or diff, beside the right rail.
    pub file_viewer_width: f32,
    /// The right rail's width on the outer edge of a panel widened by an open file or diff.
    pub file_tree_width: f32,
    /// Settings → Side panel → Panel: off hides every tab's panel and its
    /// toggle, keeping the view switches below.
    pub show_side_panel: bool,
    /// Whether the activity bar offers Files and Changes.
    pub files_view: bool,
    pub changes_view: bool,
    pub changes_tree_view: bool,
    pub changes_diff_mode: DiffViewMode,
    #[serde(alias = "changes_bases")]
    pub changes_bases: BTreeMap<String, String>,
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
            chat_panel_width: CHAT_PANEL_DEFAULT,
            mission_rail_width: MISSION_RAIL_DEFAULT,
            file_viewer_width: FILE_VIEWER_DEFAULT,
            file_tree_width: FILE_TREE_DEFAULT,
            show_side_panel: true,
            files_view: true,
            changes_view: true,
            changes_tree_view: false,
            changes_diff_mode: DiffViewMode::Unified,
            changes_bases: BTreeMap::new(),
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
        self.changes_bases = std::mem::take(&mut self.changes_bases)
            .into_iter()
            .filter(|(key, value)| !key.trim().is_empty() && !value.trim().is_empty())
            .map(|(key, value)| {
                (
                    normalize_changes_base_entry_key(&key),
                    value.trim().to_owned(),
                )
            })
            .collect();
        self.default_crew_id = self.default_crew_id.trim().to_owned();
        self.default_working_dir = self.default_working_dir.trim().to_owned();
        self.default_runtime = self.default_runtime.trim().to_owned();
        self.sidebar_collapsed_projects =
            normalize_agent_set(std::mem::take(&mut self.sidebar_collapsed_projects));
        self.disabled_agents = normalize_agent_set(std::mem::take(&mut self.disabled_agents));
        self.enabled_agents = normalize_agent_set(std::mem::take(&mut self.enabled_agents));
        keymap::normalize_overrides(&mut self.keymap_overrides);
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

    pub fn changes_base(&self, checkout_top: &Path, branch: Option<&str>) -> Option<&str> {
        let key = changes_base_key(checkout_top, branch);
        if let Some(value) = self.changes_bases.get(&key) {
            return Some(value.as_str());
        }
        self.changes_bases.iter().find_map(|(existing, value)| {
            changes_base_keys_match(&normalize_changes_base_entry_key(existing), &key)
                .then_some(value.as_str())
        })
    }

    pub fn set_changes_base(
        &mut self,
        checkout_top: &Path,
        branch: Option<&str>,
        base: Option<&str>,
    ) -> bool {
        let key = changes_base_key(checkout_top, branch);
        let stale_keys: Vec<String> = self
            .changes_bases
            .keys()
            .filter(|existing| {
                existing.as_str() != key
                    && changes_base_keys_match(&normalize_changes_base_entry_key(existing), &key)
            })
            .cloned()
            .collect();
        match base.map(str::trim).filter(|b| !b.is_empty()) {
            None => {
                let mut removed = self.changes_bases.remove(&key).is_some();
                for stale in stale_keys {
                    removed |= self.changes_bases.remove(&stale).is_some();
                }
                removed
            }
            Some(chosen) => {
                let mut changed = false;
                for stale in stale_keys {
                    changed |= self.changes_bases.remove(&stale).is_some();
                }
                match self.changes_bases.get(&key) {
                    Some(existing) if existing == chosen => changed,
                    _ => {
                        self.changes_bases.insert(key, chosen.to_owned());
                        true
                    }
                }
            }
        }
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

pub fn changes_base_key(checkout_top: &Path, branch: Option<&str>) -> String {
    let normalized = runner_core::app_paths::normalize_checkout_path(checkout_top);
    match branch.map(str::trim).filter(|b| !b.is_empty()) {
        Some(branch) => format!("{}::{branch}", normalized.to_string_lossy()),
        None => normalized.to_string_lossy().into_owned(),
    }
}

fn normalize_changes_base_entry_key(raw_key: &str) -> String {
    match raw_key.trim().split_once("::") {
        Some((top, branch)) => changes_base_key(Path::new(top), Some(branch)),
        None => changes_base_key(Path::new(raw_key.trim()), None),
    }
}

fn changes_base_keys_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    #[cfg(windows)]
    {
        match (a.split_once("::"), b.split_once("::")) {
            (Some((top_a, branch_a)), Some((top_b, branch_b))) => {
                top_a.eq_ignore_ascii_case(top_b) && branch_a == branch_b
            }
            (None, None) => a.eq_ignore_ascii_case(b),
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
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
#[path = "app_settings_tests.rs"]
mod tests;

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

#[test]
fn changes_diff_mode_defaults_to_unified_and_round_trips_split() {
    let defaults: AppSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(defaults.changes_diff_mode, DiffViewMode::Unified);

    let mut settings = defaults;
    settings.changes_diff_mode = DiffViewMode::Split;
    let json = serde_json::to_value(&settings).unwrap();
    assert_eq!(json["changesDiffMode"], "split");

    let reloaded: AppSettings = serde_json::from_value(json).unwrap();
    assert_eq!(reloaded.changes_diff_mode, DiffViewMode::Split);

    let unknown: AppSettings = serde_json::from_str(r#"{"changesDiffMode":"future"}"#).unwrap();
    assert_eq!(unknown.changes_diff_mode, DiffViewMode::Unified);
}

#[test]
fn changes_bases_keys_by_checkout_and_branch_or_checkout_alone_and_auto_removes() {
    let temp = tempfile::tempdir().unwrap();
    let checkout_dir = temp.path().join("repo/.worktrees/feat-child");
    fs::create_dir_all(&checkout_dir).unwrap();
    let checkout = checkout_dir.as_path();
    let canonical = runner_core::app_paths::normalize_checkout_path(checkout);
    assert_eq!(
        changes_base_key(checkout, Some("feat/child")),
        format!("{}::feat/child", canonical.to_string_lossy())
    );
    assert_eq!(
        changes_base_key(checkout, None),
        canonical.to_string_lossy()
    );

    let mut settings = AppSettings::default();
    assert_eq!(settings.changes_base(checkout, Some("feat/child")), None);
    assert!(settings.set_changes_base(checkout, Some("feat/child"), Some("feat/umbrella")));
    assert!(!settings.set_changes_base(&canonical, Some("feat/child"), Some("feat/umbrella")));
    assert!(settings.set_changes_base(checkout, Some("other"), Some("origin/main")));
    assert!(settings.set_changes_base(checkout, None, Some("upstream/main")));

    assert_eq!(
        settings.changes_base(&canonical, Some("feat/child")),
        Some("feat/umbrella")
    );
    assert_eq!(
        settings.changes_base(&canonical, Some("other")),
        Some("origin/main")
    );
    assert_eq!(
        settings.changes_base(&canonical, None),
        Some("upstream/main")
    );

    // Picking Auto (`None`) removes only that branch's entry regardless of path spelling.
    assert!(settings.set_changes_base(&canonical, Some("feat/child"), None));
    assert!(!settings.set_changes_base(checkout, Some("feat/child"), None));
    assert_eq!(settings.changes_base(checkout, Some("feat/child")), None);
    assert_eq!(
        settings.changes_base(checkout, Some("other")),
        Some("origin/main")
    );
    assert_eq!(settings.changes_base(checkout, None), Some("upstream/main"));

    // Round-trips JSON and accepts snake_case alias.
    let saved: AppSettings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert_eq!(saved.changes_bases, settings.changes_bases);
    let aliased: AppSettings =
        serde_json::from_str(r#"{"changes_bases":{"/repo::feat/x":"feat/u"}}"#).unwrap();
    assert_eq!(
        aliased.changes_base(Path::new("/repo"), Some("feat/x")),
        Some("feat/u")
    );
}
