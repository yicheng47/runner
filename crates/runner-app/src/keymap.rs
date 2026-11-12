use std::collections::BTreeMap;

use gpui::{App, KeyBinding, Keystroke};
use serde::{Deserialize, Deserializer, Serialize};

#[cfg(target_os = "macos")]
use crate::{Hide, HideOthers, Minimize, Quit, QuitAndStopSessions};

use crate::{CloseFileTab, FindInFile, NextFileTab, PreviousFileTab};
use crate::{
    CloseTab, CloseWindow, CommandPalette, ConfirmStartChat, Copy, FindInTerminal, FocusNextPane,
    FocusPreviousPane, MissionTabNext, MissionTabPrevious, NavigateNextPage, NavigatePreviousPage,
    NewMission, NewTab, NewTerminal, NewWindow, OpenSettings, Paste, ResumeFocusedSession,
    SearchFiles, SelectTab1, SelectTab2, SelectTab3, SelectTab4, SelectTab5, SelectTab6,
    SelectTab7, SelectTab8, SelectTab9, SplitPaneDown, SplitPaneRight, StopFocusedSession,
    SwitchSidebarView, ToggleFullscreen, ToggleMaximizePane, ToggleSidebar, ToggleTerminalDrawer,
    ZoomIn, ZoomOut, ZoomReset,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeymapScope {
    Global,
    ChatSplit,
    Mission,
    Terminal,
}

impl KeymapScope {
    fn overlaps(self, other: Self) -> bool {
        self == other || self == Self::Global || other == Self::Global
    }

    fn context(self) -> Option<&'static str> {
        match self {
            Self::Global => None,
            Self::ChatSplit => Some("ChatSplit"),
            Self::Mission => Some("Mission"),
            Self::Terminal => Some("Terminal"),
        }
    }
}

/// Whether Runner's shortcuts or a focused terminal get bare Ctrl chords.
/// Only Windows and Linux offer the choice; macOS shortcuts use ⌘.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TerminalCtrlKeys {
    TerminalFirst,
    #[default]
    #[serde(other)]
    RunnerFirst,
}

#[cfg(any(windows, target_os = "linux"))]
impl TerminalCtrlKeys {
    pub(crate) const ALL: [Self; 2] = [Self::RunnerFirst, Self::TerminalFirst];

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::RunnerFirst => "keymap.terminal-ctrl-keys.runner-first",
            Self::TerminalFirst => "keymap.terminal-ctrl-keys.terminal-first",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeyCombo {
    pub(crate) meta: bool,
    pub(crate) ctrl: bool,
    pub(crate) alt: bool,
    pub(crate) shift: bool,
    pub(crate) code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) shift_optional: bool,
}

const fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KeymapEntry {
    pub(crate) id: &'static str,
    pub(crate) title_key: &'static str,
    pub(crate) description_key: &'static str,
    pub(crate) scope: KeymapScope,
    pub(crate) default: KeyCombo,
    pub(crate) fixed: bool,
}

impl KeymapEntry {
    pub(crate) fn title(&self) -> &'static str {
        runner_app::i18n::text(self.title_key)
    }
    pub(crate) fn description(&self) -> &'static str {
        runner_app::i18n::text(self.description_key).trim()
    }
}

fn key_combo(
    code: &str,
    meta: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
    label: Option<&str>,
    shift_optional: bool,
) -> KeyCombo {
    KeyCombo {
        meta,
        ctrl,
        alt,
        shift,
        code: code.to_owned(),
        label: label.map(str::to_owned),
        shift_optional,
    }
}

pub(crate) fn platform_default(macos: &str) -> String {
    #[cfg(any(windows, target_os = "linux"))]
    {
        windows_default(macos)
    }
    #[cfg(target_os = "macos")]
    {
        macos.to_owned()
    }
}

#[cfg(any(windows, target_os = "linux", test))]
fn windows_default(macos: &str) -> String {
    match macos {
        "ctrl-cmd-f" => "f11".into(),
        // Ctrl+F still reaches the shell.
        "cmd-f" => "ctrl-shift-f".into(),
        "cmd-w" => "ctrl-shift-w".into(),
        "cmd-t" => "ctrl-shift-t".into(),
        // Ctrl+D, Ctrl+K and Ctrl+S are terminal editing keys.
        "cmd-d" => "ctrl-shift-d".into(),
        "shift-cmd-d" => "alt-shift-d".into(),
        "cmd-k" => "ctrl-shift-p".into(),
        "cmd-s" => "ctrl-b".into(),
        "shift-cmd-w" => "alt-f4".into(),
        _ => macos.replace("cmd-", "ctrl-"),
    }
}

fn default_combo(macos: &str, label: Option<&str>, shift_optional: bool) -> KeyCombo {
    let keystroke = Keystroke::parse(&platform_default(macos)).expect("valid default binding");
    let (code, implied_shift) = code_from_gpui_key(&keystroke.key).expect("valid default key");
    key_combo(
        &code,
        keystroke.modifiers.platform,
        keystroke.modifiers.control,
        keystroke.modifiers.alt,
        keystroke.modifiers.shift || implied_shift,
        label,
        shift_optional,
    )
}

pub(crate) fn fixed_shortcut(binding: &str) -> String {
    format_combo(&default_combo(binding, None, false))
}

pub(crate) fn entries() -> &'static [KeymapEntry] {
    static KEYMAP: std::sync::OnceLock<Vec<KeymapEntry>> = std::sync::OnceLock::new();
    KEYMAP.get_or_init(|| {
        vec![
            KeymapEntry {
                id: "new-window",
                title_key: "keymap.new-window.title",
                description_key: "keymap.new-window.description",
                scope: KeymapScope::Global,
                default: default_combo("shift-cmd-n", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "new-chat",
                title_key: "keymap.new-chat.title",
                description_key: "keymap.new-chat.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-n", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "new-terminal",
                title_key: "keymap.new-terminal.title",
                description_key: "keymap.new-terminal.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-t", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "new-mission",
                title_key: "keymap.new-mission.title",
                description_key: "keymap.new-mission.description",
                scope: KeymapScope::Global,
                default: default_combo("shift-cmd-m", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "command-palette",
                title_key: "keymap.command-palette.title",
                description_key: "keymap.command-palette.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-k", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "search-files",
                title_key: "keymap.search-files.title",
                description_key: "keymap.search-files.description",
                scope: KeymapScope::Global,
                default: default_combo("shift-cmd-o", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "toggle-sidebar",
                title_key: "keymap.toggle-sidebar.title",
                description_key: "keymap.toggle-sidebar.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-s", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "switch-sidebar-view",
                title_key: "keymap.switch-sidebar-view.title",
                description_key: "keymap.switch-sidebar-view.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-.", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "toggle-terminal-drawer",
                title_key: "keymap.toggle-terminal-drawer.title",
                description_key: "keymap.toggle-terminal-drawer.description",
                scope: KeymapScope::Mission,
                default: default_combo("alt-f12", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "find-in-terminal",
                title_key: "keymap.find-in-terminal.title",
                description_key: "keymap.find-in-terminal.description",
                scope: KeymapScope::Terminal,
                default: default_combo("cmd-f", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "open-settings",
                title_key: "keymap.open-settings.title",
                description_key: "keymap.open-settings.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-,", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "page-previous",
                title_key: "keymap.page-previous.title",
                description_key: "keymap.page-previous.description",
                scope: KeymapScope::Global,
                default: default_combo("shift-cmd-[", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "page-next",
                title_key: "keymap.page-next.title",
                description_key: "keymap.page-next.description",
                scope: KeymapScope::Global,
                default: default_combo("shift-cmd-]", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "zoom-in",
                title_key: "keymap.zoom-in.title",
                description_key: "keymap.zoom-in.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-=", Some("+"), true),
                fixed: false,
            },
            KeymapEntry {
                id: "zoom-out",
                title_key: "keymap.zoom-out.title",
                description_key: "keymap.zoom-out.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd--", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "zoom-reset",
                title_key: "keymap.zoom-reset.title",
                description_key: "keymap.zoom-reset.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-0", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "select-tab-1",
                title_key: "keymap.select-tab-1.title",
                description_key: "keymap.select-tab-1.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-1", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "select-tab-2",
                title_key: "keymap.select-tab-2.title",
                description_key: "keymap.select-tab-2.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-2", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "select-tab-3",
                title_key: "keymap.select-tab-3.title",
                description_key: "keymap.select-tab-3.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-3", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "select-tab-4",
                title_key: "keymap.select-tab-4.title",
                description_key: "keymap.select-tab-4.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-4", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "select-tab-5",
                title_key: "keymap.select-tab-5.title",
                description_key: "keymap.select-tab-5.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-5", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "select-tab-6",
                title_key: "keymap.select-tab-6.title",
                description_key: "keymap.select-tab-6.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-6", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "select-tab-7",
                title_key: "keymap.select-tab-7.title",
                description_key: "keymap.select-tab-7.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-7", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "select-tab-8",
                title_key: "keymap.select-tab-8.title",
                description_key: "keymap.select-tab-8.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-8", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "select-tab-9",
                title_key: "keymap.select-tab-9.title",
                description_key: "keymap.select-tab-9.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-9", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "pane-previous",
                title_key: "keymap.pane-previous.title",
                description_key: "keymap.pane-previous.description",
                scope: KeymapScope::ChatSplit,
                default: default_combo("cmd-[", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "pane-next",
                title_key: "keymap.pane-next.title",
                description_key: "keymap.pane-next.description",
                scope: KeymapScope::ChatSplit,
                default: default_combo("cmd-]", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "split-pane-right",
                title_key: "keymap.split-pane-right.title",
                description_key: "keymap.split-pane-right.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-d", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "split-pane-down",
                title_key: "keymap.split-pane-down.title",
                description_key: "keymap.split-pane-down.description",
                scope: KeymapScope::Global,
                default: default_combo("shift-cmd-d", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "toggle-maximize-pane",
                title_key: "keymap.toggle-maximize-pane.title",
                description_key: "keymap.toggle-maximize-pane.description",
                scope: KeymapScope::Global,
                default: default_combo("shift-cmd-enter", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "close-pane",
                title_key: "keymap.close-pane.title",
                description_key: "keymap.close-pane.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-w", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "system-close-window",
                title_key: "keymap.system-close-window.title",
                description_key: "keymap.system-close-window.description",
                scope: KeymapScope::Global,
                default: default_combo("shift-cmd-w", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "stop-session",
                title_key: "keymap.stop-session.title",
                description_key: "keymap.stop-session.description",
                scope: KeymapScope::Global,
                default: default_combo("shift-cmd-x", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "resume-session",
                title_key: "keymap.resume-session.title",
                description_key: "keymap.resume-session.description",
                scope: KeymapScope::Global,
                default: default_combo("shift-cmd-r", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "mission-tab-previous",
                title_key: "keymap.mission-tab-previous.title",
                description_key: "keymap.mission-tab-previous.description",
                scope: KeymapScope::Mission,
                default: default_combo("cmd-[", None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "mission-tab-next",
                title_key: "keymap.mission-tab-next.title",
                description_key: "keymap.mission-tab-next.description",
                scope: KeymapScope::Mission,
                default: default_combo("cmd-]", None, false),
                fixed: false,
            },
        ]
    })
}

pub(crate) fn entry(id: &str) -> Option<&'static KeymapEntry> {
    entries().iter().find(|entry| entry.id == id)
}

pub(crate) fn is_tab_selection_entry(entry: &KeymapEntry) -> bool {
    entry.id.starts_with("select-tab-")
}

pub(crate) fn tab_selection_entries() -> impl Iterator<Item = &'static KeymapEntry> {
    entries()
        .iter()
        .filter(|entry| is_tab_selection_entry(entry))
}

fn reserved_entries() -> &'static [KeymapEntry] {
    static RESERVED: std::sync::OnceLock<Vec<KeymapEntry>> = std::sync::OnceLock::new();
    RESERVED.get_or_init(|| {
        vec![
            #[cfg(target_os = "macos")]
            KeymapEntry {
                id: "system-quit",
                title_key: "keymap.system-quit.title",
                description_key: "keymap.system-quit.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-q", None, false),
                fixed: true,
            },
            #[cfg(target_os = "macos")]
            KeymapEntry {
                id: "system-quit-stop",
                title_key: "keymap.system-quit-stop.title",
                description_key: "keymap.system-quit-stop.description",
                scope: KeymapScope::Global,
                default: default_combo("alt-cmd-q", None, false),
                fixed: true,
            },
            #[cfg(target_os = "macos")]
            KeymapEntry {
                id: "system-hide",
                title_key: "keymap.system-hide.title",
                description_key: "keymap.system-hide.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-h", None, false),
                fixed: true,
            },
            #[cfg(target_os = "macos")]
            KeymapEntry {
                id: "system-hide-others",
                title_key: "keymap.system-hide-others.title",
                description_key: "keymap.system-hide-others.description",
                scope: KeymapScope::Global,
                default: default_combo("alt-cmd-h", None, false),
                fixed: true,
            },
            // Ctrl+M is Return in a terminal, so only macOS binds Minimize.
            #[cfg(target_os = "macos")]
            KeymapEntry {
                id: "system-minimize",
                title_key: "keymap.system-minimize.title",
                description_key: "keymap.system-minimize.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-m", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "system-toggle-fullscreen",
                title_key: "keymap.system-toggle-fullscreen.title",
                description_key: "keymap.system-toggle-fullscreen.description",
                scope: KeymapScope::Global,
                default: default_combo("ctrl-cmd-f", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "copy",
                title_key: "keymap.copy.title",
                description_key: if cfg!(any(windows, target_os = "linux")) {
                    "keymap.copy.description-ctrl"
                } else {
                    "keymap.copy.description"
                },
                scope: KeymapScope::Terminal,
                default: default_combo("cmd-c", None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "system-paste",
                title_key: "keymap.system-paste.title",
                description_key: "keymap.system-paste.description",
                scope: KeymapScope::Global,
                default: default_combo("cmd-v", None, false),
                fixed: true,
            },
        ]
    })
}

/// System-owned keys the shortcuts page lists without letting them change.
pub(crate) fn display_only_entries() -> impl Iterator<Item = &'static KeymapEntry> {
    ["copy", "system-paste", "system-toggle-fullscreen"]
        .into_iter()
        .filter_map(|id| reserved_entries().iter().find(|entry| entry.id == id))
}

pub(crate) type KeymapOverrides = BTreeMap<String, Option<KeyCombo>>;

pub(crate) fn deserialize_overrides<'de, D>(deserializer: D) -> Result<KeymapOverrides, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let values = value
        .as_object()
        .map(|values| {
            values
                .iter()
                .map(|(id, value)| (id.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    Ok(normalize_override_values(values))
}

fn normalize_override_values(values: BTreeMap<String, serde_json::Value>) -> KeymapOverrides {
    values
        .into_iter()
        .filter_map(|(id, value)| {
            entry(&id)?;
            if value.is_null() {
                return Some((id, None));
            }
            let combo = serde_json::from_value(value).ok()?;
            combo_has_bindable_code(&combo).then_some((id, Some(combo)))
        })
        .collect()
}

pub(crate) fn normalize_overrides(overrides: &mut KeymapOverrides) {
    overrides.retain(|id, combo| {
        entry(id).is_some() && combo.as_ref().is_none_or(combo_has_bindable_code)
    });
}

pub(crate) fn effective_binding(id: &str, overrides: &KeymapOverrides) -> Option<KeyCombo> {
    let entry = entry(id)?;
    if is_tab_selection_entry(entry) {
        return (!tab_selection_unbound(overrides)).then(|| entry.default.clone());
    }
    match overrides.get(id) {
        Some(None) => None,
        Some(Some(combo)) if !entry.fixed => Some(combo.clone()),
        _ => Some(entry.default.clone()),
    }
}

/// The nine tab keys are turned off and on together, so a single digit
/// unbound before they became fixed is ignored like any other per-digit
/// override.
pub(crate) fn tab_selection_unbound(overrides: &KeymapOverrides) -> bool {
    tab_selection_entries().all(|entry| overrides.get(entry.id) == Some(&None))
}

pub(crate) fn is_overridden(id: &str, overrides: &KeymapOverrides) -> bool {
    let Some(entry) = entry(id) else {
        return false;
    };
    if is_tab_selection_entry(entry) {
        return tab_selection_unbound(overrides);
    }
    match overrides.get(id) {
        None => false,
        Some(None) => true,
        Some(Some(_)) => !entry.fixed,
    }
}

fn combos_collide(left: &KeyCombo, right: &KeyCombo) -> bool {
    left.code == right.code
        && left.meta == right.meta
        && left.ctrl == right.ctrl
        && left.alt == right.alt
        && (left.shift_optional || right.shift_optional || left.shift == right.shift)
}

pub(crate) fn find_conflict(
    candidate: &KeyCombo,
    for_id: &str,
    overrides: &KeymapOverrides,
) -> Option<&'static KeymapEntry> {
    let target = entry(for_id)?;
    entries()
        .iter()
        .find(|other| {
            // A fixed key stays reserved while it is turned off, so turning
            // it off never frees it for another shortcut.
            let binding = if other.fixed {
                Some(other.default.clone())
            } else {
                effective_binding(other.id, overrides)
            };
            other.id != for_id
                && other.scope.overlaps(target.scope)
                && binding.is_some_and(|binding| combos_collide(&binding, candidate))
        })
        .or_else(|| {
            reserved_entries()
                .iter()
                .find(|other| combos_collide(&other.default, candidate))
        })
}

pub(crate) fn clear_override(
    id: &str,
    overrides: &mut KeymapOverrides,
) -> Result<bool, &'static KeymapEntry> {
    if !overrides.contains_key(id) {
        return Ok(false);
    }
    let Some(entry) = entry(id) else {
        return Ok(false);
    };
    if let Some(conflict) = find_conflict(&entry.default, id, overrides) {
        return Err(conflict);
    }
    overrides.remove(id);
    Ok(true)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KeyEventLike {
    pub(crate) meta: bool,
    pub(crate) ctrl: bool,
    pub(crate) alt: bool,
    pub(crate) shift: bool,
    pub(crate) code: String,
    pub(crate) key: String,
}

#[cfg(test)]
fn combo_matches_event(combo: &KeyCombo, event: &KeyEventLike) -> bool {
    event.code == combo.code
        && event.meta == combo.meta
        && event.ctrl == combo.ctrl
        && event.alt == combo.alt
        && (combo.shift_optional || event.shift == combo.shift)
}

pub(crate) fn combo_from_event(event: &KeyEventLike) -> Option<KeyCombo> {
    if event.code.is_empty()
        || event.code.starts_with("Meta")
        || event.code.starts_with("Control")
        || event.code.starts_with("Alt")
        || event.code.starts_with("Shift")
    {
        return None;
    }
    let has_real_modifier = event.meta || event.ctrl || event.alt;
    let function_key = event
        .code
        .strip_prefix('F')
        .is_some_and(|digits| (1..=2).contains(&digits.len()) && digits.parse::<u8>().is_ok());
    if !has_real_modifier && !function_key {
        return None;
    }
    let printable =
        (event.key.chars().count() == 1 && event.key != " ").then(|| event.key.to_uppercase());
    let default = default_key_label(&event.code);
    Some(KeyCombo {
        meta: event.meta,
        ctrl: event.ctrl,
        alt: event.alt,
        shift: event.shift,
        code: event.code.clone(),
        label: printable.filter(|label| label != &default),
        shift_optional: false,
    })
}

pub(crate) fn combo_from_keystroke(keystroke: &Keystroke) -> Option<KeyCombo> {
    let (code, implied_shift) = code_from_gpui_key(&keystroke.key)?;
    combo_from_event(&KeyEventLike {
        meta: keystroke.modifiers.platform,
        ctrl: keystroke.modifiers.control,
        alt: keystroke.modifiers.alt,
        shift: keystroke.modifiers.shift || implied_shift,
        code,
        key: keystroke.key.clone(),
    })
}

fn code_from_gpui_key(key: &str) -> Option<(String, bool)> {
    if key.len() == 1 {
        let character = key.chars().next()?;
        if character.is_ascii_alphabetic() {
            return Some((format!("Key{}", character.to_ascii_uppercase()), false));
        }
        if character.is_ascii_digit() {
            return Some((format!("Digit{character}"), false));
        }
        let (code, shifted) = match character {
            ',' => ("Comma", false),
            '<' => ("Comma", true),
            '.' => ("Period", false),
            '>' => ("Period", true),
            '/' => ("Slash", false),
            '?' => ("Slash", true),
            '\\' => ("Backslash", false),
            '|' => ("Backslash", true),
            ';' => ("Semicolon", false),
            ':' => ("Semicolon", true),
            '\'' => ("Quote", false),
            '"' => ("Quote", true),
            '`' => ("Backquote", false),
            '~' => ("Backquote", true),
            '-' => ("Minus", false),
            '_' => ("Minus", true),
            '=' => ("Equal", false),
            '+' => ("Equal", true),
            '[' => ("BracketLeft", false),
            '{' => ("BracketLeft", true),
            ']' => ("BracketRight", false),
            '}' => ("BracketRight", true),
            '!' => ("Digit1", true),
            '@' => ("Digit2", true),
            '#' => ("Digit3", true),
            '$' => ("Digit4", true),
            '%' => ("Digit5", true),
            '^' => ("Digit6", true),
            '&' => ("Digit7", true),
            '*' => ("Digit8", true),
            '(' => ("Digit9", true),
            ')' => ("Digit0", true),
            _ => return None,
        };
        return Some((code.to_owned(), shifted));
    }
    let code = match key.to_ascii_lowercase().as_str() {
        "enter" => "Enter",
        "tab" => "Tab",
        "space" => "Space",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        key if key.strip_prefix('f').is_some_and(|digits| {
            digits
                .parse::<u8>()
                .is_ok_and(|number| (1..=24).contains(&number))
        }) =>
        {
            return Some((key.to_ascii_uppercase(), false));
        }
        _ => return None,
    };
    Some((code.to_owned(), false))
}

fn default_key_label(code: &str) -> String {
    match code {
        "Comma" => ",".into(),
        "Period" => ".".into(),
        "Slash" => "/".into(),
        "Backslash" => "\\".into(),
        "Semicolon" => ";".into(),
        "Quote" => "'".into(),
        "Backquote" => "`".into(),
        "Minus" => "-".into(),
        "Equal" => "=".into(),
        "BracketLeft" => "[".into(),
        "BracketRight" => "]".into(),
        "Enter" => "↩".into(),
        "Tab" => "⇥".into(),
        "Space" => "Space".into(),
        "Backspace" => "⌫".into(),
        "Delete" => "⌦".into(),
        "ArrowLeft" => "←".into(),
        "ArrowRight" => "→".into(),
        "ArrowUp" => "↑".into(),
        "ArrowDown" => "↓".into(),
        "Home" => "↖".into(),
        "End" => "↘".into(),
        "PageUp" => "⇞".into(),
        "PageDown" => "⇟".into(),
        code if code.starts_with("Key") && code.len() == 4 => code[3..].into(),
        code if code.starts_with("Digit") && code.len() == 6 => code[5..].into(),
        code => code.into(),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn format_combo(combo: &KeyCombo) -> String {
    format!(
        "{}{}{}{}{}",
        if combo.ctrl { "⌃" } else { "" },
        if combo.alt { "⌥" } else { "" },
        if combo.shift && !combo.shift_optional {
            "⇧"
        } else {
            ""
        },
        if combo.meta { "⌘" } else { "" },
        combo
            .label
            .clone()
            .unwrap_or_else(|| default_key_label(&combo.code))
    )
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn format_combo(combo: &KeyCombo) -> String {
    format!(
        "{}{}{}{}{}",
        if combo.ctrl { "Ctrl+" } else { "" },
        if combo.alt { "Alt+" } else { "" },
        if combo.shift && !combo.shift_optional {
            "Shift+"
        } else {
            ""
        },
        if combo.meta {
            if cfg!(target_os = "linux") {
                "Super+"
            } else {
                "Win+"
            }
        } else {
            ""
        },
        combo
            .label
            .clone()
            .unwrap_or_else(|| match combo.code.as_str() {
                "Enter" | "Tab" | "Backspace" | "Delete" | "Home" | "End" => combo.code.clone(),
                "ArrowLeft" => "Left".into(),
                "ArrowRight" => "Right".into(),
                "ArrowUp" => "Up".into(),
                "ArrowDown" => "Down".into(),
                "PageUp" => "PageUp".into(),
                "PageDown" => "PageDown".into(),
                _ => default_key_label(&combo.code),
            })
    )
}

fn gpui_key(code: &str) -> Option<&'static str> {
    Some(match code {
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        "Backslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "Backquote" => "`",
        "Minus" => "-",
        "Equal" => "=",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Enter" => "enter",
        "Tab" => "tab",
        "Space" => "space",
        "Backspace" => "backspace",
        "Delete" => "delete",
        "ArrowLeft" => "left",
        "ArrowRight" => "right",
        "ArrowUp" => "up",
        "ArrowDown" => "down",
        "Home" => "home",
        "End" => "end",
        "PageUp" => "pageup",
        "PageDown" => "pagedown",
        _ => return None,
    })
}

fn combo_has_bindable_code(combo: &KeyCombo) -> bool {
    let code = combo.code.as_str();
    code.strip_prefix("Key")
        .is_some_and(|key| key.len() == 1 && key.as_bytes()[0].is_ascii_uppercase())
        || code
            .strip_prefix("Digit")
            .is_some_and(|digit| digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit())
        || code.strip_prefix('F').is_some_and(|digits| {
            digits
                .parse::<u8>()
                .is_ok_and(|number| (1..=24).contains(&number))
        })
        || gpui_key(code).is_some()
}

fn shifted_printable_key(code: &str) -> Option<&'static str> {
    Some(match code {
        "Comma" => "<",
        "Period" => ">",
        "Slash" => "?",
        "Backslash" => "|",
        "Semicolon" => ":",
        "Quote" => "\"",
        "Backquote" => "~",
        "Minus" => "_",
        "Equal" => "+",
        "BracketLeft" => "{",
        "BracketRight" => "}",
        "Digit1" => "!",
        "Digit2" => "@",
        "Digit3" => "#",
        "Digit4" => "$",
        "Digit5" => "%",
        "Digit6" => "^",
        "Digit7" => "&",
        "Digit8" => "*",
        "Digit9" => "(",
        "Digit0" => ")",
        _ => return None,
    })
}

pub(crate) fn binding_strings(combo: &KeyCombo) -> Vec<String> {
    let key = if combo
        .code
        .strip_prefix("Key")
        .is_some_and(|key| key.len() == 1 && key.as_bytes()[0].is_ascii_uppercase())
    {
        combo.code[3..].to_ascii_lowercase()
    } else if combo.code.starts_with("Digit") && combo.code.len() == 6 {
        combo.code[5..].to_owned()
    } else if combo.code.strip_prefix('F').is_some_and(|digits| {
        digits
            .parse::<u8>()
            .is_ok_and(|number| (1..=24).contains(&number))
    }) {
        combo.code.to_ascii_lowercase()
    } else {
        let Some(key) = gpui_key(&combo.code) else {
            return Vec::new();
        };
        key.to_owned()
    };
    let build = |shift: bool| {
        let shifted_key = shift.then(|| shifted_printable_key(&combo.code)).flatten();
        let mut value = String::new();
        if combo.ctrl {
            value.push_str("ctrl-");
        }
        if combo.alt {
            value.push_str("alt-");
        }
        if shift && shifted_key.is_none() {
            value.push_str("shift-");
        }
        if combo.meta {
            value.push_str("cmd-");
        }
        value.push_str(shifted_key.unwrap_or(&key));
        value
    };
    if combo.shift_optional {
        vec![build(false), build(true)]
    } else {
        vec![build(combo.shift)]
    }
}

/// Terminal first leaves a chord of Ctrl plus one key to a focused terminal;
/// zoom keeps working there in both modes.
pub(crate) fn frees_for_terminal(entry: &KeymapEntry, combo: &KeyCombo) -> bool {
    combo.ctrl
        && !combo.shift
        && !combo.alt
        && !combo.meta
        && !matches!(entry.id, "zoom-in" | "zoom-out" | "zoom-reset")
}

fn installed_context(
    entry: &KeymapEntry,
    combo: &KeyCombo,
    terminal_ctrl_keys: TerminalCtrlKeys,
) -> Option<String> {
    let context = binding_context(entry, combo);
    if terminal_ctrl_keys == TerminalCtrlKeys::RunnerFirst || !frees_for_terminal(entry, combo) {
        return context.map(str::to_owned);
    }
    Some(match context {
        Some(context) => format!("({context}) && !Terminal"),
        None => "!Terminal".to_owned(),
    })
}

fn binding_context(entry: &KeymapEntry, combo: &KeyCombo) -> Option<&'static str> {
    // On Windows and Linux, Ctrl keys inside terminals follow the Ctrl keys
    // in terminals setting rather than staying out of terminals here.
    let modified = combo.meta || (cfg!(any(windows, target_os = "linux")) && combo.ctrl);
    match entry.id {
        // The launcher's search is focused on every Start page.
        "new-chat" | "command-palette" if modified => {
            Some("!(TextInput && !LauncherSearch) && !Settings")
        }
        "page-previous" | "page-next" if modified => Some("!TextInput && !Settings"),
        "new-chat" | "command-palette" | "page-previous" | "page-next" => {
            Some("!TextInput && !Settings && !Terminal")
        }
        "toggle-sidebar" | "switch-sidebar-view" => Some("!TextInput && !Settings"),
        // The open find bar sits beside the terminal, outside its context.
        "find-in-terminal" => Some("Terminal || TerminalFind"),
        _ => entry.scope.context(),
    }
}

/// The file viewer's keys, while focus is in a side panel's Files view:
/// ⌘W there closes the file tab, and with none open falls through to the
/// pane. A binding without a context counts as matching the deepest one, so
/// these go after the pane's ⌘W to win the tie, and ⌘W gets a second binding
/// for the view's text fields.
pub(crate) fn install_file_viewer_bindings(cx: &mut App) {
    let context = Some("FileViewer");
    cx.bind_keys([
        KeyBinding::new(&platform_default("cmd-w"), CloseFileTab, context),
        KeyBinding::new(
            &platform_default("cmd-w"),
            CloseFileTab,
            Some("FileViewer > TextInput"),
        ),
        KeyBinding::new("ctrl-tab", NextFileTab, context),
        KeyBinding::new("ctrl-shift-tab", PreviousFileTab, context),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-f", FindInFile, context),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", Copy, context),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-a", crate::SelectAll, context),
        #[cfg(any(windows, target_os = "linux"))]
        KeyBinding::new("ctrl-f", FindInFile, context),
        #[cfg(any(windows, target_os = "linux"))]
        KeyBinding::new("ctrl-c", Copy, context),
        #[cfg(any(windows, target_os = "linux"))]
        KeyBinding::new("ctrl-a", crate::SelectAll, context),
    ]);
}

/// Zoom in and out, as bound or rebound, step the file viewer's text while it
/// has focus instead of the app (Jason, 2026-11-05); from the tree the Files
/// view hands them on to the app's zoom. Zoom reset stays the app's everywhere
/// (Jason, 2026-11-07). Installed after app zoom's bindings, since the later of
/// two equally deep bindings wins.
pub(crate) fn install_file_zoom_bindings(cx: &mut App, overrides: &KeymapOverrides) {
    let combos = |id| {
        effective_binding(id, overrides)
            .map(|combo| binding_strings(&combo))
            .unwrap_or_default()
    };
    // The find field is a text input inside the viewer.
    for context in [Some("FileViewer"), Some("FileViewer > TextInput")] {
        for binding in combos("zoom-in") {
            cx.bind_keys([KeyBinding::new(&binding, crate::ZoomFileIn, context)]);
        }
        for binding in combos("zoom-out") {
            cx.bind_keys([KeyBinding::new(&binding, crate::ZoomFileOut, context)]);
        }
    }
}

pub(crate) fn install_bindings(
    cx: &mut App,
    overrides: &KeymapOverrides,
    terminal_ctrl_keys: TerminalCtrlKeys,
    shortcuts_suspended: bool,
) {
    cx.clear_key_bindings();
    cx.bind_keys([
        KeyBinding::new(&platform_default("cmd-c"), Copy, Some("Terminal")),
        #[cfg(target_os = "macos")]
        KeyBinding::new(&platform_default("cmd-c"), Copy, Some("MissionFeed")),
        #[cfg(any(windows, target_os = "linux"))]
        KeyBinding::new("ctrl-c", Copy, Some("MissionFeed")),
        KeyBinding::new(&platform_default("cmd-v"), Paste, Some("Terminal")),
    ]);
    if shortcuts_suspended {
        install_modal_bindings(cx);
        return;
    }
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new(&platform_default("cmd-q"), Quit, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-q", QuitAndStopSessions, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new(&platform_default("cmd-h"), Hide, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new(&platform_default("cmd-alt-h"), HideOthers, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new(&platform_default("ctrl-cmd-f"), ToggleFullscreen, None),
    ]);
    if let Some(combo) = effective_binding("close-pane", overrides) {
        for binding in binding_strings(&combo) {
            cx.bind_keys([KeyBinding::new(&binding, CloseTab, None)]);
        }
    }
    install_file_viewer_bindings(cx);
    if let Some(combo) = effective_binding("system-close-window", overrides) {
        for binding in binding_strings(&combo) {
            cx.bind_keys([KeyBinding::new(&binding, CloseWindow, None)]);
        }
    }
    if effective_binding("new-window", overrides).is_some() {
        cx.bind_keys([KeyBinding::new(
            &platform_default("shift-cmd-n"),
            NewWindow,
            None,
        )]);
    }
    for entry in entries()
        .iter()
        .filter(|entry| !entry.fixed || is_tab_selection_entry(entry))
    {
        let Some(combo) = effective_binding(entry.id, overrides) else {
            continue;
        };
        let context = installed_context(entry, &combo, terminal_ctrl_keys);
        let context = context.as_deref();
        for binding in binding_strings(&combo) {
            let key_binding = match entry.id {
                "new-chat" => KeyBinding::new(&binding, NewTab, context),
                "new-terminal" => KeyBinding::new(&binding, NewTerminal, context),
                "new-mission" => KeyBinding::new(&binding, NewMission, context),
                "command-palette" => KeyBinding::new(&binding, CommandPalette, context),
                "search-files" => KeyBinding::new(&binding, SearchFiles, context),
                "toggle-sidebar" => KeyBinding::new(&binding, ToggleSidebar, context),
                "switch-sidebar-view" => KeyBinding::new(&binding, SwitchSidebarView, context),
                "toggle-terminal-drawer" => {
                    KeyBinding::new(&binding, ToggleTerminalDrawer, context)
                }
                "find-in-terminal" => KeyBinding::new(&binding, FindInTerminal, context),
                "open-settings" => KeyBinding::new(&binding, OpenSettings, context),
                "page-previous" => KeyBinding::new(&binding, NavigatePreviousPage, context),
                "page-next" => KeyBinding::new(&binding, NavigateNextPage, context),
                "zoom-in" => KeyBinding::new(&binding, ZoomIn, context),
                "zoom-out" => KeyBinding::new(&binding, ZoomOut, context),
                "zoom-reset" => KeyBinding::new(&binding, ZoomReset, context),
                "select-tab-1" => KeyBinding::new(&binding, SelectTab1, context),
                "select-tab-2" => KeyBinding::new(&binding, SelectTab2, context),
                "select-tab-3" => KeyBinding::new(&binding, SelectTab3, context),
                "select-tab-4" => KeyBinding::new(&binding, SelectTab4, context),
                "select-tab-5" => KeyBinding::new(&binding, SelectTab5, context),
                "select-tab-6" => KeyBinding::new(&binding, SelectTab6, context),
                "select-tab-7" => KeyBinding::new(&binding, SelectTab7, context),
                "select-tab-8" => KeyBinding::new(&binding, SelectTab8, context),
                "select-tab-9" => KeyBinding::new(&binding, SelectTab9, context),
                "pane-previous" => KeyBinding::new(&binding, FocusPreviousPane, context),
                "pane-next" => KeyBinding::new(&binding, FocusNextPane, context),
                "split-pane-right" => KeyBinding::new(&binding, SplitPaneRight, context),
                "split-pane-down" => KeyBinding::new(&binding, SplitPaneDown, context),
                "toggle-maximize-pane" => KeyBinding::new(&binding, ToggleMaximizePane, context),
                "stop-session" => KeyBinding::new(&binding, StopFocusedSession, context),
                "resume-session" => KeyBinding::new(&binding, ResumeFocusedSession, context),
                "mission-tab-previous" => KeyBinding::new(&binding, MissionTabPrevious, context),
                "mission-tab-next" => KeyBinding::new(&binding, MissionTabNext, context),
                _ => continue,
            };
            cx.bind_keys([key_binding]);
        }
    }
    // After app zoom's own bindings, which a later binding outranks.
    install_file_zoom_bindings(cx, overrides);
    install_modal_bindings(cx);
}

fn install_modal_bindings(cx: &mut App) {
    // Double negation checks the whole context stack at the focused control's depth; loading last wins ties with user bindings.
    cx.bind_keys([KeyBinding::new(
        &platform_default("cmd-enter"),
        ConfirmStartChat,
        Some("!(!StartChat)"),
    )]);
}

#[cfg(test)]
mod tests;
