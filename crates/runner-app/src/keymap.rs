use std::collections::BTreeMap;

use gpui::{App, KeyBinding, Keystroke};
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    ClosePane, CloseWindow, CommandPalette, FocusNextPane, FocusPreviousPane, Hide, HideOthers,
    Minimize, MissionTabNext, MissionTabPrevious, NavigateNextPage, NavigatePreviousPage, NewTab,
    OpenSettings, Paste, Quit, ToggleFullscreen, ToggleSidebar, ZoomIn, ZoomOut, ZoomReset,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeymapScope {
    Global,
    ChatSplit,
    Mission,
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
    pub(crate) title: &'static str,
    pub(crate) description: &'static str,
    pub(crate) scope: KeymapScope,
    pub(crate) default: KeyCombo,
    pub(crate) fixed: bool,
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

pub(crate) fn entries() -> &'static [KeymapEntry] {
    static KEYMAP: std::sync::OnceLock<Vec<KeymapEntry>> = std::sync::OnceLock::new();
    KEYMAP.get_or_init(|| {
        vec![
            KeymapEntry {
                id: "new-window",
                title: "New window",
                description: "Open another Runner window.",
                scope: KeymapScope::Global,
                default: key_combo("KeyN", true, false, false, true, None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "new-chat",
                title: "New chat",
                description: "Start a chat in a new tab.",
                scope: KeymapScope::Global,
                default: key_combo("KeyN", true, false, false, false, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "command-palette",
                title: "Command palette",
                description: "Search missions, chats, runners, and crews.",
                scope: KeymapScope::Global,
                default: key_combo("KeyK", true, false, false, false, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "toggle-sidebar",
                title: "Toggle sidebar",
                description: "Collapse or expand the app sidebar.",
                scope: KeymapScope::Global,
                default: key_combo("KeyS", true, false, false, false, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "open-settings",
                title: "Open settings",
                description: "Open this settings page.",
                scope: KeymapScope::Global,
                default: key_combo("Comma", true, false, false, false, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "page-previous",
                title: "Previous page",
                description: "Step back through recently viewed missions and chats.",
                scope: KeymapScope::Global,
                default: key_combo("BracketLeft", true, false, false, true, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "page-next",
                title: "Next page",
                description: "Step forward through recently viewed missions and chats.",
                scope: KeymapScope::Global,
                default: key_combo("BracketRight", true, false, false, true, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "zoom-in",
                title: "Zoom in",
                description: "Scale the whole app up.",
                scope: KeymapScope::Global,
                default: key_combo("Equal", true, false, false, false, Some("+"), true),
                fixed: false,
            },
            KeymapEntry {
                id: "zoom-out",
                title: "Zoom out",
                description: "Scale the whole app down.",
                scope: KeymapScope::Global,
                default: key_combo("Minus", true, false, false, false, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "zoom-reset",
                title: "Reset zoom",
                description: "Return the app to 100%.",
                scope: KeymapScope::Global,
                default: key_combo("Digit0", true, false, false, false, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "pane-previous",
                title: "Previous chat pane",
                description: "Focus the previous pane while a chat is split.",
                scope: KeymapScope::ChatSplit,
                default: key_combo("BracketLeft", true, false, false, false, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "pane-next",
                title: "Next chat pane",
                description: "Focus the next pane while a chat is split.",
                scope: KeymapScope::ChatSplit,
                default: key_combo("BracketRight", true, false, false, false, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "close-pane",
                title: "Close pane",
                description: "Collapse the focused pane while a chat is split.",
                scope: KeymapScope::ChatSplit,
                default: key_combo("KeyW", true, false, false, false, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "mission-tab-previous",
                title: "Previous mission tab",
                description: "Cycle back through the feed and open runner tabs.",
                scope: KeymapScope::Mission,
                default: key_combo("BracketLeft", true, false, false, false, None, false),
                fixed: false,
            },
            KeymapEntry {
                id: "mission-tab-next",
                title: "Next mission tab",
                description: "Cycle forward through the feed and open runner tabs.",
                scope: KeymapScope::Mission,
                default: key_combo("BracketRight", true, false, false, false, None, false),
                fixed: false,
            },
        ]
    })
}

pub(crate) fn entry(id: &str) -> Option<&'static KeymapEntry> {
    entries().iter().find(|entry| entry.id == id)
}

fn reserved_entries() -> &'static [KeymapEntry] {
    static RESERVED: std::sync::OnceLock<Vec<KeymapEntry>> = std::sync::OnceLock::new();
    RESERVED.get_or_init(|| {
        vec![
            KeymapEntry {
                id: "system-quit",
                title: "Quit Runner",
                description: "",
                scope: KeymapScope::Global,
                default: key_combo("KeyQ", true, false, false, false, None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "system-hide",
                title: "Hide Runner",
                description: "",
                scope: KeymapScope::Global,
                default: key_combo("KeyH", true, false, false, false, None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "system-hide-others",
                title: "Hide Others",
                description: "",
                scope: KeymapScope::Global,
                default: key_combo("KeyH", true, false, true, false, None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "system-minimize",
                title: "Minimize",
                description: "",
                scope: KeymapScope::Global,
                default: key_combo("KeyM", true, false, false, false, None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "system-close-window",
                title: "Close window",
                description: "",
                scope: KeymapScope::Global,
                default: key_combo("KeyW", true, false, false, false, None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "system-toggle-fullscreen",
                title: "Toggle fullscreen",
                description: "",
                scope: KeymapScope::Global,
                default: key_combo("KeyF", true, true, false, false, None, false),
                fixed: true,
            },
            KeymapEntry {
                id: "system-paste",
                title: "Paste",
                description: "",
                scope: KeymapScope::Global,
                default: key_combo("KeyV", true, false, false, false, None, false),
                fixed: true,
            },
        ]
    })
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
    if entry.fixed {
        return Some(entry.default.clone());
    }
    overrides
        .get(id)
        .cloned()
        .unwrap_or_else(|| Some(entry.default.clone()))
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
            other.id != for_id
                && other.scope.overlaps(target.scope)
                && effective_binding(other.id, overrides)
                    .as_ref()
                    .is_some_and(|binding| combos_collide(binding, candidate))
        })
        .or_else(|| {
            reserved_entries().iter().find(|other| {
                !(target.scope == KeymapScope::ChatSplit && other.id == "system-close-window")
                    && combos_collide(&other.default, candidate)
            })
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

fn binding_strings(combo: &KeyCombo) -> Vec<String> {
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

fn binding_context(entry: &KeymapEntry, combo: &KeyCombo) -> Option<&'static str> {
    match entry.id {
        "new-chat" | "command-palette" | "page-previous" | "page-next" if combo.meta => {
            Some("!TextInput && !Settings")
        }
        "new-chat" | "command-palette" | "page-previous" | "page-next" => {
            Some("!TextInput && !Settings && !Terminal")
        }
        "toggle-sidebar" => Some("!TextInput && !Settings"),
        _ => entry.scope.context(),
    }
}

pub(crate) fn install_bindings(
    cx: &mut App,
    overrides: &KeymapOverrides,
    shortcuts_suspended: bool,
) {
    cx.clear_key_bindings();
    cx.bind_keys([KeyBinding::new("cmd-v", Paste, Some("Terminal"))]);
    if shortcuts_suspended {
        return;
    }
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-w", CloseWindow, Some("!ChatSplit")),
        KeyBinding::new("ctrl-cmd-f", ToggleFullscreen, None),
    ]);
    for entry in entries().iter().filter(|entry| !entry.fixed) {
        let Some(combo) = effective_binding(entry.id, overrides) else {
            continue;
        };
        for binding in binding_strings(&combo) {
            let context = binding_context(entry, &combo);
            let key_binding = match entry.id {
                "new-chat" => KeyBinding::new(&binding, NewTab, context),
                "command-palette" => KeyBinding::new(&binding, CommandPalette, context),
                "toggle-sidebar" => KeyBinding::new(&binding, ToggleSidebar, context),
                "open-settings" => KeyBinding::new(&binding, OpenSettings, context),
                "page-previous" => KeyBinding::new(&binding, NavigatePreviousPage, context),
                "page-next" => KeyBinding::new(&binding, NavigateNextPage, context),
                "zoom-in" => KeyBinding::new(&binding, ZoomIn, context),
                "zoom-out" => KeyBinding::new(&binding, ZoomOut, context),
                "zoom-reset" => KeyBinding::new(&binding, ZoomReset, context),
                "pane-previous" => KeyBinding::new(&binding, FocusPreviousPane, context),
                "pane-next" => KeyBinding::new(&binding, FocusNextPane, context),
                "close-pane" => KeyBinding::new(&binding, ClosePane, context),
                "mission-tab-previous" => KeyBinding::new(&binding, MissionTabPrevious, context),
                "mission-tab-next" => KeyBinding::new(&binding, MissionTabNext, context),
                _ => continue,
            };
            cx.bind_keys([key_binding]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo_for(code: &str, shift: bool) -> KeyCombo {
        key_combo(code, true, false, false, shift, None, false)
    }

    #[test]
    fn registry_matches_the_shipped_defaults_and_fixed_entry() {
        assert_eq!(entries().len(), 15);
        assert_eq!(entries().iter().filter(|entry| entry.fixed).count(), 1);
        assert!(entry("new-window").unwrap().fixed);
        assert_eq!(format_combo(&entry("new-window").unwrap().default), "⇧⌘N");
        assert_eq!(
            format_combo(&entry("page-previous").unwrap().default),
            "⇧⌘["
        );
        assert_eq!(format_combo(&entry("zoom-in").unwrap().default), "⌘+");
        assert_eq!(
            entry("pane-previous").unwrap().default,
            entry("mission-tab-previous").unwrap().default
        );
    }

    #[test]
    fn overrides_preserve_null_and_absent_as_different_states() {
        let mut overrides = KeymapOverrides::new();
        assert_eq!(
            effective_binding("command-palette", &overrides),
            Some(combo_for("KeyK", false))
        );
        overrides.insert("command-palette".into(), None);
        assert_eq!(effective_binding("command-palette", &overrides), None);
        overrides.insert("new-window".into(), Some(combo_for("KeyP", false)));
        assert_eq!(
            effective_binding("new-window", &overrides),
            Some(combo_for("KeyN", true))
        );
    }

    #[test]
    fn normalization_drops_unknown_and_malformed_values() {
        let values =
            serde_json::from_value::<BTreeMap<String, serde_json::Value>>(serde_json::json!({
                "command-palette": null,
                "toggle-sidebar": {
                    "meta": true, "ctrl": false, "alt": false, "shift": false, "code": "KeyP"
                },
                "zoom-in": { "meta": "yes", "code": "Equal" },
                "zoom-out": {
                    "meta": true, "ctrl": false, "alt": false, "shift": false, "code": "a-b"
                },
                "unknown": {
                    "meta": true, "ctrl": false, "alt": false, "shift": false, "code": "KeyU"
                }
            }))
            .unwrap();
        let normalized = normalize_override_values(values);
        assert_eq!(normalized.len(), 2);
        assert!(normalized.contains_key("command-palette"));
        assert!(normalized["command-palette"].is_none());
        assert_eq!(normalized["toggle-sidebar"].as_ref().unwrap().code, "KeyP");
        assert!(!normalized.contains_key("zoom-out"));
    }

    #[test]
    fn conflicts_follow_scope_overlap_and_refuse_conflicting_restore() {
        let mut overrides = KeymapOverrides::new();
        overrides.insert("pane-previous".into(), Some(combo_for("KeyJ", false)));
        overrides.insert(
            "mission-tab-next".into(),
            Some(combo_for("BracketLeft", false)),
        );
        assert!(find_conflict(
            &combo_for("BracketLeft", false),
            "pane-previous",
            &overrides
        )
        .is_none());

        overrides.insert(
            "toggle-sidebar".into(),
            Some(combo_for("BracketLeft", false)),
        );
        let conflict = clear_override("pane-previous", &mut overrides).unwrap_err();
        assert_eq!(conflict.id, "toggle-sidebar");
        assert!(overrides.contains_key("pane-previous"));
    }

    #[test]
    fn conflicts_include_system_owned_shortcuts_without_blocking_split_close() {
        let mut overrides = KeymapOverrides::new();
        let conflict = find_conflict(&combo_for("KeyQ", false), "new-chat", &overrides).unwrap();
        assert_eq!(conflict.id, "system-quit");

        overrides.insert("close-pane".into(), Some(combo_for("KeyP", false)));
        assert!(clear_override("close-pane", &mut overrides).unwrap());
        assert!(!overrides.contains_key("close-pane"));
    }

    #[test]
    fn capture_validity_and_formatting_match_the_react_contract() {
        let bare = KeyEventLike {
            meta: false,
            ctrl: false,
            alt: false,
            shift: false,
            code: "KeyK".into(),
            key: "k".into(),
        };
        assert!(combo_from_event(&bare).is_none());
        let function = KeyEventLike {
            code: "F12".into(),
            key: "f12".into(),
            ..bare.clone()
        };
        assert_eq!(format_combo(&combo_from_event(&function).unwrap()), "F12");
        assert!(combo_from_keystroke(&Keystroke::parse("cmd-f24").unwrap()).is_some());
        assert!(combo_from_keystroke(&Keystroke::parse("cmd-f25").unwrap()).is_none());
        let plus = KeyEventLike {
            meta: true,
            shift: true,
            code: "Equal".into(),
            key: "+".into(),
            ..bare
        };
        assert_eq!(format_combo(&combo_from_event(&plus).unwrap()), "⇧⌘+");
    }

    #[test]
    fn shift_optional_matches_both_zoom_in_forms_only() {
        let combo = entry("zoom-in").unwrap().default.clone();
        for shift in [false, true] {
            assert!(combo_matches_event(
                &combo,
                &KeyEventLike {
                    meta: true,
                    ctrl: false,
                    alt: false,
                    shift,
                    code: "Equal".into(),
                    key: if shift { "+" } else { "=" }.into(),
                }
            ));
        }
        let ordinary = combo_for("Equal", false);
        assert!(!combo_matches_event(
            &ordinary,
            &KeyEventLike {
                meta: true,
                ctrl: false,
                alt: false,
                shift: true,
                code: "Equal".into(),
                key: "+".into(),
            }
        ));
    }

    #[test]
    fn registry_defaults_compile_to_gpui_runtime_keystrokes() {
        type ExpectedBinding<'a> = (&'a str, &'a str, bool);
        type ExpectedEntry<'a> = (&'a str, &'a [ExpectedBinding<'a>]);
        let expected: [ExpectedEntry<'_>; 15] = [
            ("new-window", &[("shift-cmd-n", "n", true)]),
            ("new-chat", &[("cmd-n", "n", false)]),
            ("command-palette", &[("cmd-k", "k", false)]),
            ("toggle-sidebar", &[("cmd-s", "s", false)]),
            ("open-settings", &[("cmd-,", ",", false)]),
            ("page-previous", &[("cmd-{", "{", false)]),
            ("page-next", &[("cmd-}", "}", false)]),
            ("zoom-in", &[("cmd-=", "=", false), ("cmd-+", "+", false)]),
            ("zoom-out", &[("cmd--", "-", false)]),
            ("zoom-reset", &[("cmd-0", "0", false)]),
            ("pane-previous", &[("cmd-[", "[", false)]),
            ("pane-next", &[("cmd-]", "]", false)]),
            ("close-pane", &[("cmd-w", "w", false)]),
            ("mission-tab-previous", &[("cmd-[", "[", false)]),
            ("mission-tab-next", &[("cmd-]", "]", false)]),
        ];

        for (id, bindings) in expected {
            assert_eq!(
                binding_strings(&entry(id).unwrap().default),
                bindings
                    .iter()
                    .map(|(binding, _, _)| (*binding).to_owned())
                    .collect::<Vec<_>>(),
                "{id}"
            );
            for (binding, key, shift) in bindings {
                assert_eq!(
                    Keystroke::parse(binding).unwrap(),
                    Keystroke {
                        modifiers: gpui::Modifiers {
                            platform: true,
                            shift: *shift,
                            ..Default::default()
                        },
                        key: (*key).to_owned(),
                        key_char: None,
                    },
                    "{id}: {binding}"
                );
            }
        }

        let recorded_bracket = combo_for("BracketLeft", true);
        assert_eq!(binding_strings(&recorded_bracket), ["cmd-{"]);
        assert_eq!(
            Keystroke::parse(&binding_strings(&recorded_bracket)[0]).unwrap(),
            Keystroke {
                modifiers: gpui::Modifiers {
                    platform: true,
                    ..Default::default()
                },
                key: "{".into(),
                key_char: None,
            }
        );

        let recorded_letter = combo_for("KeyK", true);
        assert_eq!(binding_strings(&recorded_letter), ["shift-cmd-k"]);
        assert!(
            Keystroke::parse(&binding_strings(&recorded_letter)[0])
                .unwrap()
                .modifiers
                .shift
        );
    }
}
