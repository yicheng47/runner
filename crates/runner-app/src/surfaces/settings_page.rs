use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, BoxShadow, Context, CursorStyle, Entity, FontWeight,
    KeyDownEvent, PathPromptOptions, Pixels, ScrollHandle, SharedString, Subscription, Window,
};
use runner_app::ui::{
    working_dir_text_field, Button, ButtonSize, ButtonVariant, IconButton, IconButtonSize,
    PaneHeader, Scrollbar, SelectHandler, SelectOption, SettingsCard, SettingsRow, StepHandler,
    Stepper, StyledSelect, TextField, Toggle, WorkingDirField,
};
use runner_core::protocol::command::{
    CommandActionOutcome, RunnerCommandState, RunnerCommandStatus,
};

use super::*;
use crate::app_settings::{
    normalize_zoom, nudge_zoom, DarkTerminalTheme, FileLinkEditor, LightTerminalTheme,
    TerminalCursorStyle, TerminalFontFamily, TERMINAL_FONT_SIZE_MAX, TERMINAL_FONT_SIZE_MIN,
    TERMINAL_SCROLLBACK_LINES, ZOOM_STEPS,
};
use crate::platform_ui::{SETTINGS_CONTENT_TOP, SETTINGS_DRAG_INSET};
use crate::surfaces::app_shell::TITLEBAR_DRAG_HEIGHT;
use crate::surfaces::settings::theme_preview::{self, PreviewMode, PreviewPane};
use crate::theme::{DarkTheme, LightTheme, ThemeIntent};
use crate::*;

const SETTINGS_SAVE_DELAY_MS: u64 = 300;
/// The four palette selects in Appearance share one trigger width, so the
/// app and terminal rows of each mode line up.
const SETTINGS_SELECT_WIDTH: Pixels = px(176.);
const TAB_SHORTCUT_ID: &str = "select-tab";
const TAB_SHORTCUT_TITLE: &str = "ui.go_to_numbered_tab";
const TAB_SHORTCUT_DESCRIPTION: &str = "ui.open_a_visible_sidebar_tab_or_mission_by_its_position";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShortcutRow {
    Entry(&'static keymap::KeymapEntry),
    TabSelection,
}

impl ShortcutRow {
    fn id(self) -> &'static str {
        match self {
            Self::Entry(entry) => entry.id,
            Self::TabSelection => TAB_SHORTCUT_ID,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Entry(entry) => entry.title(),
            Self::TabSelection => crate::i18n::text(TAB_SHORTCUT_TITLE),
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Entry(entry) => entry.description(),
            Self::TabSelection => crate::i18n::text(TAB_SHORTCUT_DESCRIPTION),
        }
    }

    fn fixed(self) -> bool {
        match self {
            Self::Entry(entry) => entry.fixed,
            Self::TabSelection => true,
        }
    }

    fn entry_ids(self) -> Vec<&'static str> {
        match self {
            Self::Entry(entry) => vec![entry.id],
            Self::TabSelection => keymap::tab_selection_entries()
                .map(|entry| entry.id)
                .collect(),
        }
    }

    fn overridden(self, overrides: &keymap::KeymapOverrides) -> bool {
        self.entry_ids()
            .into_iter()
            .any(|id| keymap::is_overridden(id, overrides))
    }

    fn binding_label(self, overrides: &keymap::KeymapOverrides) -> String {
        let label = |id: &str| {
            keymap::effective_binding(id, overrides).map(|combo| keymap::format_combo(&combo))
        };
        let ids = self.entry_ids();
        match (label(ids[0]), label(ids[ids.len() - 1])) {
            (Some(first), Some(last)) if ids.len() > 1 => format!("{first}–{last}"),
            (Some(first), _) => first,
            _ => crate::i18n::text("ui.unassigned").into(),
        }
    }

    fn matches(self, query: &str, overrides: &keymap::KeymapOverrides) -> bool {
        let matches = |value: &str| value.to_lowercase().contains(query);
        matches(self.title())
            || matches(self.description())
            || matches(&self.binding_label(overrides))
            || (self == Self::TabSelection
                && keymap::tab_selection_entries().any(|entry| {
                    matches(entry.title())
                        || matches(entry.description())
                        || keymap::effective_binding(entry.id, overrides)
                            .is_some_and(|combo| matches(&keymap::format_combo(&combo)))
                }))
    }
}

fn shortcut_rows(query: &str, overrides: &keymap::KeymapOverrides) -> Vec<ShortcutRow> {
    let query = query.trim().to_lowercase();
    let mut rows = Vec::new();
    for entry in keymap::entries() {
        let row = if keymap::is_tab_selection_entry(entry) {
            ShortcutRow::TabSelection
        } else {
            ShortcutRow::Entry(entry)
        };
        if rows.contains(&row) || (!query.is_empty() && !row.matches(&query, overrides)) {
            continue;
        }
        rows.push(row);
    }
    rows
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandInstallActivity {
    Install,
    Uninstall,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SettingsPane {
    Account,
    #[default]
    General,
    Missions,
    Appearance,
    Notifications,
    Remote,
    Terminal,
    Shortcuts,
    Agents,
    Skills,
    Mcp,
    Updates,
    Diagnostics,
    About,
    Archived,
}

impl SettingsPane {
    fn from_route(value: Option<&str>) -> Self {
        match value {
            Some("account") => Self::Account,
            Some("missions") => Self::Missions,
            Some("appearance") => Self::Appearance,
            Some("remote") => Self::Remote,
            Some("notifications") => Self::Notifications,
            Some("terminal") => Self::Terminal,
            Some("shortcuts") => Self::Shortcuts,
            Some("agents") => Self::Agents,
            Some("mcp") => Self::Mcp,
            Some("skills") => Self::Skills,
            Some("updates") => Self::Updates,
            Some("diagnostics") => Self::Diagnostics,
            Some("about") => Self::About,
            Some("archived") => Self::Archived,
            Some("general") | None | Some(_) => Self::General,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::General => "general",
            Self::Missions => "missions",
            Self::Appearance => "appearance",
            Self::Remote => "remote",
            Self::Notifications => "notifications",
            Self::Terminal => "terminal",
            Self::Shortcuts => "shortcuts",
            Self::Agents => "agents",
            Self::Skills => "skills",
            Self::Mcp => "mcp",
            Self::Updates => "updates",
            Self::Diagnostics => "diagnostics",
            Self::About => "about",
            Self::Archived => "archived",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Account => crate::i18n::text("account.title"),
            Self::General => crate::i18n::text("ui.general"),
            Self::Missions => crate::i18n::text("ui.missions"),
            Self::Appearance => crate::i18n::text("ui.appearance"),
            Self::Remote => crate::i18n::text("ui.remote_access"),
            Self::Notifications => crate::i18n::text("ui.notifications"),
            Self::Terminal => crate::i18n::text("ui.terminal"),
            Self::Shortcuts => crate::i18n::text("ui.keyboard_shortcuts"),
            Self::Agents => crate::i18n::text("ui.agents"),
            Self::Skills => crate::i18n::text("ui.skills"),
            Self::Mcp => "MCP",
            Self::Updates => crate::i18n::text("ui.updates"),
            Self::Diagnostics => crate::i18n::text("ui.diagnostics"),
            Self::About => crate::i18n::text("ui.about"),
            Self::Archived => crate::i18n::text("ui.archived"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Account => "user-round.svg",
            Self::General => "settings.svg",
            Self::Missions => "rocket.svg",
            Self::Appearance => "sun.svg",
            Self::Remote => "globe.svg",
            Self::Notifications => "bell.svg",
            Self::Terminal => "terminal.svg",
            Self::Shortcuts => "keyboard.svg",
            Self::Agents => "bot.svg",
            Self::Skills => "file-text.svg",
            Self::Mcp => "plug.svg",
            Self::Updates => "refresh-cw.svg",
            Self::Diagnostics => "file-text.svg",
            Self::About => "info.svg",
            Self::Archived => "archive.svg",
        }
    }
}

const APP_PANES: &[SettingsPane] = &[
    SettingsPane::Account,
    SettingsPane::General,
    SettingsPane::Appearance,
    SettingsPane::Notifications,
    SettingsPane::Remote,
    SettingsPane::Missions,
    SettingsPane::Terminal,
    SettingsPane::Shortcuts,
    SettingsPane::Archived,
];
const INTEGRATION_PANES: &[SettingsPane] = &[
    SettingsPane::Agents,
    SettingsPane::Skills,
    SettingsPane::Mcp,
];
const SYSTEM_PANES: &[SettingsPane] = &[
    SettingsPane::Updates,
    SettingsPane::Diagnostics,
    SettingsPane::About,
];
const NAV_GROUPS: &[(&str, &[SettingsPane])] = &[
    ("ui.app", APP_PANES),
    ("ui.integrations", INTEGRATION_PANES),
    ("ui.system", SYSTEM_PANES),
];

fn filtered_nav_groups(query: &str) -> Vec<(&'static str, Vec<SettingsPane>)> {
    let query = query.trim().to_lowercase();
    NAV_GROUPS
        .iter()
        .filter_map(|(label, panes)| {
            let panes = panes
                .iter()
                .copied()
                .filter(|pane| query.is_empty() || pane.label().to_lowercase().contains(&query))
                .collect::<Vec<_>>();
            (!panes.is_empty()).then_some((crate::i18n::text(label), panes))
        })
        .collect()
}

#[cfg(test)]
fn launch_dims_for(
    session_id: &str,
    direct_sizes: &HashMap<String, (u16, u16)>,
    mission_size: (u16, u16),
) -> Option<(u16, u16)> {
    direct_sizes.get(session_id).copied().or(Some(mission_size))
}

#[derive(Clone, Copy)]
enum SettingsSelection {
    Language,
    DefaultCrew,
    LightTheme,
    DarkTheme,
    LightTerminalTheme,
    DarkTerminalTheme,
    TerminalFont,
    TerminalCursor,
    FileLinkEditor,
    QuitBehavior,
}

pub(crate) struct SettingsState {
    pub(crate) remote: settings::remote::RemotePaneState,
    pub(crate) pane: SettingsPane,
    focus: FocusHandle,
    search_query: String,
    search: Entity<TextField>,
    shortcut_query: String,
    shortcut_search: Entity<TextField>,
    shortcut_recording: Option<&'static str>,
    shortcut_conflict: Option<ShortcutConflict>,
    shortcut_recording_focus: FocusHandle,
    default_crew: Entity<StyledSelect>,
    default_working_dir: Entity<TextField>,
    working_dir_browse_focus: FocusHandle,
    light_theme: Entity<StyledSelect>,
    dark_theme: Entity<StyledSelect>,
    light_terminal_theme: Entity<StyledSelect>,
    dark_terminal_theme: Entity<StyledSelect>,
    terminal_font: Entity<StyledSelect>,
    terminal_cursor: Entity<StyledSelect>,
    file_link_editor: Entity<StyledSelect>,
    pub(crate) language: Entity<StyledSelect>,
    pub(crate) quit_behavior: Entity<StyledSelect>,
    agents: Option<Entity<settings::agents::AgentsPane>>,
    skills: Option<Entity<settings::skills::SkillsPane>>,
    mcp: Option<Entity<settings::mcp::McpPane>>,
    updates: Option<Entity<settings::updates::UpdatesPane>>,
    diagnostics: Option<Entity<settings::diagnostics::DiagnosticsPane>>,
    about: Option<Entity<settings::about::AboutPane>>,
    archived: Option<Entity<settings::archived::ArchivedPane>>,
    nav_scroll: ScrollHandle,
    nav_scrollbar: Entity<Scrollbar>,
    content_scroll: ScrollHandle,
    content_scrollbar: Entity<Scrollbar>,
    command_install_activity: Option<CommandInstallActivity>,
    command_install_error: Option<(CommandInstallActivity, PathBuf, String)>,
    save_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl SettingsState {
    pub(crate) fn new(
        root: Entity<NativeRoot>,
        settings: &AppSettings,
        cx: &mut Context<NativeRoot>,
    ) -> Self {
        let search = cx.new(|input_cx| {
            let mut input = TextField::new(
                input_cx.focus_handle(),
                "",
                crate::i18n::message("ui.search_settings"),
                false,
            )
            .text_size(theme::text_body());
            input.set_bare(true, input_cx);
            input
        });
        let shortcut_search = cx.new(|input_cx| {
            let mut input = TextField::new(
                input_cx.focus_handle(),
                "",
                crate::i18n::message("ui.search_shortcuts"),
                false,
            )
            .text_size(theme::text_body());
            input.set_bare(true, input_cx);
            input
        });
        let default_working_dir = cx.new(|input_cx| {
            working_dir_text_field(
                input_cx.focus_handle(),
                settings.default_working_dir.clone(),
                runner_app::ui::working_dir_placeholder(None, ""),
            )
            .truncate_unfocused()
        });
        let default_crew = settings_select(
            &root,
            "settings-default-crew",
            settings.default_crew_id.clone(),
            vec![SelectOption::new("", crate::i18n::message("ui.no_default"))],
            SettingsSelection::DefaultCrew,
            cx,
        );
        let light_theme = settings_select(
            &root,
            "settings-light-theme",
            light_theme_value(settings.light_app_theme),
            light_theme_options(),
            SettingsSelection::LightTheme,
            cx,
        );
        let dark_theme = settings_select(
            &root,
            "settings-dark-theme",
            dark_theme_value(settings.dark_app_theme),
            vec![
                SelectOption::new("carbon", "Runner").swatch(0x00ff9c),
                SelectOption::new("catppuccin-mocha", "Catppuccin Mocha").swatch(0xcba6f7),
            ],
            SettingsSelection::DarkTheme,
            cx,
        );
        let light_terminal_theme = settings_select(
            &root,
            "settings-light-terminal-theme",
            settings.light_terminal_theme.key(),
            light_terminal_theme_options(),
            SettingsSelection::LightTerminalTheme,
            cx,
        );
        let dark_terminal_theme = settings_select(
            &root,
            "settings-dark-terminal-theme",
            settings.dark_terminal_theme.key(),
            dark_terminal_theme_options(),
            SettingsSelection::DarkTerminalTheme,
            cx,
        );
        let terminal_font = settings_select(
            &root,
            "settings-terminal-font",
            terminal_font_value(settings.terminal_font_family),
            [
                ("JetBrains Mono", "JetBrains Mono"),
                ("Menlo", theme::SYSTEM_MONOSPACE_FONT),
            ]
            .into_iter()
            .map(|(value, label)| SelectOption::new(value, label))
            .collect(),
            SettingsSelection::TerminalFont,
            cx,
        );
        let terminal_cursor = settings_select(
            &root,
            "settings-terminal-cursor",
            terminal_cursor_value(settings.terminal_cursor_style),
            ["block", "underline", "bar"]
                .into_iter()
                .map(|value| {
                    let label = match value {
                        "block" => "ui.block",
                        "underline" => "ui.underline",
                        _ => "ui.bar",
                    };
                    SelectOption::new(value, crate::i18n::message(label))
                })
                .collect(),
            SettingsSelection::TerminalCursor,
            cx,
        );
        let file_link_editor = settings_select(
            &root,
            "settings-file-link-editor",
            settings.file_link_editor.key(),
            FileLinkEditor::ALL
                .into_iter()
                .map(|editor| {
                    SelectOption::new(
                        editor.key(),
                        if editor == FileLinkEditor::DefaultApp {
                            i18n::message("ui.default_app")
                        } else {
                            editor.label().into()
                        },
                    )
                })
                .collect(),
            SettingsSelection::FileLinkEditor,
            cx,
        );
        let language = settings_select(
            &root,
            "settings-language",
            settings.language.key(),
            i18n::Language::ALL
                .into_iter()
                .map(|language| SelectOption::new(language.key(), language.label_source()))
                .collect(),
            SettingsSelection::Language,
            cx,
        );
        let quit_behavior = settings_select(
            &root,
            "settings-quit-behavior",
            settings.quit_behavior.key(),
            runner_app::lifecycle::QuitBehavior::ALL
                .into_iter()
                .map(|value| {
                    SelectOption::new(value.key(), i18n::message(value.label_key()))
                        .description(i18n::message(value.description_key()))
                })
                .collect(),
            SettingsSelection::QuitBehavior,
            cx,
        );
        let owner = cx.entity_id();
        let nav_scroll = ScrollHandle::new();
        let nav_scrollbar = cx.new(|_| Scrollbar::app(nav_scroll.clone(), owner));
        let content_scroll = ScrollHandle::new();
        let content_scrollbar = cx.new(|_| Scrollbar::app(content_scroll.clone(), owner));

        let search_subscription = cx.observe(&search, |this, input, cx| {
            let query = input.read(cx).text().to_owned();
            if this.settings_page.search_query != query {
                this.settings_page.search_query = query;
                cx.notify();
            }
        });
        let shortcut_search_subscription = cx.observe(&shortcut_search, |this, input, cx| {
            let query = input.read(cx).text().to_owned();
            if this.settings_page.shortcut_query != query {
                this.settings_page.shortcut_query = query;
                cx.notify();
            }
        });
        let working_dir_subscription = cx.observe(&default_working_dir, |this, input, cx| {
            let value = input.read(cx).text().trim().to_owned();
            if this.settings(cx).default_working_dir != value {
                this.update_app_settings(cx, false, |settings| {
                    settings.default_working_dir = value;
                    true
                });
                this.schedule_settings_save(cx);
            }
        });
        Self {
            remote: settings::remote::RemotePaneState::new(cx),
            pane: SettingsPane::General,
            focus: cx.focus_handle(),
            search_query: String::new(),
            search,
            shortcut_query: String::new(),
            shortcut_search,
            shortcut_recording: None,
            shortcut_conflict: None,
            shortcut_recording_focus: cx.focus_handle(),
            default_crew,
            default_working_dir,
            working_dir_browse_focus: cx.focus_handle(),
            light_theme,
            dark_theme,
            light_terminal_theme,
            dark_terminal_theme,
            terminal_font,
            terminal_cursor,
            file_link_editor,
            language,
            quit_behavior,
            agents: None,
            skills: None,
            mcp: None,
            updates: None,
            diagnostics: None,
            about: None,
            archived: None,
            nav_scroll,
            nav_scrollbar,
            content_scroll,
            content_scrollbar,
            command_install_activity: None,
            command_install_error: None,
            save_generation: 0,
            _subscriptions: vec![
                search_subscription,
                shortcut_search_subscription,
                working_dir_subscription,
            ],
        }
    }
}

struct ShortcutConflict {
    id: &'static str,
    message: i18n::Text,
}

/// The content column: the platform's titlebar drag strip, if it draws one,
/// the padded scroll container with its centered column, and the scrollbar
/// in an overlay. The overlay's top inset keeps the thumb out from under the
/// traffic lights where the page draws its own strip, and its bottom inset
/// keeps it out of the window's bottom corner on every platform (#553). The
/// overlay has no id or handlers, so wheel and click pass through it to the
/// scroll container.
fn settings_content_column(
    titlebar_drag_area: Option<impl IntoElement>,
    content: Option<AnyElement>,
    scroll: &ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    zoom: f32,
) -> impl IntoElement {
    div()
        .relative()
        .min_w(px(0.))
        .min_h(px(0.))
        .h_full()
        .flex_1()
        .children(titlebar_drag_area)
        .child(
            div()
                .id("settings-content-scroll")
                .size_full()
                .overflow_y_scroll()
                .scrollbar_width(px(0.))
                .track_scroll(scroll)
                .px(rems(40. / 16.))
                .pb(rems(64. / 16.))
                .pt(rems(SETTINGS_CONTENT_TOP / 16.))
                .children(content.map(|content| {
                    div()
                        .mx_auto()
                        .w_full()
                        .max_w(rems(760. / 16.))
                        .child(content)
                })),
        )
        .child(
            div()
                .absolute()
                .top(px(SETTINGS_DRAG_INSET * zoom))
                .bottom(px(TITLEBAR_DRAG_HEIGHT * zoom))
                .left_0()
                .right_0()
                .child(
                    div()
                        .debug_selector(|| "SETTINGS_CONTENT_SCROLLBAR_TRACK".into())
                        .relative()
                        .size_full()
                        .child(scrollbar),
                ),
        )
}

pub(crate) fn settings_section_heading(label: impl Into<SharedString>) -> AnyElement {
    div()
        .px_1()
        .text_size(theme::text_meta())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::faint())
        .child(label.into())
        .into_any_element()
}

fn command_line_settings_row(
    title: AnyElement,
    description: &'static str,
    status_lines: Vec<AnyElement>,
    control: Option<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rems(24. / 16.))
        .px_4()
        .py_3()
        .child(
            div()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(rems(2. / 16.))
                .child(title)
                .child(
                    div()
                        .text_size(theme::text_meta())
                        .text_color(theme::muted())
                        .child(description),
                )
                .children(status_lines),
        )
        .children(control.map(|control| div().flex_none().child(control)))
        .into_any_element()
}

fn command_status_text(
    text: impl Into<SharedString>,
    color: gpui::Hsla,
    icon: Option<&'static str>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .text_size(theme::text_caption())
        .text_color(color)
        .children(icon.map(|icon| svg().path(icon).size(px(12.)).text_color(color)))
        .child(text.into())
        .into_any_element()
}

fn command_installed_line(path: &Path) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .text_size(theme::text_caption())
        .text_color(theme::accent())
        .child(
            svg()
                .path("check.svg")
                .size(px(12.))
                .text_color(theme::accent()),
        )
        .child(crate::i18n::text("ui.installed_at"))
        .child(
            div()
                .font_family(theme::UI_MONOSPACE_FONT)
                .child(path.display().to_string()),
        )
        .into_any_element()
}

fn runner_skill_status_line(
    enabled: bool,
    status: &crate::app_store::RunnerSkillStatus,
) -> Option<(String, bool)> {
    if let Some(folder) = &status.foreign {
        return Some((
            crate::i18n::tf(
                "format.was_not_installed_by_runner_and_is_left_alone",
                &[("path", (folder.display()).to_string())],
            ),
            true,
        ));
    }
    (enabled && !status.detected).then(|| {
        (
            crate::i18n::text(
                "ui.no_supported_agent_detected_yet_the_skill_is_added_when_one_is_installed",
            )
            .into(),
            false,
        )
    })
}

fn missions_settings_pane(default_crew: Entity<StyledSelect>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_5()
        .debug_selector(|| "SETTINGS_MISSIONS_PANE".into())
        .child(PaneHeader::new(crate::i18n::text("ui.missions"), ""))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(14. / 16.))
                .child(
                    div()
                        .debug_selector(|| "SETTINGS_MISSIONS_DEFAULTS_HEADING".into())
                        .text_size(theme::text_heading())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(crate::i18n::text("ui.defaults")),
                )
                .child(
                    div()
                        .debug_selector(|| "SETTINGS_MISSIONS_CARD".into())
                        .child(SettingsCard::new(vec![SettingsRow::new(
                            crate::i18n::text("ui.default_crew"),
                            div()
                                .debug_selector(|| "SETTINGS_DEFAULT_CREW".into())
                                .child(default_crew),
                        )
                        .subtitle(crate::i18n::text(
                            "ui.pre_selected_when_starting_a_new_mission",
                        ))
                        .into_any_element()])),
                ),
        )
        .into_any_element()
}

fn settings_select(
    root: &Entity<NativeRoot>,
    id: &'static str,
    value: impl Into<String>,
    options: Vec<SelectOption>,
    selection: SettingsSelection,
    cx: &mut Context<NativeRoot>,
) -> Entity<StyledSelect> {
    let root = root.clone();
    let handler: SelectHandler = Rc::new(move |value, window, cx| {
        root.update(cx, |this, root_cx| {
            this.apply_settings_selection(selection, &value, window, root_cx)
        });
    });
    let value = value.into();
    let width = match selection {
        SettingsSelection::Language
        | SettingsSelection::LightTheme
        | SettingsSelection::DarkTheme
        | SettingsSelection::LightTerminalTheme
        | SettingsSelection::DarkTerminalTheme => Some(SETTINGS_SELECT_WIDTH),
        SettingsSelection::DefaultCrew
        | SettingsSelection::TerminalFont
        | SettingsSelection::TerminalCursor
        | SettingsSelection::FileLinkEditor
        | SettingsSelection::QuitBehavior => None,
    };
    cx.new(|select_cx| {
        let select = StyledSelect::new(
            id,
            select_cx.focus_handle(),
            value,
            options,
            handler,
            select_cx,
        );
        let select = if matches!(selection, SettingsSelection::QuitBehavior) {
            select.min_menu_width(px(300.))
        } else {
            select
        };
        match width {
            Some(width) => select.width(width),
            None => select,
        }
    })
}

impl NativeRoot {
    pub(crate) fn reset_settings_scroll(&self, cx: &mut Context<Self>) {
        self.settings_page
            .content_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    pub(crate) fn focus_settings_page(&self, window: &mut Window, cx: &mut App) {
        self.settings_page.focus.focus(window, cx);
    }

    pub(crate) fn shortcut_recording_active(&self) -> bool {
        self.settings_page.shortcut_recording.is_some()
    }

    fn refresh_settings_crews(&self, cx: &mut Context<Self>) {
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            core.crew_list_all()
                .map(|crews| {
                    crews
                        .into_iter()
                        .map(|item| (item.crew.id, item.crew.name))
                        .collect::<Vec<_>>()
                })
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.route != AppRoute::Settings
                    || this.settings_page.pane != SettingsPane::Missions
                {
                    return;
                }
                let Ok(crews) = result else { return };
                let mut options =
                    vec![SelectOption::new("", crate::i18n::message("ui.no_default"))];
                options.extend(
                    crews
                        .into_iter()
                        .map(|(id, name)| SelectOption::new(id, name)),
                );
                this.settings_page
                    .default_crew
                    .update(cx, |select, select_cx| {
                        select.set_options(options, select_cx)
                    });
            });
        })
        .detach();
    }

    fn schedule_settings_save(&mut self, cx: &mut Context<Self>) {
        self.settings_page.save_generation = self.settings_page.save_generation.wrapping_add(1);
        let generation = self.settings_page.save_generation;
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SETTINGS_SAVE_DELAY_MS))
                .await;
            let _ = weak.update(cx, |this, root_cx| {
                if this.settings_page.save_generation == generation {
                    this.save_settings(root_cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn enter_settings_pane(
        &mut self,
        pane: SettingsPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_page.pane != pane {
            self.dismiss_remote_dialog();
        }
        if self.settings_page.pane == SettingsPane::Updates && pane != SettingsPane::Updates {
            self.pause_updates_pane(cx);
        }
        if pane != SettingsPane::Shortcuts && self.settings_page.shortcut_recording.is_some() {
            self.finish_shortcut_recording(window, cx);
        }
        if self.route != AppRoute::Settings
            || (self.settings_page.pane == SettingsPane::Shortcuts)
                != (pane == SettingsPane::Shortcuts)
        {
            self.settings_page.shortcut_query.clear();
            self.settings_page
                .shortcut_search
                .update(cx, |input, input_cx| input.reset("", input_cx));
        }
        if self.route != AppRoute::Settings {
            self.app_store.update(cx, |store, _| {
                store.refresh_runner_command_status();
                store.refresh_runner_skill_status();
            });
            self.settings_return_route = self.route.clone();
            self.settings_page.search_query.clear();
            self.settings_page
                .search
                .update(cx, |input, input_cx| input.reset("", input_cx));
            self.dismiss_sidebar_transients(cx);
            self.set_route(AppRoute::Settings, cx);
        }
        self.settings_page.pane = pane;
        match pane {
            SettingsPane::Account | SettingsPane::Remote => self.enter_remote_settings(cx),
            SettingsPane::Missions => self.refresh_settings_crews(cx),
            SettingsPane::Agents => {
                if self.settings_page.agents.is_none() {
                    let shell = cx.entity().downgrade();
                    let app_store = self.app_store.clone();
                    self.settings_page.agents = Some(cx.new(|pane_cx| {
                        settings::agents::AgentsPane::new(shell, app_store, window, pane_cx)
                    }));
                }
                if let Some(agents) = self.settings_page.agents.clone() {
                    agents.update(cx, |pane, pane_cx| {
                        pane.show_list(pane_cx);
                        pane.refresh(pane_cx);
                    });
                }
                let _ = self.core(cx).runtime_check_updates(false);
            }
            SettingsPane::Skills => {
                if self.settings_page.skills.is_none() {
                    let app_store = self.app_store.clone();
                    let pane = cx.new(|cx| settings::skills::SkillsPane::new(app_store, cx));
                    let detail = pane.read(cx).detail.clone();
                    self.settings_page._subscriptions.push(cx.subscribe(
                        &detail,
                        |this, _, notice: &settings::SaveNotice, cx| {
                            this.show_toast(notice.message.clone(), notice.tone, cx)
                        },
                    ));
                    self.settings_page.skills = Some(pane);
                }
                if let Some(skills) = self.settings_page.skills.clone() {
                    skills.update(cx, |pane, cx| pane.refresh(cx));
                }
            }
            SettingsPane::Mcp => {
                if self.settings_page.mcp.is_none() {
                    let app_store = self.app_store.clone();
                    let pane = cx.new(|cx| settings::mcp::McpPane::new(app_store, cx));
                    let detail = pane.read(cx).detail.clone();
                    self.settings_page._subscriptions.push(cx.subscribe(
                        &detail,
                        |this, _, notice: &settings::SaveNotice, cx| {
                            this.show_toast(notice.message.clone(), notice.tone, cx)
                        },
                    ));
                    self.settings_page.mcp = Some(pane);
                }
                if let Some(mcp) = self.settings_page.mcp.clone() {
                    mcp.update(cx, |pane, cx| pane.refresh(cx));
                }
            }
            SettingsPane::Updates => {
                if self.settings_page.updates.is_none() {
                    let app_store = self.app_store.clone();
                    let updater = runner_app::updater::global_updater(cx);
                    self.settings_page.updates = Some(cx.new(|pane_cx| {
                        settings::updates::UpdatesPane::new(app_store, updater, pane_cx)
                    }));
                }
                if let Some(updates) = self.settings_page.updates.clone() {
                    updates.update(cx, |pane, pane_cx| pane.refresh(pane_cx));
                }
            }
            SettingsPane::Diagnostics => {
                if self.settings_page.diagnostics.is_none() {
                    let log_dir = self.log_dir.clone();
                    self.settings_page.diagnostics =
                        Some(cx.new(move |_| settings::diagnostics::DiagnosticsPane::new(log_dir)));
                }
            }
            SettingsPane::About => {
                if self.settings_page.about.is_none() {
                    self.settings_page.about = Some(cx.new(|_| settings::about::AboutPane::new()));
                }
            }
            SettingsPane::Archived => {
                if self.settings_page.archived.is_none() {
                    let shell = cx.entity().downgrade();
                    let app_store = self.app_store.clone();
                    self.settings_page.archived = Some(cx.new(|pane_cx| {
                        settings::archived::ArchivedPane::new(shell, app_store, pane_cx)
                    }));
                }
                if let Some(archived) = self.settings_page.archived.clone() {
                    archived.update(cx, |pane, pane_cx| pane.refresh(pane_cx));
                }
            }
            SettingsPane::General
            | SettingsPane::Appearance
            | SettingsPane::Terminal
            | SettingsPane::Shortcuts
            | SettingsPane::Notifications => {}
        }
        window.focus(&self.settings_page.focus, cx);
        cx.notify();
    }

    pub(crate) fn enter_settings_route(
        &mut self,
        pane: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.enter_settings_pane(SettingsPane::from_route(pane), window, cx);
    }

    fn apply_settings_selection(
        &mut self,
        selection: SettingsSelection,
        value: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed = self.update_app_settings(cx, true, |settings| match selection {
            SettingsSelection::Language => i18n::Language::ALL
                .into_iter()
                .find(|language| language.key() == value)
                .is_some_and(|language| update_if_changed(&mut settings.language, language)),
            SettingsSelection::DefaultCrew => {
                update_if_changed(&mut settings.default_crew_id, value.to_owned())
            }
            SettingsSelection::LightTheme => parse_light_theme(value)
                .is_some_and(|value| update_if_changed(&mut settings.light_app_theme, value)),
            SettingsSelection::DarkTheme => parse_dark_theme(value)
                .is_some_and(|value| update_if_changed(&mut settings.dark_app_theme, value)),
            SettingsSelection::LightTerminalTheme => LightTerminalTheme::parse(value)
                .is_some_and(|value| update_if_changed(&mut settings.light_terminal_theme, value)),
            SettingsSelection::DarkTerminalTheme => DarkTerminalTheme::parse(value)
                .is_some_and(|value| update_if_changed(&mut settings.dark_terminal_theme, value)),
            SettingsSelection::TerminalFont => parse_terminal_font(value)
                .is_some_and(|value| update_if_changed(&mut settings.terminal_font_family, value)),
            SettingsSelection::TerminalCursor => parse_terminal_cursor(value)
                .is_some_and(|value| update_if_changed(&mut settings.terminal_cursor_style, value)),
            SettingsSelection::QuitBehavior => runner_app::lifecycle::QuitBehavior::ALL
                .into_iter()
                .find(|pick| pick.key() == value)
                .is_some_and(|value| update_if_changed(&mut settings.quit_behavior, value)),
            SettingsSelection::FileLinkEditor => FileLinkEditor::parse(value)
                .is_some_and(|value| update_if_changed(&mut settings.file_link_editor, value)),
        });
        if !changed {
            if matches!(selection, SettingsSelection::Language) {
                self.app_store.update(cx, |store, cx| store.sync_locale(cx));
            }
            return;
        }
        cx.notify();
    }

    fn set_theme_intent(&mut self, intent: ThemeIntent, cx: &mut Context<Self>) {
        if self.update_app_settings(cx, true, |settings| {
            update_if_changed(&mut settings.app_theme, intent)
        }) {
            cx.notify();
        }
    }

    fn set_terminal_font_size(&mut self, size: u16, cx: &mut Context<Self>) {
        let size = size.clamp(TERMINAL_FONT_SIZE_MIN, TERMINAL_FONT_SIZE_MAX);
        if self.update_app_settings(cx, true, |settings| {
            update_if_changed(&mut settings.terminal_font_size, size)
        }) {
            cx.notify();
        }
    }

    fn set_resume_on_launch(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.update_app_settings(cx, true, |settings| {
            update_if_changed(&mut settings.resume_on_launch, enabled)
        }) {
            cx.notify();
        }
    }

    fn set_runner_skill_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if !self.update_app_settings(cx, true, |settings| {
            update_if_changed(&mut settings.runner_skill_enabled, enabled)
        }) {
            return;
        }
        self.app_store
            .update(cx, |store, _| store.initialize_skill_defaults());
        cx.notify();
    }

    pub(crate) fn install_runner_command(&mut self, cx: &mut Context<Self>) {
        self.run_runner_command_action(CommandInstallActivity::Install, cx);
    }

    pub(crate) fn uninstall_runner_command(&mut self, cx: &mut Context<Self>) {
        self.run_runner_command_action(CommandInstallActivity::Uninstall, cx);
    }

    fn run_runner_command_action(
        &mut self,
        activity: CommandInstallActivity,
        cx: &mut Context<Self>,
    ) {
        if self.settings_page.command_install_activity.is_some() {
            return;
        }
        let (inputs, integration, target) = {
            let store = self.app_store.read(cx);
            let Some(inputs) = store.command_install_inputs(false) else {
                return;
            };
            let Some(integration) = store.command_install_integration() else {
                return;
            };
            let target = if activity == CommandInstallActivity::Uninstall {
                store
                    .runner_command_status()
                    .and_then(|status| status.path.clone())
            } else {
                store.command_action_target().cloned()
            }
            .unwrap_or_else(|| inputs.sidecar.clone());
            (inputs, integration, target)
        };
        self.update_app_settings(cx, true, crate::app_store::mark_command_install_initialized);
        self.settings_page.command_install_activity = Some(activity);
        self.settings_page.command_install_error = None;
        let action = match activity {
            CommandInstallActivity::Install => crate::app_store::UserCommandAction::Install,
            CommandInstallActivity::Uninstall => crate::app_store::UserCommandAction::Uninstall,
        };
        let client = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            crate::app_store::run_user_command_action(&client, &inputs, action, integration)
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.settings_page.command_install_activity = None;
                this.app_store
                    .update(cx, |store, _| store.refresh_runner_command_status());
                match result {
                    Ok(CommandActionOutcome::Cancelled) => {}
                    Ok(_) => this.settings_page.command_install_error = None,
                    Err(error) => {
                        this.settings_page.command_install_error =
                            Some((activity, target, error.to_string()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn apply_terminal_settings(&self, cx: &mut Context<Self>) {
        let cursor = match self.settings(cx).terminal_cursor_style {
            TerminalCursorStyle::Block => alacritty_terminal::vte::ansi::CursorShape::Block,
            TerminalCursorStyle::Underline => alacritty_terminal::vte::ansi::CursorShape::Underline,
            TerminalCursorStyle::Bar => alacritty_terminal::vte::ansi::CursorShape::Beam,
        };
        self.apply_terminal_palette(cx);
        for chat in self.attached.values() {
            chat.terminal.configure(TERMINAL_SCROLLBACK_LINES, cursor);
        }
    }

    fn browse_settings_working_dir(&mut self, cx: &mut Context<Self>) {
        let input = self.settings_page.default_working_dir.clone();
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(crate::i18n::text("ui.pick_a_working_directory").into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = selected.await.ok().and_then(|result| result.ok()).flatten();
            let _ = weak.update(cx, |this, cx| {
                if this.settings_page.default_working_dir != input {
                    return;
                }
                if let Some(path) = result.and_then(|paths| paths.into_iter().next()) {
                    input.update(cx, |input, input_cx| {
                        input.reset(path.to_string_lossy().into_owned(), input_cx)
                    });
                }
            });
        })
        .detach();
    }

    pub(crate) fn render_settings_takeover(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let zoom = self.settings(cx).app_zoom;
        let width = self.settings(cx).sidebar_width * zoom;
        let groups = filtered_nav_groups(&self.settings_page.search_query);
        let active = self.settings_page.pane;
        let nav = if groups.is_empty() {
            vec![div()
                .px(rems(10. / 16.))
                .py_1()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(crate::i18n::text("ui.no_matching_settings"))
                .into_any_element()]
        } else {
            groups
                .into_iter()
                .map(|(label, panes)| self.render_settings_nav_group(label, panes, active, cx))
                .collect()
        };
        let content = self.render_settings_pane(cx);

        div()
            .absolute()
            .debug_selector(|| "SETTINGS_CHROME".into())
            .key_context("Settings")
            .track_focus(&self.settings_page.focus)
            .when(active == SettingsPane::Agents, |settings| {
                settings.on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "tab"
                        && !this.settings_page.search.read(cx).is_composing()
                        && !this
                            .settings_page
                            .agents
                            .as_ref()
                            .is_some_and(|pane| pane.read(cx).is_composing(cx))
                    {
                        cx.stop_propagation();
                        if event.keystroke.modifiers.shift {
                            window.focus_prev(cx);
                        } else {
                            window.focus_next(cx);
                        }
                    }
                }))
            })
            .inset_0()
            .flex()
            .overflow_hidden()
            .bg(gpui::transparent_black())
            .occlude()
            .child(
                div()
                    .relative()
                    .w(px(width))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .bg(if theme::is_glass() {
                        gpui::transparent_black()
                    } else {
                        theme::sidebar()
                    })
                    .children(self.render_sidebar_titlebar(window, cx))
                    .child(
                        div()
                            .flex_none()
                            .px_4()
                            .pb_3()
                            .pt_1()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(self.render_settings_back_button(cx))
                            .child(
                                div()
                                    .h(rems(32. / 16.))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .rounded(rems(6. / 16.))
                                    .border_1()
                                    .border_color(theme::chrome_selected_border())
                                    .bg(theme::chrome_selected())
                                    .px(rems(10. / 16.))
                                    .child(
                                        svg()
                                            .path("search.svg")
                                            .size(rems(14. / 16.))
                                            .flex_none()
                                            .text_color(theme::faint()),
                                    )
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .flex_1()
                                            .child(self.settings_page.search.clone()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .relative()
                            .min_h(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .id("settings-nav-scroll")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .scrollbar_width(px(0.))
                                    .track_scroll(&self.settings_page.nav_scroll)
                                    .px_3()
                                    .py_2()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .children(nav),
                            )
                            .child(self.settings_page.nav_scrollbar.clone()),
                    ),
            )
            .child(
                runner_app::ui::work_card::work_card(false)
                    .children(self.render_daemon_banner(cx).map(|banner| {
                        div()
                            .flex_none()
                            .pt(rems(SETTINGS_DRAG_INSET / 16.))
                            .child(banner)
                    }))
                    .child(settings_content_column(
                        self.render_settings_titlebar_drag_area(
                            "settings-content-titlebar-drag",
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .h(px(TITLEBAR_DRAG_HEIGHT * zoom)),
                            cx,
                        ),
                        content,
                        &self.settings_page.content_scroll,
                        self.settings_page.content_scrollbar.clone(),
                        zoom,
                    )),
            )
            .child(self.render_sidebar_resize_handle(width, cx))
            .when(active == SettingsPane::Skills, |takeover| {
                takeover.children(
                    self.settings_page
                        .skills
                        .as_ref()
                        .map(|pane| pane.read(cx).detail.clone()),
                )
            })
            .when(active == SettingsPane::Mcp, |takeover| {
                takeover.children(
                    self.settings_page
                        .mcp
                        .as_ref()
                        .map(|pane| pane.read(cx).detail.clone()),
                )
            })
            .into_any_element()
    }

    fn render_settings_back_button(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("settings-back")
            .debug_selector(|| "SETTINGS_BACK".into())
            .group("settings-back")
            .px_2()
            .py(rems(6. / 16.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_sm()
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .text_color(theme::muted())
            .hover(|button| {
                button
                    .border_color(theme::chrome_selected_border())
                    .bg(theme::chrome_hover())
                    .text_color(theme::text())
            })
            .child(
                svg()
                    .path("arrow-left.svg")
                    .size(rems(14. / 16.))
                    .flex_none()
                    .text_color(theme::muted())
                    .group_hover("settings-back", |icon| icon.text_color(theme::text())),
            )
            .child(
                div()
                    .text_size(theme::text_body())
                    .child(crate::i18n::text("ui.back_to_app")),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.leave_settings(window, cx);
            }))
            .into_any_element()
    }

    fn render_settings_nav_group(
        &self,
        label: &'static str,
        panes: Vec<SettingsPane>,
        active: SettingsPane,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(rems(2. / 16.))
            .child(
                div()
                    .px(rems(10. / 16.))
                    .pb_1()
                    .text_size(theme::text_caption())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::faint())
                    .child(label.to_uppercase()),
            )
            .children(
                panes
                    .into_iter()
                    .map(|pane| self.render_settings_nav_item(pane, pane == active, cx)),
            )
            .into_any_element()
    }

    fn render_settings_nav_item(
        &self,
        pane: SettingsPane,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let key_root = root.clone();
        div()
            .id(SharedString::from(format!("settings-nav-{}", pane.key())))
            .tab_index(0)
            .px(rems(10. / 16.))
            .py_1()
            .flex()
            .items_center()
            .gap(rems(10. / 16.))
            .rounded_sm()
            .border_1()
            .border_color(if active {
                theme::chrome_selected_border()
            } else {
                gpui::transparent_black()
            })
            .bg(if active {
                theme::chrome_selected()
            } else {
                gpui::transparent_black()
            })
            .font_weight(if active {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .text_size(theme::text_title())
            .text_color(if active {
                theme::text()
            } else {
                theme::muted()
            })
            .cursor_pointer()
            // A shadow paints its whole box, which shows through glass's translucent fill.
            .when(active && !theme::is_glass(), |row| row.shadow_sm())
            .when(!active, |row| {
                row.hover(|row| {
                    row.border_color(theme::chrome_selected_border())
                        .bg(theme::chrome_hover())
                        .text_color(theme::text())
                })
            })
            .focus_visible(|row| {
                row.shadow(vec![BoxShadow {
                    color: theme::border_strong(),
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: false,
                }])
            })
            .child(
                svg()
                    .path(pane.icon())
                    .size(rems(14. / 16.))
                    .flex_none()
                    .text_color(if active {
                        theme::text()
                    } else {
                        theme::muted()
                    }),
            )
            .child(div().min_w(px(0.)).truncate().child(pane.label()))
            .on_click(move |_, window, cx| {
                root.update(cx, |this, root_cx| {
                    this.enter_settings_pane(pane, window, root_cx)
                });
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    key_root.update(cx, |this, root_cx| {
                        this.enter_settings_pane(pane, window, root_cx)
                    });
                }
            })
            .into_any_element()
    }

    fn render_settings_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.settings_page.pane {
            SettingsPane::General => Some(self.render_general_settings(cx)),
            SettingsPane::Missions => Some(self.render_missions_settings(cx)),
            SettingsPane::Appearance => Some(self.render_appearance_settings(cx)),
            SettingsPane::Account => Some(self.render_account_settings(cx)),
            SettingsPane::Remote => Some(self.render_remote_settings(cx)),
            SettingsPane::Notifications => Some(self.render_notifications_settings(cx)),
            SettingsPane::Terminal => Some(self.render_terminal_settings(cx)),
            SettingsPane::Shortcuts => Some(self.render_shortcuts_settings(cx)),
            SettingsPane::Agents => self
                .settings_page
                .agents
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::Skills => self
                .settings_page
                .skills
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::Mcp => self
                .settings_page
                .mcp
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::Updates => self
                .settings_page
                .updates
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::Diagnostics => self
                .settings_page
                .diagnostics
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::About => self
                .settings_page
                .about
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::Archived => self
                .settings_page
                .archived
                .clone()
                .map(IntoElement::into_any_element),
        }
    }

    pub(crate) fn open_claude_accounts(&self, cx: &mut Context<Self>) {
        if let Some(agents) = &self.settings_page.agents {
            agents.update(cx, |agents, cx| {
                agents.open_runtime(runner_core::protocol::Runtime::ClaudeCode, cx)
            });
        }
    }

    pub(crate) fn refresh_agents_pane(&self, cx: &mut Context<Self>) {
        if let Some(agents) = self.settings_page.agents.clone() {
            agents.update(cx, |pane, pane_cx| pane.refresh(pane_cx));
        }
    }

    /// Keeps the Update guard's running-session count live while the
    /// Agents pane is on screen.
    pub(crate) fn refresh_agents_live_sessions(&self, cx: &mut Context<Self>) {
        if self.route != AppRoute::Settings || self.settings_page.pane != SettingsPane::Agents {
            return;
        }
        if let Some(agents) = self.settings_page.agents.clone() {
            agents.update(cx, |pane, pane_cx| pane.refresh_live_sessions(pane_cx));
        }
    }

    pub(crate) fn pause_updates_pane(&self, cx: &mut Context<Self>) {
        if let Some(updates) = self.settings_page.updates.clone() {
            updates.update(cx, |pane, _| pane.pause());
        }
    }

    pub(crate) fn settings_confirm_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.route != AppRoute::Settings {
            return None;
        }
        if self.remote_overlay_open() {
            return Some(self.remote_confirm_overlay(cx));
        }
        if self.settings_page.pane == SettingsPane::Agents {
            return self
                .settings_page
                .agents
                .as_ref()
                .map(|agents| agents.read(cx).accounts_overlay(cx));
        }
        self.settings_page
            .archived
            .as_ref()
            .map(|archived| archived.read(cx).confirm_overlay().into_any_element())
    }

    fn render_shortcuts_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let overrides = self.settings(cx).keymap_overrides.clone();
        let rows = shortcut_rows(&self.settings_page.shortcut_query, &overrides)
            .into_iter()
            .map(|row| self.render_shortcut_row(row, &overrides, cx))
            .collect::<Vec<_>>();
        let has_overrides = keymap::entries()
            .iter()
            .any(|entry| keymap::is_overridden(entry.id, &overrides));
        let reset_root = cx.entity();
        let reset = Button::new(
            "reset-keymap",
            crate::i18n::text("ui.reset_all_to_defaults"),
        )
        .icon("rotate-ccw.svg")
        .size(ButtonSize::Sm)
        .disabled(!has_overrides)
        .on_press(move |window, cx| {
            reset_root.update(cx, |this, root_cx| {
                this.reset_shortcut_overrides(window, root_cx)
            });
        });
        let content = if rows.is_empty() {
            div()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(crate::i18n::tf(
                    "format.no_shortcuts_match",
                    &[(
                        "query",
                        (self.settings_page.shortcut_query.trim()).to_string(),
                    )],
                ))
                .into_any_element()
        } else {
            SettingsCard::new(rows).into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                PaneHeader::new(
                    crate::i18n::text("ui.keyboard_shortcuts"),
                    crate::i18n::text(crate::platform_ui::SHORTCUT_REQUIREMENTS_KEY),
                )
                .action(reset),
            )
            .child(
                div()
                    .h(rems(36. / 16.))
                    .w(rems(280. / 16.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(rems(6. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .px(rems(10. / 16.))
                    .child(
                        svg()
                            .path("search.svg")
                            .size(rems(14. / 16.))
                            .flex_none()
                            .text_color(theme::faint()),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .child(self.settings_page.shortcut_search.clone()),
                    ),
            )
            .child(content)
            .into_any_element()
    }

    fn render_shortcut_row(
        &self,
        row: ShortcutRow,
        overrides: &keymap::KeymapOverrides,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = row.id();
        let overridden = row.overridden(overrides);
        let recording = self.settings_page.shortcut_recording == Some(id);
        let conflict = self
            .settings_page
            .shortcut_conflict
            .as_ref()
            .filter(|conflict| conflict.id == id)
            .map(|conflict| conflict.message.resolve().to_string());
        let label = shortcut_row_label(row.title(), row.description(), conflict);

        if recording {
            let key_root = cx.entity();
            let outside_root = key_root.clone();
            return div()
                .min_h(rems(58. / 16.))
                .px_4()
                .py_3()
                .flex()
                .items_center()
                .gap_3()
                .child(label)
                .child(
                    div()
                        .id(SharedString::from(format!("shortcut-recorder-{}", id)))
                        .track_focus(&self.settings_page.shortcut_recording_focus)
                        .h_8()
                        .min_w(px(0.))
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(rems(6. / 16.))
                        .border_1()
                        .border_color(theme::border_strong())
                        .bg(theme::bg())
                        .text_size(theme::text_ui())
                        .text_color(theme::muted())
                        .child(crate::i18n::text("ui.press_keys"))
                        .on_key_down(move |event: &KeyDownEvent, window, cx| {
                            key_root.update(cx, |this, root_cx| {
                                this.record_shortcut_key(id, event, window, root_cx)
                            });
                        })
                        .on_mouse_down_out(move |_, window, cx| {
                            outside_root.update(cx, |this, root_cx| {
                                this.finish_shortcut_recording(window, root_cx)
                            });
                        }),
                )
                .child(div().w_6().flex_none())
                .into_any_element();
        }

        let binding_label = row.binding_label(overrides);
        let restore = overridden.then(|| {
            let restore_root = cx.entity();
            IconButton::new(format!("restore-shortcut-{id}"), "rotate-ccw.svg")
                .size(IconButtonSize::Sm)
                .tooltip(crate::i18n::text("ui.restore_default"))
                .on_press(move |window, cx| {
                    restore_root.update(cx, |this, root_cx| {
                        this.restore_shortcut_default(row, window, root_cx)
                    });
                })
        });
        let edit = if row.fixed() {
            div().w_6().flex_none().into_any_element()
        } else {
            let edit_root = cx.entity();
            IconButton::new(format!("edit-shortcut-{id}"), "pencil.svg")
                .size(IconButtonSize::Sm)
                .tooltip(crate::i18n::text("ui.edit_shortcut"))
                .on_press(move |window, cx| {
                    edit_root.update(cx, |this, root_cx| {
                        this.start_shortcut_recording(id, window, root_cx)
                    });
                })
                .into_any_element()
        };
        let chip = if row.fixed() {
            shortcut_chip(binding_label, false).into_any_element()
        } else {
            let chip_root = cx.entity();
            shortcut_chip(binding_label, true)
                .id(SharedString::from(format!("binding-shortcut-{id}")))
                .on_click(move |_, window, cx| {
                    chip_root.update(cx, |this, root_cx| {
                        this.start_shortcut_recording(id, window, root_cx)
                    });
                })
                .into_any_element()
        };
        let controls = div()
            .min_w(px(0.))
            .flex_1()
            .flex()
            .items_center()
            .justify_between()
            .gap(rems(6. / 16.))
            .child(chip)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .children(restore)
                    .child(edit),
            );
        let unbind_root = cx.entity();
        let unbind = IconButton::new(format!("unbind-shortcut-{id}"), "trash.svg")
            .size(IconButtonSize::Sm)
            .tooltip(crate::i18n::text("ui.unbind_shortcut"))
            .on_press(move |window, cx| {
                unbind_root.update(cx, |this, root_cx| {
                    this.set_shortcut_override(&row.entry_ids(), None, window, root_cx)
                });
            });

        div()
            .min_h(rems(58. / 16.))
            .px_4()
            .py_3()
            .flex()
            .items_center()
            .gap_3()
            .child(label)
            .child(controls)
            .child(unbind)
            .into_any_element()
    }

    fn rebuild_key_bindings(&self, suspended: bool, cx: &mut Context<Self>) {
        let overrides = self.settings(cx).keymap_overrides.clone();
        keymap::install_bindings(cx, &overrides, suspended);
        cx.set_menus(crate::app_menus());
    }

    fn start_shortcut_recording(
        &mut self,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if keymap::entry(id).is_none_or(|entry| entry.fixed) {
            return;
        }
        self.settings_page.shortcut_recording = Some(id);
        self.settings_page.shortcut_conflict = None;
        self.rebuild_key_bindings(true, cx);
        self.settings_page
            .shortcut_recording_focus
            .focus(window, cx);
        cx.notify();
    }

    pub(crate) fn finish_shortcut_recording(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_page.shortcut_recording.take().is_none() {
            return;
        }
        self.settings_page.shortcut_conflict = None;
        self.rebuild_key_bindings(false, cx);
        self.settings_page.focus.focus(window, cx);
        cx.notify();
    }

    fn record_shortcut_key(
        &mut self,
        id: &'static str,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if event.keystroke.key == "escape" {
            self.finish_shortcut_recording(window, cx);
            return;
        }
        let Some(combo) = keymap::combo_from_keystroke(&event.keystroke) else {
            return;
        };
        if let Some(conflict) =
            keymap::find_conflict(&combo, id, &self.settings(cx).keymap_overrides)
        {
            self.settings_page.shortcut_conflict = Some(ShortcutConflict {
                id,
                message: crate::i18n::message_with(
                    "format.already_used_by",
                    &[("command", i18n::message(conflict.title_key))],
                ),
            });
            cx.notify();
            return;
        }
        self.set_shortcut_override(&[id], Some(combo), window, cx);
    }

    fn set_shortcut_override(
        &mut self,
        ids: &[&'static str],
        value: Option<keymap::KeyCombo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ids
            .iter()
            .any(|id| keymap::entry(id).is_none_or(|entry| entry.fixed && value.is_some()))
        {
            return;
        }
        self.update_app_settings(cx, true, |settings| {
            let mut changed = false;
            for id in ids {
                if settings.keymap_overrides.get(*id) != Some(&value) {
                    settings
                        .keymap_overrides
                        .insert((*id).into(), value.clone());
                    changed = true;
                }
            }
            changed
        });
        self.settings_page.shortcut_recording = None;
        self.settings_page.shortcut_conflict = None;
        self.rebuild_key_bindings(false, cx);
        self.settings_page.focus.focus(window, cx);
        cx.notify();
    }

    fn restore_shortcut_default(
        &mut self,
        row: ShortcutRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut overrides = self.settings(cx).keymap_overrides.clone();
        let mut changed = false;
        for id in row.entry_ids() {
            match keymap::clear_override(id, &mut overrides) {
                Err(conflict) => {
                    self.settings_page.shortcut_conflict = Some(ShortcutConflict {
                        id: row.id(),
                        message: crate::i18n::message_with(
                            "format.already_used_by",
                            &[("command", i18n::message(conflict.title_key))],
                        ),
                    });
                    cx.notify();
                    return;
                }
                Ok(cleared) => changed |= cleared,
            }
        }
        if !changed {
            return;
        }
        self.update_app_settings(cx, true, |settings| {
            settings.keymap_overrides = overrides;
            true
        });
        self.settings_page.shortcut_conflict = None;
        self.rebuild_key_bindings(false, cx);
        self.settings_page.focus.focus(window, cx);
        cx.notify();
    }

    fn reset_shortcut_overrides(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings(cx).keymap_overrides.is_empty() {
            return;
        }
        self.update_app_settings(cx, true, |settings| {
            settings.keymap_overrides.clear();
            true
        });
        self.settings_page.shortcut_recording = None;
        self.settings_page.shortcut_conflict = None;
        self.rebuild_key_bindings(false, cx);
        self.settings_page.focus.focus(window, cx);
        cx.notify();
    }

    fn render_general_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let zoom_value = normalize_zoom(self.settings(cx).app_zoom);
        let zoom_index = ZOOM_STEPS
            .iter()
            .position(|step| *step == zoom_value)
            .unwrap_or(2);
        let decrement_root = cx.entity();
        let increment_root = decrement_root.clone();
        let decrement: StepHandler = Rc::new(move |window, cx| {
            decrement_root.update(cx, |this, root_cx| {
                this.set_zoom(nudge_zoom(zoom_value, -1), window, root_cx)
            });
        });
        let increment: StepHandler = Rc::new(move |window, cx| {
            increment_root.update(cx, |this, root_cx| {
                this.set_zoom(nudge_zoom(zoom_value, 1), window, root_cx)
            });
        });
        let zoom = Stepper::new(
            "settings-app-zoom",
            56.,
            div()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_ui())
                .font_weight(FontWeight::MEDIUM)
                .child(format!(
                    "{}%",
                    (ZOOM_STEPS[zoom_index] * 100.).round() as u16
                )),
            decrement,
            increment,
        )
        .decrement_disabled(zoom_index == 0)
        .increment_disabled(zoom_index == ZOOM_STEPS.len() - 1);
        let browse_root = cx.entity();
        let working_dir = WorkingDirField::new(
            self.settings_page.default_working_dir.clone(),
            false,
            Rc::new(move |_, cx| {
                browse_root.update(cx, |this, root_cx| {
                    this.browse_settings_working_dir(root_cx)
                });
            }),
        )
        .browse_focus(self.settings_page.working_dir_browse_focus.clone());
        let file_link_editor = self.settings(cx).file_link_editor;
        let found = file_link_editor.cli().map(|command| {
            self.app_store
                .read(cx)
                .file_link_environment
                .1
                .get(command)
                .copied()
                .unwrap_or(false)
        });
        let (file_link_hint, file_link_warns) = file_link_hint(file_link_editor, found);
        let file_link_row = SettingsRow::new(
            crate::i18n::text("ui.open_file_links_in"),
            self.settings_page.file_link_editor.clone(),
        )
        .subtitle(file_link_hint);
        let file_link_row = if file_link_warns {
            file_link_row.subtitle_color(theme::warning())
        } else {
            file_link_row
        };
        let toggle_root = cx.entity();
        let (command_status, command_requires_escalation, command_action_target, skill_status) = {
            let store = self.app_store.read(cx);
            (
                store.runner_command_status().cloned(),
                store.command_install_requires_escalation(),
                store.command_action_target().cloned(),
                store.runner_skill_status().clone(),
            )
        };
        let skill_enabled = self.settings(cx).runner_skill_enabled;
        let command_row = self.render_command_install_row(
            command_status.as_ref(),
            command_requires_escalation,
            command_action_target.as_deref(),
            cx,
        );
        let skill_toggle_root = cx.entity();
        let skill_row = command_line_settings_row(
            div()
                .text_size(theme::text_body())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text())
                .child(crate::i18n::text("ui.runner_skill_for_agents"))
                .into_any_element(),
            crate::i18n::text("ui.installs_a_skill_for_every_detected_agent_so_it_can_find_runner_and_drive_it_through_the_r"),
            runner_skill_status_line(skill_enabled, &skill_status)
                .into_iter()
                .map(|(text, warning)| {
                    div()
                        .text_size(theme::text_caption())
                        .text_color(if warning { theme::warning() } else { theme::faint() })
                        .child(text)
                        .into_any_element()
                })
                .collect(),
            Some(
                Toggle::new("settings-runner-skill", skill_enabled)
                    .on_change(move |enabled, _, cx| {
                        skill_toggle_root.update(cx, |this, root_cx| {
                            this.set_runner_skill_enabled(enabled, root_cx)
                        });
                    })
                    .into_any_element(),
            ),
        );

        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(crate::i18n::text("ui.general"), crate::i18n::text("ui.defaults_and_startup_behavior")))
            .child(SettingsCard::new(vec![
                SettingsRow::new(
                    i18n::t("language.title"),
                    div().when(cfg!(test), |select| select.debug_selector(|| "SETTINGS_LANGUAGE_SELECT".into()))
                        .child(self.settings_page.language.clone()),
                ).subtitle(i18n::t("language.description")).wrap_control().into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("ui.default_working_directory"),
                    div().w(rems(280. / 16.)).child(working_dir),
                )
                .subtitle(crate::i18n::text("ui.default_for_new_chats_leave_blank_to_use_your_home_directory"))
                .into_any_element(),
                file_link_row.into_any_element(),
                SettingsRow::new(crate::i18n::text("ui.app_zoom"), zoom)
                    .subtitle(
                        crate::i18n::text("ui.whole_app_scale_doesn_t_apply_to_the_terminal_canvas_see_terminal_pane"),
                    )
                    .into_any_element(),
            ]))
            .child(
                div().flex().flex_col().gap_2()
                    .child(settings_section_heading(crate::i18n::text("ui.sessions")))
                    .child(SettingsCard::new(vec![SettingsRow::new(
                        crate::i18n::text("ui.when_runner_quits"), self.settings_page.quit_behavior.clone(),
                    ).subtitle(crate::i18n::tnf("format.keep_sessions_running_in_the_background_or_stop_them_until_you_next_open_runne", self.app_store.read(cx).live_session_count, &[])).into_any_element()])),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(settings_section_heading(crate::i18n::text("ui.command_line")))
                    .child(SettingsCard::new(vec![command_row, skill_row])),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .px_1()
                            .text_size(theme::text_meta())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::faint())
                            .child(crate::i18n::text("heading.startup")),
                    )
                    .child(SettingsCard::new(vec![SettingsRow::new(
                        crate::i18n::text("ui.resume_running_agents_on_launch"),
                        Toggle::new(
                            "settings-resume-on-launch",
                            self.settings(cx).resume_on_launch,
                        )
                        .on_change(move |enabled, _, cx| {
                            toggle_root.update(cx, |this, root_cx| {
                                this.set_resume_on_launch(enabled, root_cx)
                            });
                        }),
                    )
                    .subtitle(
                        crate::i18n::text("ui.reopen_chats_and_mission_agents_that_were_live_when_runner_quit"),
                    )
                    .into_any_element()])),
            )
            .into_any_element()
    }

    fn render_command_install_row(
        &self,
        status: Option<&RunnerCommandStatus>,
        requires_escalation: bool,
        action_target: Option<&Path>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.settings_page.command_install_activity;
        let install_root = cx.entity();
        let uninstall_root = install_root.clone();
        let mut lines = Vec::new();
        let mut control = None;
        match status.map(|status| &status.state) {
            Some(RunnerCommandState::Installed | RunnerCommandState::Shadowed) => {
                if cfg!(windows) {
                    lines.push(command_status_text(
                        crate::i18n::text("ui.on_your_path_new_terminals_pick_it_up"),
                        theme::accent(),
                        Some("check.svg"),
                    ));
                } else if let Some(path) = status.and_then(|status| status.path.as_ref()) {
                    lines.push(command_installed_line(path));
                }
                if let Some(path) = status.and_then(|status| status.shadowed_by.as_ref()) {
                    let command_name =
                        runner_core::command_install::runner_command_name(cfg!(debug_assertions));
                    lines.push(command_status_text(
                        crate::i18n::tf(
                            "format.comes_first_on_your_path_so_typing_starts_that_one",
                            &[
                                ("path", (path.display()).to_string()),
                                ("command", (command_name).to_string()),
                            ],
                        ),
                        theme::warning(),
                        Some("triangle-alert.svg"),
                    ));
                }
                let label = if busy == Some(CommandInstallActivity::Uninstall) {
                    crate::i18n::text("ui.uninstalling")
                } else {
                    crate::i18n::text("ui.uninstall")
                };
                control = Some(
                    Button::new("settings-uninstall-runner-command", label)
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .disabled(busy.is_some())
                        .on_press(move |_, cx| {
                            uninstall_root
                                .update(cx, |this, root_cx| this.uninstall_runner_command(root_cx));
                        })
                        .into_any_element(),
                );
            }
            Some(RunnerCommandState::Foreign) => {
                if let Some(path) = status.and_then(|status| status.path.as_ref()) {
                    lines.push(command_status_text(
                        crate::i18n::tf(
                            "format.belongs_to_another_program_runner_will_not_replace_it",
                            &[("path", (path.display()).to_string())],
                        ),
                        theme::warning(),
                        Some("triangle-alert.svg"),
                    ));
                }
            }
            Some(RunnerCommandState::Unsupported) => {
                lines.push(command_status_text(
                    crate::i18n::text(
                        "ui.the_development_build_does_not_install_a_command_on_windows",
                    ),
                    theme::faint(),
                    None,
                ));
            }
            Some(RunnerCommandState::NotInstalled) | None => {
                let text = if requires_escalation && busy == Some(CommandInstallActivity::Install) {
                    crate::i18n::text("ui.waiting_for_your_password").to_owned()
                } else if requires_escalation {
                    let target = action_target
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| {
                            format!(
                                "/usr/local/bin/{}",
                                runner_core::command_install::runner_command_name(cfg!(
                                    debug_assertions
                                ))
                            )
                        });
                    if runner_core::protocol::command::force_escalated_command_install() {
                        crate::i18n::tf(
                            "format.not_installed_install_links_and_asks_for_your_password_once",
                            &[("target", (target).to_string())],
                        )
                    } else {
                        crate::i18n::tf("format.not_installed_local_bin_is_not_on_your_path_so_install_links_and_asks_for_your", &[("target", (target).to_string())])
                    }
                } else {
                    crate::i18n::text("ui.not_installed").to_owned()
                };
                lines.push(command_status_text(text, theme::faint(), None));
                let label = if busy == Some(CommandInstallActivity::Install) {
                    crate::i18n::text("ui.installing")
                } else if requires_escalation {
                    crate::i18n::text("ui.install")
                } else {
                    crate::i18n::text("ui.install_command_action")
                };
                control = Some(
                    Button::new("settings-install-runner-command", label)
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .disabled(busy.is_some())
                        .on_press(move |_, cx| {
                            install_root
                                .update(cx, |this, root_cx| this.install_runner_command(root_cx));
                        })
                        .into_any_element(),
                );
            }
        }
        if let Some((activity, path, reason)) = &self.settings_page.command_install_error {
            let verb = match activity {
                CommandInstallActivity::Install => crate::i18n::text("word.link"),
                CommandInstallActivity::Uninstall => crate::i18n::text("word.remove"),
            };
            lines.push(command_status_text(
                crate::i18n::tf(
                    "format.could_not",
                    &[
                        ("verb", (verb).to_string()),
                        ("path", (path.display()).to_string()),
                        ("reason", (reason.trim_end_matches('.')).to_string()),
                    ],
                ),
                theme::danger(),
                None,
            ));
        }
        command_line_settings_row(
            div()
                .flex()
                .items_baseline()
                .gap_1()
                .text_size(theme::text_body())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text())
                .child(div().font_family(theme::UI_MONOSPACE_FONT).child(
                    runner_core::command_install::runner_command_name(cfg!(debug_assertions)),
                ))
                .child(crate::i18n::text("word.command_suffix"))
                .into_any_element(),
            crate::i18n::text("ui.use_runner_from_any_terminal_or_script"),
            lines,
            control,
        )
    }

    fn render_missions_settings(&self, _cx: &mut Context<Self>) -> AnyElement {
        missions_settings_pane(self.settings_page.default_crew.clone())
    }

    fn render_appearance_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                crate::i18n::text("ui.appearance"),
                crate::i18n::text("ui.theme_and_the_app_and_terminal_palettes_for_each_mode"),
            ))
            .child(SettingsCard::new({
                let rows = vec![SettingsRow::new(crate::i18n::text("ui.theme"), self.render_theme_segmented(cx))
                    .subtitle(crate::i18n::text("ui.match_the_os_or_pin_to_light_or_dark"))
                    .into_any_element()];
                #[cfg(target_os = "macos")]
                let rows = {
                    let mut rows = rows;
                    rows.push(SettingsRow::new(crate::i18n::text("ui.window_material"), self.render_material_segmented(cx))
                        .subtitle(crate::i18n::text("ui.glass_adds_depth_to_the_sidebar_and_window_chrome"))
                        .into_any_element());
                    rows
                };
                rows
            }))
            .child(self.render_theme_preview(cx))
            .child(palette_section(
                crate::i18n::text("ui.light"),
                "SETTINGS_APPEARANCE_LIGHT",
                self.settings_page.light_theme.clone(),
                crate::i18n::text("ui.chrome_colours_when_the_app_is_light"),
                self.settings_page.light_terminal_theme.clone(),
                crate::i18n::text("ui.terminal_colours_when_the_app_is_light"),
            ))
            .child(palette_section(
                crate::i18n::text("ui.dark"),
                "SETTINGS_APPEARANCE_DARK",
                self.settings_page.dark_theme.clone(),
                crate::i18n::text("ui.chrome_colours_when_the_app_is_dark"),
                self.settings_page.dark_terminal_theme.clone(),
                crate::i18n::text("ui.terminal_colours_when_the_app_is_dark"),
            ))
            .when(cfg!(target_os = "macos"), |page| page.child(
                div().text_size(theme::text_ui()).text_color(theme::muted())
                    .child(crate::i18n::text("ui.runner_follows_macos_reduce_transparency_when_enabled_glass_uses_the_solid_appearance")),
            ))
            .into_any_element()
    }

    #[cfg(target_os = "macos")]
    fn render_material_segmented(&self, cx: &mut Context<Self>) -> AnyElement {
        use runner_app::appearance::WindowMaterial;
        let selected = self.settings(cx).window_material;
        div()
            .debug_selector(|| "SETTINGS_WINDOW_MATERIAL".into())
            .flex()
            .gap(rems(2. / 16.))
            .p(rems(2. / 16.))
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .children(
                [
                    (WindowMaterial::Glass, crate::i18n::text("ui.glass")),
                    (WindowMaterial::Solid, crate::i18n::text("ui.solid")),
                ]
                .into_iter()
                .map(|(material, label)| {
                    let root = cx.entity();
                    let key_root = root.clone();
                    div()
                        .id(SharedString::from(format!("settings-material-{label}")))
                        .tab_index(0)
                        .px(rems(12. / 16.))
                        .py(rems(5. / 16.))
                        .rounded(rems(4. / 16.))
                        .text_size(theme::text_ui())
                        .when(material == selected, |item| item.bg(theme::raised()))
                        .cursor_pointer()
                        .child(label)
                        .on_click(move |_, _, cx| {
                            root.update(cx, |this, cx| this.set_window_material(material, cx));
                        })
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_root
                                    .update(cx, |this, cx| this.set_window_material(material, cx));
                            }
                        })
                }),
            )
            .into_any_element()
    }

    #[cfg(target_os = "macos")]
    fn set_window_material(
        &self,
        material: runner_app::appearance::WindowMaterial,
        cx: &mut Context<Self>,
    ) {
        self.update_app_settings(cx, true, |settings| {
            if settings.window_material == material {
                return false;
            }
            settings.window_material = material;
            true
        });
        runner_app::appearance::configure(material, cx);
        cx.refresh_windows();
    }

    /// Both panes are built from the picks, not from the active variant: the
    /// left pane is always the light picks and the right the dark picks, and
    /// only the outline and caption follow which mode is resolved now.
    fn render_theme_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings(cx);
        let light_variant = theme::resolve_variant(
            ThemeIntent::Light,
            true,
            settings.light_app_theme,
            settings.dark_app_theme,
        );
        let dark_variant = theme::resolve_variant(
            ThemeIntent::Dark,
            false,
            settings.light_app_theme,
            settings.dark_app_theme,
        );
        let is_light = theme::active_variant().is_light();
        let pick_root = cx.entity();
        let on_pick: theme_preview::PickHandler = Rc::new(move |mode, _, cx| {
            pick_root.update(cx, |this, cx| this.set_theme_intent(mode.intent(), cx));
        });
        theme_preview::theme_preview(
            PreviewPane {
                mode: PreviewMode::Light,
                colors: theme::colors_for(light_variant),
                palette: settings.light_terminal_theme.palette(),
                font_family: settings.terminal_font_family.family(),
                caption: crate::i18n::tf(
                    "format.light",
                    &[
                        (
                            "app_theme",
                            (light_theme_label(settings.light_app_theme)).to_string(),
                        ),
                        (
                            "terminal_theme",
                            (settings.light_terminal_theme.label()).to_string(),
                        ),
                    ],
                )
                .into(),
                active: is_light,
            },
            PreviewPane {
                mode: PreviewMode::Dark,
                colors: theme::colors_for(dark_variant),
                palette: settings.dark_terminal_theme.palette(),
                font_family: settings.terminal_font_family.family(),
                caption: crate::i18n::tf(
                    "format.dark",
                    &[
                        (
                            "app_theme",
                            (dark_theme_label(settings.dark_app_theme)).to_string(),
                        ),
                        (
                            "terminal_theme",
                            (settings.dark_terminal_theme.label()).to_string(),
                        ),
                    ],
                )
                .into(),
                active: !is_light,
            },
            on_pick,
        )
    }

    fn render_theme_segmented(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(rems(2. / 16.))
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .p(rems(2. / 16.))
            .children(
                [
                    (
                        ThemeIntent::Auto,
                        crate::i18n::text("ui.auto"),
                        "monitor.svg",
                    ),
                    (ThemeIntent::Light, crate::i18n::text("ui.light"), "sun.svg"),
                    (ThemeIntent::Dark, crate::i18n::text("ui.dark"), "moon.svg"),
                ]
                .into_iter()
                .map(|(intent, label, icon)| {
                    let active = intent == self.settings(cx).app_theme;
                    let foreground = if active {
                        theme::text()
                    } else {
                        theme::muted()
                    };
                    let hover_group = SharedString::from(format!(
                        "settings-theme-{}",
                        label.to_ascii_lowercase()
                    ));
                    let root = cx.entity();
                    let key_root = root.clone();
                    div()
                        .id(SharedString::from(format!("settings-theme-{label}")))
                        .group(hover_group.clone())
                        .tab_index(0)
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .rounded(rems(4. / 16.))
                        .px(rems(10. / 16.))
                        .py(rems(5. / 16.))
                        .bg(if active {
                            theme::raised()
                        } else {
                            gpui::transparent_black()
                        })
                        .text_size(theme::text_ui())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(foreground)
                        .cursor(CursorStyle::PointingHand)
                        .hover(|button| button.text_color(theme::text()))
                        .child(
                            svg()
                                .path(icon)
                                .size(rems(12. / 16.))
                                .flex_none()
                                .text_color(foreground)
                                .group_hover(hover_group, |icon| icon.text_color(theme::text())),
                        )
                        .child(label)
                        .on_click(move |_, _, cx| {
                            root.update(cx, |this, root_cx| this.set_theme_intent(intent, root_cx));
                        })
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_root.update(cx, |this, root_cx| {
                                    this.set_theme_intent(intent, root_cx)
                                });
                            }
                        })
                }),
            )
            .into_any_element()
    }

    fn render_terminal_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let size = self
            .settings(cx)
            .terminal_font_size
            .clamp(TERMINAL_FONT_SIZE_MIN, TERMINAL_FONT_SIZE_MAX);
        let decrement_root = cx.entity();
        let increment_root = decrement_root.clone();
        let decrement: StepHandler = Rc::new(move |_, cx| {
            decrement_root.update(cx, |this, root_cx| {
                this.set_terminal_font_size(size.saturating_sub(1), root_cx)
            });
        });
        let increment: StepHandler = Rc::new(move |_, cx| {
            increment_root.update(cx, |this, root_cx| {
                this.set_terminal_font_size(size.saturating_add(1), root_cx)
            });
        });
        let size_stepper = Stepper::new(
            "settings-terminal-font-size",
            64.,
            div()
                .flex()
                .items_center()
                .gap(rems(3. / 16.))
                .font_family(theme::UI_MONOSPACE_FONT)
                .child(
                    div()
                        .text_size(theme::text_ui())
                        .font_weight(FontWeight::MEDIUM)
                        .child(size.to_string()),
                )
                .child(
                    div()
                        .text_size(theme::text_caption())
                        .text_color(theme::faint())
                        .child("px"),
                ),
            decrement,
            increment,
        )
        .decrement_disabled(size == TERMINAL_FONT_SIZE_MIN)
        .increment_disabled(size == TERMINAL_FONT_SIZE_MAX);

        div()
            .flex()
            .flex_col()
            .gap_5()
            .debug_selector(|| "SETTINGS_TERMINAL_PANE".into())
            .child(PaneHeader::new(
                crate::i18n::text("ui.terminal"),
                crate::i18n::text(
                    "ui.font_size_and_cursor_for_the_embedded_terminal_palettes_live_in_appearance",
                ),
            ))
            .child(SettingsCard::new(vec![
                SettingsRow::new(
                    crate::i18n::text("ui.font_family"),
                    div()
                        .debug_selector(|| "SETTINGS_TERMINAL_FONT".into())
                        .child(self.settings_page.terminal_font.clone()),
                )
                .subtitle(crate::i18n::text(
                    "ui.typeface_used_by_the_embedded_terminal",
                ))
                .into_any_element(),
                SettingsRow::new(crate::i18n::text("ui.terminal_font_size"), size_stepper)
                    .subtitle(crate::i18n::text("ui.glyph_size_for_the_embedded_terminal"))
                    .into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("ui.cursor_style"),
                    self.settings_page.terminal_cursor.clone(),
                )
                .subtitle(crate::i18n::text(
                    "ui.block_underline_or_bar_affects_the_prompt_caret_only",
                ))
                .into_any_element(),
            ]))
            .into_any_element()
    }
}

fn shortcut_row_label(
    title: &'static str,
    description: &'static str,
    conflict: Option<String>,
) -> gpui::Div {
    div()
        .min_w(px(0.))
        .flex_1()
        .flex()
        .flex_col()
        .gap(rems(2. / 16.))
        .child(
            div()
                .text_size(theme::text_body())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text())
                .child(title),
        )
        .child(
            div()
                .text_size(theme::text_meta())
                .text_color(theme::muted())
                .child(description),
        )
        .children(conflict.map(|message| {
            div()
                .text_size(theme::text_meta())
                .text_color(theme::danger())
                .child(message)
        }))
}

fn shortcut_chip(label: String, editable: bool) -> gpui::Div {
    div()
        .px_2()
        .py_1()
        .rounded(rems(4. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::raised())
        .text_size(theme::text_meta())
        .line_height(rems(14. / 16.))
        .text_color(theme::muted())
        .when(editable, |chip| {
            chip.cursor_pointer().hover(|chip| {
                chip.border_color(theme::border_strong())
                    .text_color(theme::text())
            })
        })
        .child(label)
}

/// The row subtitle for the current editor choice, and whether it warns.
fn file_link_hint(editor: FileLinkEditor, cli_found: Option<bool>) -> (String, bool) {
    match (editor, cli_found) {
        (FileLinkEditor::DefaultApp, _) => (
            crate::i18n::tf(
                "format.click_opens_the_file_in_its_default_app_the_cited_line_is_lost",
                &[(
                    "shortcut",
                    (crate::platform_ui::PRIMARY_MODIFIER).to_string(),
                )],
            ),
            false,
        ),
        (editor, Some(false)) => (
            crate::i18n::tf(
                "format.not_found_on_path",
                &[
                    ("command", (editor.cli().unwrap_or_default()).to_string()),
                    (
                        "install_hint",
                        (editor.install_hint().unwrap_or_default()).to_string(),
                    ),
                ],
            ),
            true,
        ),
        (editor, _) => (
            crate::i18n::tf(
                "format.click_opens_the_file_at_the_cited_line_via",
                &[
                    (
                        "shortcut",
                        (crate::platform_ui::PRIMARY_MODIFIER).to_string(),
                    ),
                    ("editor", (editor.cli().unwrap_or_default()).to_string()),
                ],
            ),
            false,
        ),
    }
}

fn update_if_changed<T: PartialEq>(target: &mut T, value: T) -> bool {
    if *target == value {
        return false;
    }
    *target = value;
    true
}

fn light_theme_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("runner-light", "Runner Light").swatch(0x00a66a),
        SelectOption::new("catppuccin-latte", "Catppuccin Latte").swatch(0x8839ef),
    ]
}

fn light_terminal_theme_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("rose-pine-dawn", "Rosé Pine Dawn").swatch(0xd7827e),
        SelectOption::new("runner", "Runner").swatch(0x00ff9c),
        SelectOption::new("catppuccin-mocha", "Catppuccin Mocha").swatch(0xcba6f7),
    ]
}

fn dark_terminal_theme_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("runner", "Runner").swatch(0x00ff9c),
        SelectOption::new("catppuccin-mocha", "Catppuccin Mocha").swatch(0xcba6f7),
        SelectOption::new("rose-pine-dawn", "Rosé Pine Dawn").swatch(0xd7827e),
    ]
}

/// A mode's label and its App palette + Terminal palette card.
fn palette_section(
    label: &'static str,
    selector: &'static str,
    app_palette: Entity<StyledSelect>,
    app_subtitle: &'static str,
    terminal_palette: Entity<StyledSelect>,
    terminal_subtitle: &'static str,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .debug_selector(move || selector.into())
        .child(
            div()
                .text_size(theme::text_body())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::muted())
                .child(label),
        )
        .child(SettingsCard::new(vec![
            SettingsRow::new(crate::i18n::text("ui.app_palette"), app_palette)
                .subtitle(app_subtitle)
                .into_any_element(),
            SettingsRow::new(crate::i18n::text("ui.terminal_palette"), terminal_palette)
                .subtitle(terminal_subtitle)
                .into_any_element(),
        ]))
        .into_any_element()
}

fn light_theme_label(value: LightTheme) -> &'static str {
    match value {
        LightTheme::RunnerLight => "Runner Light",
        LightTheme::CatppuccinLatte => "Catppuccin Latte",
    }
}

fn dark_theme_label(value: DarkTheme) -> &'static str {
    match value {
        DarkTheme::Runner => "Runner",
        DarkTheme::CatppuccinMocha => "Catppuccin Mocha",
    }
}

fn light_theme_value(value: LightTheme) -> &'static str {
    match value {
        LightTheme::RunnerLight => "runner-light",
        LightTheme::CatppuccinLatte => "catppuccin-latte",
    }
}

fn parse_light_theme(value: &str) -> Option<LightTheme> {
    match value {
        "runner-light" => Some(LightTheme::RunnerLight),
        "catppuccin-latte" => Some(LightTheme::CatppuccinLatte),
        _ => None,
    }
}

fn dark_theme_value(value: DarkTheme) -> &'static str {
    match value {
        DarkTheme::Runner => "carbon",
        DarkTheme::CatppuccinMocha => "catppuccin-mocha",
    }
}

fn parse_dark_theme(value: &str) -> Option<DarkTheme> {
    match value {
        "carbon" => Some(DarkTheme::Runner),
        "catppuccin-mocha" => Some(DarkTheme::CatppuccinMocha),
        _ => None,
    }
}

fn terminal_font_value(value: TerminalFontFamily) -> &'static str {
    match value {
        TerminalFontFamily::JetBrainsMono => "JetBrains Mono",
        TerminalFontFamily::Menlo => "Menlo",
    }
}

fn parse_terminal_font(value: &str) -> Option<TerminalFontFamily> {
    match value {
        "Menlo" => Some(TerminalFontFamily::Menlo),
        "JetBrains Mono" => Some(TerminalFontFamily::JetBrainsMono),
        _ => None,
    }
}

fn terminal_cursor_value(value: TerminalCursorStyle) -> &'static str {
    match value {
        TerminalCursorStyle::Block => "block",
        TerminalCursorStyle::Underline => "underline",
        TerminalCursorStyle::Bar => "bar",
    }
}

fn parse_terminal_cursor(value: &str) -> Option<TerminalCursorStyle> {
    match value {
        "block" => Some(TerminalCursorStyle::Block),
        "underline" => Some(TerminalCursorStyle::Underline),
        "bar" => Some(TerminalCursorStyle::Bar),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(windows))]
    fn shortcut_rows_keep_registry_order_with_one_tab_row_and_no_copy() {
        let overrides = keymap::KeymapOverrides::new();
        let rows = shortcut_rows("", &overrides);
        let titles = rows.iter().map(|row| row.title()).collect::<Vec<_>>();
        assert_eq!(titles.len(), 24);
        assert_eq!(titles[0], "New window");
        assert!(!titles.contains(&"Copy"));
        let after = |title: &str| titles[titles.iter().position(|t| *t == title).unwrap() + 1];
        assert_eq!(after("Reset zoom"), "Go to tab 1–9");
        assert_eq!(after("Split pane down"), "Close tab");
        assert_eq!(
            rows.iter()
                .filter(|row| row.fixed())
                .map(|row| row.title())
                .collect::<Vec<_>>(),
            ["New window", "Go to tab 1–9", "Close tab", "Close window"]
        );
        assert_eq!(ShortcutRow::TabSelection.binding_label(&overrides), "⌘1–⌘9");
    }

    #[test]
    #[cfg(not(windows))]
    fn fixed_rows_turn_off_and_restore_without_taking_a_custom_key() {
        let tabs = ShortcutRow::TabSelection;
        let close = ShortcutRow::Entry(keymap::entry("close-pane").unwrap());
        let mut overrides = keymap::KeymapOverrides::new();
        overrides.insert("close-pane".into(), None);
        for id in tabs.entry_ids() {
            overrides.insert(id.into(), None);
        }
        assert_eq!(close.binding_label(&overrides), "Unassigned");
        assert_eq!(tabs.binding_label(&overrides), "Unassigned");
        assert!(close.overridden(&overrides) && tabs.overridden(&overrides));

        let mut stale = keymap::KeymapOverrides::new();
        stale.insert(
            "close-pane".into(),
            Some(keymap::entry("new-chat").unwrap().default.clone()),
        );
        stale.insert("select-tab-3".into(), None);
        assert_eq!(close.binding_label(&stale), "⌘W");
        assert_eq!(tabs.binding_label(&stale), "⌘1–⌘9");
        assert!(!close.overridden(&stale) && !tabs.overridden(&stale));
    }

    #[test]
    #[cfg(not(windows))]
    fn shortcut_search_matches_grouped_tab_terms() {
        let overrides = keymap::KeymapOverrides::new();
        for query in ["tab 3", "⌘3", "go to tab"] {
            assert_eq!(
                shortcut_rows(query, &overrides),
                [ShortcutRow::TabSelection],
                "{query}"
            );
        }
        assert_eq!(
            shortcut_rows("resume focused", &overrides)
                .iter()
                .map(|row| row.title())
                .collect::<Vec<_>>(),
            ["Resume focused session"]
        );
        assert!(shortcut_rows("definitely missing", &overrides).is_empty());
    }

    #[test]
    fn runner_skill_row_only_reports_exception_states() {
        let healthy = crate::app_store::RunnerSkillStatus {
            detected: true,
            foreign: None,
        };
        assert_eq!(runner_skill_status_line(true, &healthy), None);
        assert_eq!(runner_skill_status_line(false, &healthy), None);

        let missing_agent = crate::app_store::RunnerSkillStatus::default();
        assert_eq!(
            runner_skill_status_line(true, &missing_agent),
            Some((
                "No supported agent detected yet. The skill is added when one is installed.".into(),
                false,
            ))
        );
        assert_eq!(runner_skill_status_line(false, &missing_agent), None);

        let foreign = crate::app_store::RunnerSkillStatus {
            detected: false,
            foreign: Some(PathBuf::from("/tmp/home/.agents/skills/runner-dev")),
        };
        assert_eq!(
            runner_skill_status_line(false, &foreign),
            Some((
                "/tmp/home/.agents/skills/runner-dev was not installed by Runner and is left alone."
                    .into(),
                true,
            ))
        );
    }

    #[test]
    fn theme_selects_have_the_signed_off_options_in_order() {
        let light = light_theme_options();
        assert_eq!(
            light,
            vec![
                SelectOption::new("runner-light", "Runner Light").swatch(0x00a66a),
                SelectOption::new("catppuccin-latte", "Catppuccin Latte").swatch(0x8839ef),
            ]
        );
        let light_terminal = light_terminal_theme_options();
        assert_eq!(
            light_terminal,
            vec![
                SelectOption::new("rose-pine-dawn", "Rosé Pine Dawn").swatch(0xd7827e),
                SelectOption::new("runner", "Runner").swatch(0x00ff9c),
                SelectOption::new("catppuccin-mocha", "Catppuccin Mocha").swatch(0xcba6f7),
            ]
        );
        let dark_terminal = dark_terminal_theme_options();
        assert_eq!(
            dark_terminal,
            vec![
                SelectOption::new("runner", "Runner").swatch(0x00ff9c),
                SelectOption::new("catppuccin-mocha", "Catppuccin Mocha").swatch(0xcba6f7),
                SelectOption::new("rose-pine-dawn", "Rosé Pine Dawn").swatch(0xd7827e),
            ]
        );
        for option in light {
            let theme = parse_light_theme(&option.value).unwrap();
            assert_eq!(light_theme_value(theme), option.value);
            assert_eq!(option.label.as_ref(), light_theme_label(theme));
        }
        for (option, theme) in light_terminal.iter().zip(LightTerminalTheme::ALL) {
            assert_eq!(option.value, theme.key());
            assert_eq!(option.label.as_ref(), theme.label());
        }
        for (option, theme) in dark_terminal.iter().zip(DarkTerminalTheme::ALL) {
            assert_eq!(option.value, theme.key());
            assert_eq!(option.label.as_ref(), theme.label());
        }
        assert_eq!(dark_theme_label(DarkTheme::Runner), "Runner");
        assert_eq!(parse_light_theme("codex"), None);
        assert_eq!(LightTerminalTheme::parse("match-app"), None);
        assert_eq!(DarkTerminalTheme::parse("runner-dark"), None);
    }

    #[test]
    fn usage_stays_solid_and_settings_gear_opens_agents_in_both_materials() {
        use crate::theme_snapshot::{assert_fill, ThemeGuard};
        use gpui::{size, Modifiers, TestAppContext, VisualTestContext};
        use runner_app::appearance::{Appearance, WindowMaterial};
        use runner_core::protocol::model::Runtime;

        let _theme = ThemeGuard::new();
        for material in [WindowMaterial::Solid, WindowMaterial::Glass] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            cx.update(|cx| {
                cx.set_global(Appearance {
                    material,
                    reduce_transparency: false,
                })
            });
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            store.update(&mut cx, |store, cx| {
                store.settings.window_material = material;
                cx.notify();
            });
            let host = cx.add_window(|window, cx| {
                let mut root = NativeRoot::new(
                    "usage-settings-gear".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                root.usage_installed = vec![Runtime::Codex];
                root
            });
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            visual.simulate_resize(size(px(1200.), px(900.)));
            visual.run_until_parked();
            let usage = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
            visual.simulate_click(usage.center(), Modifiers::default());
            visual.run_until_parked();
            assert_fill(&mut visual, "USAGE_POPOVER_PANEL", theme::colors().panel);
            visual.update(|_, cx| assert_eq!(cx.windows().len(), 1));
            let gear = visual.debug_bounds("USAGE_AGENT_SETTINGS").unwrap();
            visual.simulate_click(gear.center(), Modifiers::default());
            visual.run_until_parked();
            host.read_with(&visual, |root, _| {
                assert!(!root.usage_open);
                assert_eq!(root.route, AppRoute::Settings);
                assert_eq!(root.settings_page.pane, SettingsPane::Agents);
            })
            .unwrap();
            assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_none());
        }
    }

    #[test]
    fn agent_page_keyboard_tabs_from_executable_to_browse_and_back() {
        use gpui::{
            point, size, KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext,
        };

        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let host = cx.add_window(|window, cx| {
            NativeRoot::new(
                "agent-settings-keyboard".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1200.), px(900.)));
        host.update(&mut visual, |root, window, cx| {
            root.enter_settings_pane(SettingsPane::Agents, window, cx);
        })
        .unwrap();
        visual.run_until_parked();
        let row = visual.debug_bounds("AGENT_CARD_codex").unwrap();
        visual.simulate_click(
            point(row.left() + px(24.), row.center().y),
            Modifiers::default(),
        );
        visual.run_until_parked();
        let executable = visual.debug_bounds("AGENT_EXECUTABLE_codex").unwrap();
        visual.simulate_click(
            point(executable.left() + px(24.), executable.center().y),
            Modifiers::default(),
        );
        visual.run_until_parked();
        let editor = host
            .update(&mut visual, |_, window, cx| window.focused(cx).unwrap())
            .unwrap();
        visual.simulate_keystrokes("tab");
        visual.run_until_parked();
        host.update(&mut visual, |_, window, _| {
            assert!(!editor.is_focused(window))
        })
        .unwrap();
        visual.simulate_keystrokes("space");
        visual.run_until_parked();
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|_| None);
        visual.run_until_parked();
        visual.simulate_keystrokes("shift-tab");
        visual.run_until_parked();
        host.update(&mut visual, |_, window, _| {
            assert!(editor.is_focused(window))
        })
        .unwrap();
        let back = visual.debug_bounds("AGENTS_BREADCRUMB").unwrap();
        visual.simulate_click(back.center(), Modifiers::default());
        visual.run_until_parked();
        host.update(&mut visual, |root, window, cx| {
            root.settings_page
                .search
                .read(cx)
                .focus_handle()
                .focus(window, cx);
        })
        .unwrap();
        visual.simulate_keystrokes("shift-tab");
        visual.run_until_parked();
        if visual.debug_bounds("AGENT_TOGGLE_opencode").is_some() {
            visual.simulate_keystrokes("space");
            visual.simulate_event(KeyUpEvent {
                keystroke: Keystroke::parse("space").unwrap(),
            });
            visual.run_until_parked();
            store.read_with(&visual, |store, _| {
                assert!(store.settings.disabled_agents.contains("opencode"))
            });
            assert!(visual.debug_bounds("AGENT_CARD_opencode").is_some());
            visual.simulate_keystrokes("shift-tab");
        }
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        assert!(visual.debug_bounds("AGENT_PAGE_opencode").is_some());
    }

    #[test]
    fn general_quit_menu_renders_a_right_hand_tick_for_each_choice() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{Modifiers, TestAppContext, VisualTestContext};
        use runner_app::lifecycle::QuitBehavior;
        struct QuitSelectHost(Entity<NativeRoot>);
        impl Render for QuitSelectHost {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .p_6()
                    .child(self.0.read(cx).settings_page.quit_behavior.clone())
            }
        }
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let host = cx.add_window(|window, cx| {
            QuitSelectHost(cx.new(|cx| {
                NativeRoot::new(
                    "quit-select".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                )
            }))
        });
        cx.run_until_parked();
        let select = host
            .read_with(&cx, |host, cx| {
                host.0.read(cx).settings_page.quit_behavior.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        for (index, choice) in QuitBehavior::ALL.into_iter().enumerate() {
            select.update(&mut cx, |select, cx| select.set_value(choice.key(), cx));
            cx.run_until_parked();
            let trigger = visual.debug_bounds("STYLED_SELECT_TRIGGER").unwrap();
            visual.simulate_click(trigger.center(), Modifiers::default());
            cx.run_until_parked();
            let row = visual
                .debug_bounds(Box::leak(
                    format!("STYLED_SELECT_OPTION_{index}").into_boxed_str(),
                ))
                .unwrap();
            let check = visual.debug_bounds("STYLED_SELECT_CHECK").unwrap();
            assert!(check.center().x > row.center().x);
            assert!(check.left() >= row.left() && check.right() <= row.right());
            assert!((check.center().y - row.center().y).abs() <= px(1.));
            visual.simulate_click(row.center(), Modifiers::default());
            cx.run_until_parked();
            assert_eq!(
                store.read_with(&cx, |store, _| store.settings.quit_behavior),
                choice
            );
        }
    }

    #[test]
    fn appearance_preview_draws_each_mode_from_its_picks_and_outlines_the_resolved_one() {
        use crate::theme_snapshot::{assert_fill, ThemeGuard};
        use gpui::{size, Render, TestAppContext, VisualTestContext};
        use runner_daemon::{db, session, shell_path};
        use std::sync::{Arc, RwLock};

        struct PaneHost(Entity<NativeRoot>);
        impl Render for PaneHost {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .p_6()
                    .children(self.0.update(cx, |root, cx| root.render_settings_pane(cx)))
            }
        }
        fn packed(rgb: alacritty_terminal::vte::ansi::Rgb) -> u32 {
            u32::from(rgb.r) << 16 | u32::from(rgb.g) << 8 | u32::from(rgb.b)
        }
        fn captioned_by(
            pane: gpui::Bounds<gpui::Pixels>,
            caption: gpui::Bounds<gpui::Pixels>,
        ) -> bool {
            caption.left() >= pane.left()
                && caption.right() <= pane.right()
                && caption.top() >= pane.bottom()
        }

        let _theme = ThemeGuard::new();
        theme::set_active_variant(theme::ThemeVariant::Carbon);
        let temp = tempfile::tempdir().unwrap();
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = crate::test_support::core(
            Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap()),
            temp.path().to_owned(),
            session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
        );
        let mut cx = TestAppContext::single();
        let store = cx.new(|cx| {
            AppStore::new(
                core,
                None,
                None,
                temp.path().join("settings.json"),
                AppSettings {
                    app_theme: ThemeIntent::Dark,
                    dark_terminal_theme: DarkTerminalTheme::CatppuccinMocha,
                    ..AppSettings::default()
                },
                None,
                cx,
            )
        });
        cx.update(|cx| {
            cx.set_global(GlobalAppStore(store.clone()));
            cx.set_global(WindowLayoutCheckpoint::default());
            #[cfg(not(windows))]
            let updater = cx.new(|cx| Updater::new(false, cx));
            #[cfg(windows)]
            let updater = cx.new(|cx| Updater::new(false, temp.path().join("updates"), cx));
            cx.set_global(GlobalUpdater(updater));
        });
        let host = cx.add_window(|window, cx| {
            window.resize(size(px(1200.), px(900.)));
            let root = cx.new(|cx| {
                NativeRoot::new(
                    "theme-preview".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                )
            });
            root.update(cx, |root, _| {
                root.settings_page.pane = SettingsPane::Appearance
            });
            PaneHost(root)
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(host.into(), &cx);
        let light_pane = PreviewMode::Light.pane_selector();
        let dark_pane = PreviewMode::Dark.pane_selector();

        // The app is Carbon, yet the left pane is Runner Light + Dawn: the
        // preview reads the picks, not the globals.
        assert_eq!(theme::active_variant(), theme::ThemeVariant::Carbon);
        let light = window.debug_bounds(light_pane).unwrap();
        let dark = window.debug_bounds(dark_pane).unwrap();
        assert!(light.right() <= dark.left(), "{light:?} {dark:?}");
        assert_fill(&mut window, light_pane, theme::RUNNER_LIGHT.bg);
        assert_fill(
            &mut window,
            PreviewMode::Light.terminal_selector(),
            packed(runner_terminal::palette::ROSE_PINE_DAWN.background),
        );
        assert_fill(&mut window, dark_pane, theme::CARBON.bg);
        assert_fill(
            &mut window,
            PreviewMode::Dark.terminal_selector(),
            packed(runner_terminal::palette::CATPPUCCIN_MOCHA.background),
        );
        let active = window.debug_bounds(theme_preview::ACTIVE_CAPTION).unwrap();
        assert!(captioned_by(dark, active), "{dark:?} {active:?}");

        theme::set_active_variant(theme::ThemeVariant::RunnerLight);
        window.refresh().unwrap();
        window.run_until_parked();
        let light = window.debug_bounds(light_pane).unwrap();
        let active = window.debug_bounds(theme_preview::ACTIVE_CAPTION).unwrap();
        assert!(captioned_by(light, active), "{light:?} {active:?}");
        assert_fill(&mut window, light_pane, theme::RUNNER_LIGHT.bg);
        assert_fill(&mut window, dark_pane, theme::CARBON.bg);
        // The outline is a selection, not only a report: clicking a pane pins
        // the Theme intent to that mode.
        let pick = |window: &mut VisualTestContext, selector: &'static str| {
            let at = window.debug_bounds(selector).unwrap().center();
            window.simulate_click(at, gpui::Modifiers::default());
            window.run_until_parked();
            host.update(window, |host, _, cx| host.0.read(cx).settings(cx).app_theme)
                .unwrap()
        };
        assert_eq!(pick(&mut window, dark_pane), ThemeIntent::Dark);
        assert_eq!(pick(&mut window, light_pane), ThemeIntent::Light);
        for selector in ["SETTINGS_APPEARANCE_LIGHT", "SETTINGS_APPEARANCE_DARK"] {
            assert!(window.debug_bounds(selector).is_some(), "{selector}");
        }
        assert_eq!(
            window.debug_bounds("SETTINGS_WINDOW_MATERIAL").is_some(),
            cfg!(target_os = "macos")
        );

        host.update(&mut window, |host, _, cx| {
            host.0.update(cx, |root, _| {
                root.settings_page.pane = SettingsPane::Terminal
            });
            cx.notify();
        })
        .unwrap();
        window.run_until_parked();
        assert!(window.debug_bounds("SETTINGS_TERMINAL_THEME").is_none());
        // GPUI clears `debug_bounds` each frame; Appearance selectors must be gone.
        for selector in ["SETTINGS_APPEARANCE_LIGHT", "SETTINGS_APPEARANCE_DARK"] {
            assert!(window.debug_bounds(selector).is_none(), "{selector}");
        }
        let pane = window.debug_bounds("SETTINGS_TERMINAL_PANE").unwrap();
        let font = window.debug_bounds("SETTINGS_TERMINAL_FONT").unwrap();
        assert!(
            font.top() >= pane.top() && font.bottom() <= pane.bottom(),
            "{pane:?} {font:?}"
        );
    }

    #[test]
    fn settings_routes_fall_back_to_general() {
        assert_eq!(SettingsPane::from_route(None), SettingsPane::General);
        assert_eq!(
            SettingsPane::from_route(Some("account")),
            SettingsPane::Account
        );
        assert_eq!(SettingsPane::Account.key(), "account");
        assert_eq!(SettingsPane::Account.icon(), "user-round.svg");
        assert_eq!(
            SettingsPane::from_route(Some("appearance")),
            SettingsPane::Appearance
        );
        assert_eq!(
            SettingsPane::from_route(Some("not-a-pane")),
            SettingsPane::General
        );
        assert_eq!(
            SettingsPane::from_route(Some("missions")),
            SettingsPane::Missions
        );
        assert_eq!(
            SettingsPane::from_route(Some("notifications")),
            SettingsPane::Notifications
        );
        assert_eq!(SettingsPane::Notifications.key(), "notifications");
        assert_eq!(SettingsPane::Notifications.icon(), "bell.svg");
        assert_eq!(SettingsPane::Missions.key(), "missions");
        assert_eq!(SettingsPane::Missions.label(), "Missions");
        assert_eq!(SettingsPane::Missions.icon(), "rocket.svg");
    }

    #[test]
    fn nav_search_filters_labels_and_removes_empty_groups() {
        let all = filtered_nav_groups("");
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().map(|(_, panes)| panes.len()).sum::<usize>(), 15);
        assert_eq!(
            all[0],
            (
                "App",
                vec![
                    SettingsPane::Account,
                    SettingsPane::General,
                    SettingsPane::Appearance,
                    SettingsPane::Notifications,
                    SettingsPane::Remote,
                    SettingsPane::Missions,
                    SettingsPane::Terminal,
                    SettingsPane::Shortcuts,
                    SettingsPane::Archived,
                ],
            )
        );

        assert_eq!(
            all[1],
            (
                "Integrations",
                vec![
                    SettingsPane::Agents,
                    SettingsPane::Skills,
                    SettingsPane::Mcp
                ]
            )
        );
        assert_eq!(
            SettingsPane::from_route(Some("skills")),
            SettingsPane::Skills
        );
        assert_eq!(SettingsPane::from_route(Some("mcp")), SettingsPane::Mcp);
        assert_eq!(
            filtered_nav_groups("mcp"),
            vec![("Integrations", vec![SettingsPane::Mcp])]
        );
        assert_eq!(
            filtered_nav_groups("skills"),
            vec![("Integrations", vec![SettingsPane::Skills])]
        );

        let terminal = filtered_nav_groups("  TERM  ");
        assert_eq!(terminal, vec![("App", vec![SettingsPane::Terminal])]);

        let archived = filtered_nav_groups("archived");
        assert_eq!(archived, vec![("App", vec![SettingsPane::Archived])]);
        assert!(filtered_nav_groups("no such setting").is_empty());
    }

    #[test]
    fn missions_pane_renders_default_crew_at_two_rem_sizes() {
        use gpui::{size, Render, TestAppContext, VisualTestContext};

        struct PaneHost {
            default_crew: Entity<StyledSelect>,
        }
        impl Render for PaneHost {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .p_6()
                    .child(missions_settings_pane(self.default_crew.clone()))
            }
        }

        let mut cx = TestAppContext::single();
        let host = cx.add_window(|window, cx| {
            window.resize(size(px(1200.), px(900.)));
            let noop: SelectHandler = Rc::new(|_, _, _| {});
            let default_crew = cx.new(|cx| {
                StyledSelect::new(
                    "settings-default-crew",
                    cx.focus_handle(),
                    "",
                    vec![SelectOption::new("", "No default")],
                    noop.clone(),
                    cx,
                )
            });
            PaneHost { default_crew }
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(host.into(), &cx);
        for rem in [16., 20.8] {
            host.update(&mut window, |_, window, _| window.set_rem_size(px(rem)))
                .unwrap();
            window.run_until_parked();
            let pane = window.debug_bounds("SETTINGS_MISSIONS_PANE").unwrap();
            let heading = window
                .debug_bounds("SETTINGS_MISSIONS_DEFAULTS_HEADING")
                .unwrap();
            let card = window.debug_bounds("SETTINGS_MISSIONS_CARD").unwrap();
            let crew = window.debug_bounds("SETTINGS_DEFAULT_CREW").unwrap();
            assert!(window
                .debug_bounds("SETTINGS_MISSION_PERMISSIONS")
                .is_none());
            assert!(
                heading.size.height >= px(rem) && heading.bottom() <= card.top(),
                "{rem}: {heading:?} {card:?}"
            );
            assert!(
                card.size.width > px(0.) && card.size.width <= pane.size.width,
                "{rem}: {card:?} {pane:?}"
            );
            assert!(
                crew.bottom() <= card.bottom() && crew.right() <= card.right(),
                "{rem}: {crew:?} {card:?}"
            );
        }
    }

    #[test]
    fn content_scrollbar_starts_below_the_drag_strip_at_the_window_edge() {
        use gpui::{
            point, size, Modifiers, Pixels, Point, Render, TestAppContext, VisualTestContext,
        };

        use crate::app_settings::SIDEBAR_DEFAULT;

        const HEIGHT: f32 = 320.;
        // ScrollbarKind::App's track width, private to ui/scrollbar.rs.
        const GUTTER: f32 = 10.;

        struct ColumnHost {
            default_crew: Entity<StyledSelect>,
            content_scroll: ScrollHandle,
            content_scrollbar: Entity<Scrollbar>,
        }
        impl Render for ColumnHost {
            fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let zoom = f32::from(window.rem_size()) / 16.;
                div()
                    .size_full()
                    .flex()
                    .child(div().flex_none().h_full().w(px(SIDEBAR_DEFAULT * zoom)))
                    .child(settings_content_column(
                        (SETTINGS_DRAG_INSET > 0.).then(|| {
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .h(px(TITLEBAR_DRAG_HEIGHT * zoom))
                        }),
                        Some(
                            div()
                                .flex()
                                .flex_col()
                                .child(missions_settings_pane(self.default_crew.clone()))
                                .child(div().h(px(400. * zoom)))
                                .into_any_element(),
                        ),
                        &self.content_scroll,
                        self.content_scrollbar.clone(),
                        zoom,
                    ))
            }
        }

        let mut cx = TestAppContext::single();
        let host = cx.add_window(|_, cx| {
            let noop: SelectHandler = Rc::new(|_, _, _| {});
            let default_crew = cx.new(|cx| {
                StyledSelect::new(
                    "settings-default-crew",
                    cx.focus_handle(),
                    "",
                    vec![SelectOption::new("", "No default")],
                    noop.clone(),
                    cx,
                )
            });
            let content_scroll = ScrollHandle::new();
            let owner = cx.entity_id();
            let content_scrollbar = cx.new(|_| Scrollbar::app(content_scroll.clone(), owner));
            ColumnHost {
                default_crew,
                content_scroll,
                content_scrollbar,
            }
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(host.into(), &cx);
        let scroll = host
            .update(&mut window, |host, _, _| host.content_scroll.clone())
            .unwrap();
        let near = |a: Pixels, b: Pixels| (f32::from(a) - f32::from(b)).abs() <= 0.5;

        for width in [1000., 2000.] {
            for rem in [16., 20.8] {
                let zoom = rem / 16.;
                let track_top = px(SETTINGS_DRAG_INSET * zoom);
                let content_top = px(SETTINGS_CONTENT_TOP * zoom);
                window.simulate_resize(size(px(width), px(HEIGHT)));
                scroll.set_offset(point(px(0.), px(0.)));
                host.update(&mut window, |_, window, _| {
                    window.set_rem_size(px(rem));
                    window.refresh();
                })
                .unwrap();
                window.run_until_parked();

                let track = window
                    .debug_bounds("SETTINGS_CONTENT_SCROLLBAR_TRACK")
                    .unwrap();
                let label = format!("{width}x{rem}: track {track:?}");

                assert!(near(track.right(), px(width)), "{label}");
                assert!(near(track.left(), px(SIDEBAR_DEFAULT * zoom)), "{label}");
                assert!(near(track.top(), track_top), "{label}");
                assert!(
                    near(track.bottom(), px(HEIGHT - TITLEBAR_DRAG_HEIGHT * zoom)),
                    "{label}"
                );
                let pane = window.debug_bounds("SETTINGS_MISSIONS_PANE").unwrap();
                assert!(
                    near(pane.top(), content_top),
                    "{label}: pane {pane:?} content {content_top:?}"
                );

                // The scrollbar entity's root carries no selector, so prove its
                // right-anchored, top-inset track by where a click jumps the
                // scroll: above the thumb jumps to the top, below it to the
                // bottom, and a click outside the gutter changes nothing.
                let max = scroll.max_offset().y;
                assert!(max > px(0.), "{label}: the pane should overflow");
                let inside_left = px(width - GUTTER * zoom + 1.);
                let outside_left = px(width - GUTTER * zoom - 1.);
                let inside_right = px(width - 1.);
                let below_top = track.top() + px(1.);
                let bottom = track.bottom() - px(1.);
                let below_bottom = track.bottom() + px(1.);
                let mut click_from = |from: Pixels, at: Point<Pixels>| {
                    scroll.set_offset(point(px(0.), from));
                    host.update(&mut window, |_, window, _| window.refresh())
                        .unwrap();
                    window.run_until_parked();
                    window.simulate_click(at, Modifiers::default());
                    scroll.offset().y
                };
                assert!(
                    near(click_from(-max, point(inside_left, below_top)), px(0.)),
                    "{label}: a click on the gutter's left edge should jump to the top"
                );
                assert!(
                    near(click_from(-max, point(inside_right, below_top)), px(0.)),
                    "{label}: a click on the gutter's right edge should jump to the top"
                );
                assert!(
                    near(click_from(px(0.), point(inside_right, bottom)), -max),
                    "{label}: a click at the track's bottom should jump to the bottom"
                );
                assert!(
                    near(click_from(-max, point(outside_left, below_top)), -max),
                    "{label}: a click left of the gutter should not scroll"
                );
                if SETTINGS_DRAG_INSET > 0. {
                    assert!(
                        near(
                            click_from(-max, point(inside_right, track.top() - px(1.))),
                            -max
                        ),
                        "{label}: a click in the drag strip should not scroll"
                    );
                }
                assert!(
                    near(
                        click_from(px(0.), point(inside_right, below_bottom)),
                        px(0.)
                    ),
                    "{label}: a click below the track should not scroll"
                );
            }
        }
    }

    #[test]
    fn settings_back_button_clears_window_navigation() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{size, TestAppContext, VisualTestContext};

        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let window = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "settings-back".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.route = AppRoute::Settings;
            root
        });
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        for zoom in [1.0, 1.3] {
            for width in [1440., 2400.] {
                visual.simulate_resize(size(px(width), px(900.)));
                window
                    .update(&mut cx, |root, window, cx| {
                        root.set_zoom(zoom, window, cx);
                    })
                    .unwrap();
                cx.run_until_parked();
                let back = visual.debug_bounds("SETTINGS_BACK").unwrap();
                let sidebar_top = if cfg!(windows) { 12. + 32. } else { 52. };
                assert!(
                    back.top()
                        >= visual
                            .debug_bounds("WINDOW_TITLEBAR_CONTROLS")
                            .unwrap()
                            .bottom()
                        && back.top() >= px(sidebar_top * zoom)
                        && back.top() < px((sidebar_top + 16.) * zoom),
                    "zoom {zoom} width {width}: {back:?}"
                );
            }
        }
    }

    #[test]
    fn launch_dims_prefer_direct_layouts_and_fall_back_to_mission_geometry() {
        let direct = HashMap::from([("direct".to_owned(), (132, 38))]);

        assert_eq!(
            launch_dims_for("direct", &direct, (104, 42)),
            Some((132, 38))
        );
        assert_eq!(
            launch_dims_for("mission", &direct, (104, 42)),
            Some((104, 42))
        );
    }
}

#[cfg(all(test, debug_assertions))]
mod localization_tests {
    use super::*;
    use gpui::{size, TestAppContext};
    use i18n::{testing::with_locale, Language, Locale};

    #[test]
    fn language_switch_refreshes_two_windows_menus_and_cached_choices_without_resetting_drafts() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        with_locale(Locale::English, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            store.update(&mut cx, |store, _| {
                store.settings.language = Language::English
            });
            let windows = ["locale-one", "locale-two"].map(|label| {
                cx.add_window(|window, cx| {
                    window.resize(size(px(1200.), px(900.)));
                    let mut root = NativeRoot::new(
                        label.into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    );
                    root.route = AppRoute::Settings;
                    root
                })
            });
            cx.run_until_parked();
            let draft = windows[1]
                .read_with(&cx, |root, _| {
                    root.settings_page.default_working_dir.clone()
                })
                .unwrap();
            draft.update(&mut cx, |field, cx| {
                field.set_text("/tmp/中文 {count} draft", cx);
                field.select_all(cx);
            });
            cx.run_until_parked();
            let caret = draft.read_with(&cx, |field, _| field.caret_offset());
            let before = store.read_with(&cx, |store, _| store.revisions);
            for (language, expected, menu) in [
                (Language::SimplifiedChinese, "每次询问", "文件"),
                (Language::English, "Ask", "File"),
            ] {
                windows[0]
                    .update(&mut cx, |root, window, cx| {
                        root.apply_settings_selection(
                            SettingsSelection::Language,
                            language.key(),
                            window,
                            cx,
                        )
                    })
                    .unwrap();
                cx.run_until_parked();
                for window in windows {
                    window
                        .read_with(&cx, |root, cx| {
                            assert_eq!(root.settings(cx).language, language);
                            assert_eq!(
                                root.settings_page.language.read(cx).value(),
                                language.key()
                            );
                            assert_eq!(
                                root.store_revisions.locale,
                                store.read(cx).revisions.locale
                            );
                            assert_eq!(
                                root.settings_page
                                    .quit_behavior
                                    .read(cx)
                                    .selected()
                                    .unwrap()
                                    .label,
                                expected
                            );
                        })
                        .unwrap();
                }
                let menus = cx.update(|cx| cx.get_menus().unwrap());
                assert_eq!(menus[1].name, menu);
                assert!(
                    matches!(&menus[1].items[0], gpui::OwnedMenuItem::Action { name, .. } if name == if language == Language::English { "New Window" } else { "新建窗口" })
                );
                #[cfg(target_os = "macos")]
                assert!(
                    matches!(&menus[0].items[0], gpui::OwnedMenuItem::Action { name, .. } if name == if language == Language::English { "About Runner" } else { "关于 Runner" })
                );
                draft.read_with(&cx, |field, _| {
                    assert_eq!(field.text(), "/tmp/中文 {count} draft");
                    assert_eq!(field.caret_offset(), caret);
                });
                let same_draft = windows[1]
                    .read_with(&cx, |root, _| {
                        root.settings_page.default_working_dir.clone()
                    })
                    .unwrap();
                assert_eq!(draft.entity_id(), same_draft.entity_id());
            }
            let after = store.read_with(&cx, |store, _| store.revisions);
            assert_eq!(after.locale, before.locale + 2);
            assert_eq!(after.terminal_settings, before.terminal_settings);
            assert_eq!(after.role_surfaces, before.role_surfaces);
            assert_eq!(after.crews, before.crews);
        });
    }

    #[test]
    fn general_language_dropdown_persists_keyboard_choices_across_windows() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        with_locale(Locale::English, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let windows = ["language-select-one", "language-select-two"].map(|label| {
                cx.add_window(|window, cx| {
                    window.resize(size(px(1200.), px(900.)));
                    let mut root = NativeRoot::new(
                        label.into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    );
                    root.route = AppRoute::Settings;
                    root
                })
            });
            cx.run_until_parked();
            let select = windows[0]
                .read_with(&cx, |root, _| root.settings_page.language.clone())
                .unwrap();
            let mut visual = gpui::VisualTestContext::from_window(windows[0].into(), &cx);
            for (keys, language, expected) in [
                ("space end enter", Language::SimplifiedChinese, "简体中文"),
                ("space home down enter", Language::English, "English"),
                ("space home enter", Language::System, "System"),
            ] {
                windows[0]
                    .update(&mut visual, |_, window, cx| {
                        select.read(cx).focus_handle().focus(window, cx)
                    })
                    .unwrap();
                visual.simulate_keystrokes(keys);
                visual.run_until_parked();
                for window in windows {
                    window
                        .read_with(&visual, |root, cx| {
                            let selected = root.settings_page.language.read(cx);
                            assert_eq!(selected.value(), language.key());
                            assert_eq!(selected.selected().unwrap().label, expected);
                            assert_eq!(root.settings(cx).language, language);
                        })
                        .unwrap();
                }
                assert_eq!(
                    AppSettings::load(&temp.path().join("settings.json"))
                        .unwrap()
                        .language,
                    language
                );
                assert!(visual.debug_bounds("SETTINGS_LANGUAGE_HEADING").is_none());
                let control = visual.debug_bounds("SETTINGS_LANGUAGE_SELECT").unwrap();
                assert_eq!(control.size.width, SETTINGS_SELECT_WIDTH);
                windows[0]
                    .read_with(&visual, |root, _| {
                        assert_eq!(root.settings_page.language.entity_id(), select.entity_id())
                    })
                    .unwrap();
            }
            store.update(&mut visual, |store, cx| {
                store.sync_locale_from(Some("zh-Hans"), cx)
            });
            visual.run_until_parked();
            select.read_with(&visual, |select, _| {
                assert_eq!(select.value(), "system");
                assert_eq!(select.selected().unwrap().label, "跟随系统");
            });
            windows[0]
                .update(&mut visual, |_, window, cx| {
                    select.read(cx).focus_handle().focus(window, cx)
                })
                .unwrap();
            visual.simulate_keystrokes("space home enter");
            visual.run_until_parked();
            assert_eq!(
                store.read_with(&visual, |store, _| store.resolved_locale),
                Locale::English
            );
            select.read_with(&visual, |select, _| {
                assert_eq!(select.value(), "system");
                assert_eq!(select.selected().unwrap().label, "System");
            });
        });
    }

    #[test]
    fn language_row_keeps_readable_text_at_narrow_widths_and_enlarged_zoom() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        with_locale(Locale::English, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let host = cx.add_window(|window, cx| {
                window.resize(size(px(800.), px(600.)));
                let mut root = NativeRoot::new(
                    "language-layout".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                root.route = AppRoute::Settings;
                root
            });
            cx.run_until_parked();
            let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
            for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
                for language in [Language::English, Language::SimplifiedChinese] {
                    for width in [800., 1200.] {
                        for zoom in [1., 1.3] {
                            visual.simulate_resize(size(px(width), px(600.)));
                            host.update(&mut visual, |root, window, cx| {
                                root.set_theme_intent(intent, cx);
                                root.set_zoom(zoom, window, cx);
                                root.apply_settings_selection(
                                    SettingsSelection::Language,
                                    language.key(),
                                    window,
                                    cx,
                                );
                            })
                            .unwrap();
                            visual.run_until_parked();
                            let row = visual.debug_bounds("SETTINGS_WRAPPED_ROW").unwrap();
                            let text = visual.debug_bounds("SETTINGS_WRAPPED_ROW_TEXT").unwrap();
                            let description = visual
                                .debug_bounds("SETTINGS_WRAPPED_ROW_DESCRIPTION")
                                .unwrap();
                            let control = visual.debug_bounds("SETTINGS_LANGUAGE_SELECT").unwrap();
                            let case = format!("{intent:?}/{language:?}/{width}/{zoom}");
                            assert!(
                                description.size.width >= px(200. * zoom),
                                "{case}: {description:?}"
                            );
                            assert!(
                                description.size.height <= px(3. * 22. * zoom),
                                "{case}: {description:?}"
                            );
                            assert!(
                                text.left() >= row.left()
                                    && text.right() <= row.right()
                                    && control.left() >= row.left()
                                    && control.right() <= row.right()
                                    && control.bottom() <= row.bottom(),
                                "{case}: {row:?} {text:?} {control:?}"
                            );
                            assert!(
                                f32::from(control.size.width - px(176. * zoom)).abs() <= 1.,
                                "{case}"
                            );
                            if width == 800. && zoom == 1.3 {
                                assert!(control.top() >= text.bottom(), "{case}");
                                assert!(
                                    f32::from(control.left() - text.left()).abs() <= 1.,
                                    "{case}"
                                );
                            } else if width == 1200. {
                                assert!(control.left() >= text.right(), "{case}");
                                assert!(control.top() < text.bottom(), "{case}");
                            }
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn open_archive_confirmation_follows_the_other_window_language() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        with_locale(Locale::English, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            store.update(&mut cx, |store, _| {
                store.settings.language = Language::English
            });
            let windows = ["message-one", "message-two"].map(|label| {
                cx.add_window(|window, cx| {
                    NativeRoot::new(
                        label.into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    )
                })
            });
            store.update(&mut cx, |store, _| {
                store
                    .sessions
                    .push(runner_core::protocol::session::DirectSessionEntry {
                        claude_account: None,
                        session_id: "owned-shell".into(),
                        project_id: None,
                        role_id: None,
                        handle: None,
                        agent_runtime: "shell".into(),
                        agent_command: "zsh".into(),
                        agent_model: None,
                        agent_effort: None,
                        display_name: "literal {count} 终端".into(),
                        status: SessionStatus::Running,
                        title: None,
                        live_title: None,
                        cwd: None,
                        started_at: None,
                        stopped_at: None,
                        resumable: false,
                        native_fork: false,
                        forkable: false,
                        agent_session_key: None,
                        pinned: false,
                        archived_at: None,
                    })
            });
            windows[1]
                .update(&mut cx, |root, window, cx| {
                    root.request_archive_all(
                        None,
                        vec!["owned-shell".into()],
                        ArchiveAllSource::Chat,
                        window,
                        cx,
                    );
                })
                .unwrap();
            cx.run_until_parked();
            let confirm_focus = windows[1]
                .read_with(&cx, |root, _| {
                    root.terminal_close_confirm.as_ref().unwrap().focus.clone()
                })
                .unwrap();
            for (language, body) in [
                (
                    Language::SimplifiedChinese,
                    "此操作将永久关闭 1 个终端。已关闭的终端无法恢复。",
                ),
                (
                    Language::English,
                    "This permanently closes 1 terminal. Closed terminals cannot be restored.",
                ),
            ] {
                windows[0]
                    .update(&mut cx, |root, window, cx| {
                        root.apply_settings_selection(
                            SettingsSelection::Language,
                            language.key(),
                            window,
                            cx,
                        )
                    })
                    .unwrap();
                cx.run_until_parked();
                windows[1]
                    .read_with(&cx, |root, _| {
                        let confirm = root.terminal_close_confirm.as_ref().unwrap();
                        let TerminalCloseTarget::ArchiveAll {
                            confirmation_body,
                            session_ids,
                            ..
                        } = &confirm.target
                        else {
                            panic!("archive target retained")
                        };
                        assert_eq!(confirmation_body.resolve(), body);
                        assert_eq!(session_ids, &["owned-shell"]);
                        assert_eq!(confirm.focus, confirm_focus);
                    })
                    .unwrap();
            }
        });
    }

    #[test]
    fn system_resolution_changes_invalidate_even_when_the_saved_intent_does_not_change() {
        with_locale(Locale::English, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let before = store.read_with(&cx, |store, _| store.revisions);
            store.update(&mut cx, |store, cx| {
                store.sync_locale_from(Some("zh-Hant-HK"), cx)
            });
            store.read_with(&cx, |store, _| {
                assert_eq!(store.settings.language, Language::System);
                assert_eq!(store.resolved_locale, Locale::SimplifiedChinese);
                assert_eq!(store.revisions.locale, before.locale + 1);
                assert_eq!(store.revisions.settings, before.settings);
                let reactions = store.revisions.reactions_since(before);
                assert!(reactions.notify_shell());
                assert!(!reactions.apply_terminal_settings);
                assert!(!reactions.reload_role_surfaces);
                assert!(!reactions.reload_crew_surfaces);
            });
            assert_eq!(
                cx.update(|cx| cx.get_menus().unwrap()[1].name.clone()),
                "文件"
            );
            store.update(&mut cx, |store, cx| {
                store.sync_locale_from(Some("en-US"), cx)
            });
            assert_eq!(
                cx.update(|cx| cx.get_menus().unwrap()[1].name.clone()),
                "File"
            );
        });
    }

    #[test]
    fn shortcut_search_translates_while_override_ids_and_combinations_stay_stable() {
        let overrides: keymap::KeymapOverrides = serde_json::from_str(
            r#"{"new-chat":{"ctrl":true,"meta":false,"alt":false,"shift":true,"code":"KeyJ"}}"#,
        )
        .unwrap();
        let before = serde_json::to_value(&overrides).unwrap();
        let binding = keymap::effective_binding("new-chat", &overrides).unwrap();
        with_locale(Locale::SimplifiedChinese, || {
            let rows = shortcut_rows("新建对话", &overrides);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id(), "new-chat");
            assert_eq!(rows[0].title(), "新建对话");
            assert_eq!(
                keymap::effective_binding("new-chat", &overrides),
                Some(binding)
            );
            assert_eq!(serde_json::to_value(&overrides).unwrap(), before);
            assert!(shortcut_rows("打开另一个", &overrides)
                .iter()
                .any(|row| row.id() == "new-window"));
        });
    }
}
