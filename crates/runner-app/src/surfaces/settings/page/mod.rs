mod appearance;
mod general;
mod render;
mod shortcuts;
mod side_panel;

use appearance::*;
pub(crate) use general::*;
#[cfg(test)]
use shortcuts::*;
use side_panel::*;

use std::path::{Path, PathBuf};

use std::rc::Rc;

use std::time::Duration;

use gpui::prelude::*;

use gpui::{
    div, px, rems, svg, AnyElement, BoxShadow, Context, CursorStyle, Entity, FontWeight,
    KeyDownEvent, PathPromptOptions, Pixels, ScrollHandle, SharedString, Subscription, Window,
};

#[cfg(target_os = "macos")]
use runner_app::terminal_ime::OptionAsMeta;

use runner_app::ui::{
    working_dir_text_field, Button, ButtonSize, ButtonVariant, IconButton, IconButtonSize,
    PaneHeader, Scrollbar, SelectHandler, SelectLeading, SelectOption, SettingsCard, SettingsRow,
    StepHandler, Stepper, StyledSelect, TextField, Toggle, WorkingDirField,
};

use runner_core::protocol::command::{
    CommandActionOutcome, RunnerCommandState, RunnerCommandStatus,
};

use crate::surfaces::*;

use crate::app_settings::{
    normalize_terminal_line_height, normalize_terminal_scrollback, normalize_zoom, nudge_zoom,
    DarkTerminalTheme, LightTerminalTheme, TerminalCursorStyle, TerminalFontFamily,
    TERMINAL_FONT_SIZE_MAX, TERMINAL_FONT_SIZE_MIN, TERMINAL_LINE_HEIGHT_MAX,
    TERMINAL_LINE_HEIGHT_MIN, TERMINAL_SCROLLBACK_PRESETS, ZOOM_STEPS,
};

use crate::platform_ui::{SETTINGS_CONTENT_TOP, SETTINGS_DRAG_INSET};

use crate::surfaces::app_shell::TITLEBAR_DRAG_HEIGHT;

use crate::surfaces::settings::theme_preview::{self, PreviewMode, PreviewPane};

use crate::theme::{DarkTheme, LightTheme, ThemeIntent};

use crate::*;

const SETTINGS_SAVE_DELAY_MS: u64 = 300;

/// The palette selects share one trigger width, so the rows of each palette
/// card line up.
const SETTINGS_SELECT_WIDTH: Pixels = px(176.);

const FILE_ICONS_SELECT_WIDTH: Pixels = px(200.);

const TAB_SHORTCUT_ID: &str = "select-tab";

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
    SidePanel,
    Files,
    Agents,
    Usage,
    Skills,
    Mcp,
    Updates,
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
            Some("side-panel") => Self::SidePanel,
            Some("files") => Self::Files,
            Some("agents") => Self::Agents,
            Some("usage") => Self::Usage,
            Some("mcp") => Self::Mcp,
            Some("skills") => Self::Skills,
            Some("updates") => Self::Updates,
            Some("about") | Some("diagnostics") => Self::About,
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
            Self::SidePanel => "side-panel",
            Self::Files => "files",
            Self::Agents => "agents",
            Self::Usage => "usage",
            Self::Skills => "skills",
            Self::Mcp => "mcp",
            Self::Updates => "updates",
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
            Self::SidePanel => crate::i18n::text("side_panel.panel"),
            Self::Files => crate::i18n::text("ui.files"),
            Self::Agents => crate::i18n::text("ui.agents"),
            Self::Usage => crate::i18n::text("ui.usage"),
            Self::Skills => crate::i18n::text("ui.skills"),
            Self::Mcp => "MCP",
            Self::Updates => crate::i18n::text("ui.updates"),
            Self::About => crate::i18n::text("ui.about"),
            Self::Archived => crate::i18n::text("ui.archived"),
        }
    }

    /// Row labels the Settings search matches besides the pane's own label.
    fn search_terms(self) -> Vec<&'static str> {
        let keys: &[&'static str] = match self {
            Self::Appearance => &[
                "ui.theme",
                "ui.window_material",
                "ui.app_palette",
                "ui.light_mode",
                "ui.dark_mode",
            ],
            Self::Terminal => &[
                "ui.font_family",
                "ui.font_size",
                "terminal.line_height",
                "terminal.section_colors",
                "ui.light_mode_palette",
                "ui.dark_mode_palette",
                "ui.cursor_style",
                "terminal.blink_cursor",
                #[cfg(target_os = "macos")]
                "terminal.option_as_meta",
                "terminal.scrollback",
                "terminal.copy_on_select",
            ],
            Self::About => &["ui.application_logs"],
            _ => &[],
        };
        keys.iter().map(|key| crate::i18n::text(key)).collect()
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
            Self::SidePanel => "panel-right-open.svg",
            Self::Files => "folder.svg",
            Self::Agents => "bot.svg",
            Self::Usage => "gauge.svg",
            Self::Skills => "file-text.svg",
            Self::Mcp => "plug.svg",
            Self::Updates => "refresh-cw.svg",
            Self::About => "info.svg",
            Self::Archived => "archive.svg",
        }
    }
}

const APP_PANES: &[SettingsPane] = &[
    SettingsPane::General,
    SettingsPane::Appearance,
    SettingsPane::Notifications,
    SettingsPane::Missions,
    SettingsPane::Terminal,
    SettingsPane::Shortcuts,
    SettingsPane::Archived,
];

const SIDE_PANEL_PANES: &[SettingsPane] = &[SettingsPane::SidePanel, SettingsPane::Files];

const INTEGRATION_PANES: &[SettingsPane] = &[SettingsPane::Agents, SettingsPane::Usage];

const CLOUD_PANES: &[SettingsPane] = &[SettingsPane::Account, SettingsPane::Remote];

const SYSTEM_PANES: &[SettingsPane] = &[SettingsPane::Updates, SettingsPane::About];

const NAV_GROUPS: &[(&str, &[SettingsPane])] = &[
    ("ui.app", APP_PANES),
    ("side_panel.group", SIDE_PANEL_PANES),
    ("ui.integrations", INTEGRATION_PANES),
    ("ui.cloud", CLOUD_PANES),
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
                .chain(
                    (!query.is_empty() && *panes == INTEGRATION_PANES)
                        .then_some([SettingsPane::Skills, SettingsPane::Mcp])
                        .into_iter()
                        .flatten(),
                )
                .filter(|pane| {
                    query.is_empty()
                        || std::iter::once(pane.label())
                            .chain(pane.search_terms())
                            .any(|term| term.to_lowercase().contains(&query))
                })
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
    TerminalScrollback,
    #[cfg(target_os = "macos")]
    OptionAsMeta,
    QuitBehavior,
    FileIcons,
    FilesFont,
    MarkdownOpenAs,
    UsageThreshold,
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
    terminal_scrollback: Entity<StyledSelect>,
    #[cfg(target_os = "macos")]
    pub(crate) option_as_meta: Entity<StyledSelect>,
    file_icons: Entity<StyledSelect>,
    files_font: Entity<StyledSelect>,
    markdown_open_as: Entity<StyledSelect>,
    pub(crate) usage_threshold: Entity<StyledSelect>,
    pub(crate) usage_interval: Entity<StyledSelect>,
    pub(crate) language: Entity<StyledSelect>,
    pub(crate) quit_behavior: Entity<StyledSelect>,
    agents: Option<Entity<settings::agents::AgentsPane>>,
    updates: Option<Entity<settings::updates::UpdatesPane>>,
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
        let usage_threshold = settings_select(
            &root,
            "usage-threshold",
            settings.notifications.usage.threshold.to_string(),
            [50, 60, 70, 80, 90]
                .into_iter()
                .map(|n| SelectOption::new(n.to_string(), format!("{n}%")))
                .collect(),
            SettingsSelection::UsageThreshold,
            cx,
        );
        let interval_root = root.clone();
        let usage_interval = cx.new(|cx| {
            StyledSelect::new(
                "usage-interval",
                cx.focus_handle(),
                "900",
                [300, 900, 1800, 3600]
                    .into_iter()
                    .map(|n| {
                        SelectOption::new(
                            n.to_string(),
                            crate::i18n::message(match n {
                                300 => "usage.every_5",
                                900 => "usage.every_15",
                                1800 => "usage.every_30",
                                _ => "usage.every_hour",
                            }),
                        )
                    })
                    .collect(),
                Rc::new(move |value, _, cx| {
                    interval_root.update(cx, |root, cx| {
                        if let Ok(seconds) = value.parse() {
                            if let Err(error) = root.core(cx).usage_set_interval(seconds) {
                                if let Ok(seconds) = root.core(cx).usage_interval() {
                                    let select = root.settings_page.usage_interval.clone();
                                    cx.defer(move |cx| {
                                        select.update(cx, |select, cx| {
                                            select.set_value(seconds.to_string(), cx)
                                        })
                                    });
                                }
                                root.show_toast(
                                    error.to_string(),
                                    crate::toast::ToastTone::Error,
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });
                }),
                cx,
            )
            .width(px(140.))
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
        let font_cache = crate::terminal::fonts::FontFamilyCache::global(cx);
        let terminal_font = settings_select(
            &root,
            "settings-terminal-font",
            settings.terminal_font_family.family(),
            terminal_font_options(&settings.terminal_font_family, cx),
            SettingsSelection::TerminalFont,
            cx,
        );
        let font_subscription = cx.observe(&font_cache, |this, _, cx| {
            this.sync_terminal_font_options(cx);
            cx.notify();
        });
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
        let terminal_scrollback = settings_select(
            &root,
            "settings-terminal-scrollback",
            settings.terminal_scrollback_lines.to_string(),
            terminal_scrollback_options(settings.terminal_scrollback_lines),
            SettingsSelection::TerminalScrollback,
            cx,
        );
        #[cfg(target_os = "macos")]
        let option_as_meta = settings_select(
            &root,
            "settings-option-as-meta",
            settings.option_as_meta.key(),
            OptionAsMeta::ALL
                .into_iter()
                .map(|value| {
                    SelectOption::new(value.key(), i18n::message(value.label_key()))
                        .description(i18n::message(value.description_key()))
                })
                .collect(),
            SettingsSelection::OptionAsMeta,
            cx,
        );
        let file_icons = settings_select(
            &root,
            "settings-file-icons",
            settings.file_icons.key(),
            file_icon_options(),
            SettingsSelection::FileIcons,
            cx,
        );
        let files_font = settings_select(
            &root,
            "settings-files-font",
            files_font_value(settings.files_font_family.as_ref()),
            files_font_options(settings.files_font_family.as_ref(), cx),
            SettingsSelection::FilesFont,
            cx,
        );
        let markdown_open_as = settings_select(
            &root,
            "settings-markdown-open-as",
            settings.markdown_open_as.key(),
            markdown_open_as_options(),
            SettingsSelection::MarkdownOpenAs,
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
            terminal_scrollback,
            #[cfg(target_os = "macos")]
            option_as_meta,
            file_icons,
            files_font,
            markdown_open_as,
            usage_threshold,
            usage_interval,
            language,
            quit_behavior,
            agents: None,
            updates: None,
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
                font_subscription,
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

fn terminal_font_options(family: &TerminalFontFamily, cx: &App) -> Vec<SelectOption> {
    use crate::terminal::fonts::{FontFamilyCache, BUNDLED_FONT};
    let mut options = vec![SelectOption::new(BUNDLED_FONT, "JetBrains Mono")
        .section(crate::i18n::message("ui.bundled"))];
    if let Some(families) = FontFamilyCache::loaded(cx) {
        let mut installed = families
            .monospace
            .iter()
            .map(|name| SelectOption::new(name, name.clone()))
            .collect::<Vec<_>>();
        if let Some(first) = installed.first_mut() {
            *first = first.clone().section(crate::i18n::message("ui.installed"));
        }
        options.extend(installed);
        if !options.iter().any(|option| option.value == family.family()) {
            let mut saved = SelectOption::new(family.family(), family.family().to_owned())
                .section(crate::i18n::message("ui.installed"));
            if !families.all.iter().any(|name| name == family.family()) {
                saved = saved.description(crate::i18n::message("ui.not_installed"));
            }
            options.push(saved);
        }
    } else if family.family() != BUNDLED_FONT {
        options.push(SelectOption::new(
            family.family(),
            family.family().to_owned(),
        ));
    }
    options
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
        // Room for the sample icons beside the longest name, Catppuccin.
        SettingsSelection::FileIcons => Some(FILE_ICONS_SELECT_WIDTH),
        SettingsSelection::Language
        | SettingsSelection::LightTheme
        | SettingsSelection::DarkTheme
        | SettingsSelection::LightTerminalTheme
        | SettingsSelection::DarkTerminalTheme => Some(SETTINGS_SELECT_WIDTH),
        SettingsSelection::DefaultCrew
        | SettingsSelection::TerminalFont
        | SettingsSelection::TerminalCursor
        | SettingsSelection::TerminalScrollback
        | SettingsSelection::QuitBehavior
        | SettingsSelection::FilesFont
        | SettingsSelection::MarkdownOpenAs
        | SettingsSelection::UsageThreshold => None,
        #[cfg(target_os = "macos")]
        SettingsSelection::OptionAsMeta => None,
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
        let select = if matches!(
            selection,
            SettingsSelection::TerminalFont | SettingsSelection::FilesFont
        ) {
            select
                .searchable(crate::i18n::message("ui.search_fonts"), select_cx)
                .width(px(220.))
        } else {
            select
        };
        let select = if matches!(selection, SettingsSelection::QuitBehavior) {
            select.min_menu_width(px(300.))
        } else {
            select
        };
        #[cfg(target_os = "macos")]
        let select = if matches!(selection, SettingsSelection::OptionAsMeta) {
            select.wrapped_menu(px(350.))
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
        if self.remote_client.is_active() {
            return;
        }
        if matches!(pane, SettingsPane::Skills | SettingsPane::Mcp) {
            self.enter_settings_pane(SettingsPane::Agents, window, cx);
            let tab = if pane == SettingsPane::Skills {
                settings::agents::AgentTab::Skills
            } else {
                settings::agents::AgentTab::Mcp
            };
            if let Some(agents) = self.settings_page.agents.clone() {
                agents.update(cx, |pane, cx| pane.open_tab(tab, cx));
            }
            return;
        }

        if self.settings_page.pane != pane {
            self.dismiss_remote_dialog();
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
            SettingsPane::Usage => {
                let _ = self
                    .core(cx)
                    .usage_refresh(runner_core::protocol::RefreshReason::Open);
                if let Ok(seconds) = self.core(cx).usage_interval() {
                    self.settings_page
                        .usage_interval
                        .update(cx, |select, cx| select.set_value(seconds.to_string(), cx));
                }
            }
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
            SettingsPane::Skills | SettingsPane::Mcp => {}
            SettingsPane::Updates => {
                if self.settings_page.updates.is_none() {
                    let app_store = self.app_store.clone();
                    let updater = runner_app::updater::global_updater(cx);
                    let log_dir = self.log_dir.clone();
                    let root = cx.weak_entity();
                    self.settings_page.updates = Some(cx.new(|pane_cx| {
                        settings::updates::UpdatesPane::new(
                            app_store, updater, log_dir, root, pane_cx,
                        )
                    }));
                }
                if let Some(updates) = self.settings_page.updates.clone() {
                    updates.update(cx, |pane, pane_cx| pane.refresh(pane_cx));
                }
            }
            SettingsPane::About => {
                if self.settings_page.about.is_none() {
                    let log_dir = self.log_dir.clone();
                    self.settings_page.about =
                        Some(cx.new(move |_| settings::about::AboutPane::new(log_dir)));
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
            | SettingsPane::SidePanel
            | SettingsPane::Files
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
        if self.remote_client.is_active() {
            return;
        }

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
            SettingsSelection::TerminalFont => update_if_changed(
                &mut settings.terminal_font_family,
                TerminalFontFamily::new(value),
            ),
            SettingsSelection::TerminalCursor => parse_terminal_cursor(value)
                .is_some_and(|value| update_if_changed(&mut settings.terminal_cursor_style, value)),
            SettingsSelection::TerminalScrollback => value
                .parse::<usize>()
                .ok()
                .filter(|lines| normalize_terminal_scrollback(*lines) == *lines)
                .is_some_and(|lines| {
                    update_if_changed(&mut settings.terminal_scrollback_lines, lines)
                }),
            #[cfg(target_os = "macos")]
            SettingsSelection::OptionAsMeta => OptionAsMeta::ALL
                .into_iter()
                .find(|pick| pick.key() == value)
                .is_some_and(|value| update_if_changed(&mut settings.option_as_meta, value)),
            SettingsSelection::QuitBehavior => runner_app::lifecycle::QuitBehavior::ALL
                .into_iter()
                .find(|pick| pick.key() == value)
                .is_some_and(|value| update_if_changed(&mut settings.quit_behavior, value)),
            SettingsSelection::FileIcons => crate::file_icons::IconSet::from_key(value)
                .is_some_and(|value| update_if_changed(&mut settings.file_icons, value)),
            SettingsSelection::FilesFont => update_if_changed(
                &mut settings.files_font_family,
                (!value.is_empty()).then(|| TerminalFontFamily::new(value)),
            ),
            SettingsSelection::UsageThreshold => value
                .parse::<u8>()
                .ok()
                .filter(|n| [50, 60, 70, 80, 90].contains(n))
                .is_some_and(|n| update_if_changed(&mut settings.notifications.usage.threshold, n)),
            SettingsSelection::MarkdownOpenAs => crate::app_settings::MarkdownOpenAs::parse(value)
                .is_some_and(|value| update_if_changed(&mut settings.markdown_open_as, value)),
        });
        if !changed {
            if matches!(selection, SettingsSelection::Language) {
                self.app_store.update(cx, |store, cx| store.sync_locale(cx));
            }
            return;
        }
        cx.notify();
    }

    fn sync_terminal_font_options(&self, cx: &mut Context<Self>) {
        let family = self.settings(cx).terminal_font_family.clone();
        let options = terminal_font_options(&family, cx);
        self.settings_page.terminal_font.update(cx, |select, cx| {
            select.set_value(family.family(), cx);
            select.set_options(options, cx);
        });
        self.sync_files_font_options(cx);
    }

    fn sync_files_font_options(&self, cx: &mut Context<Self>) {
        let files = self.settings(cx).files_font_family.clone();
        let options = files_font_options(files.as_ref(), cx);
        self.settings_page.files_font.update(cx, |select, cx| {
            select.set_value(files_font_value(files.as_ref()), cx);
            select.set_options(options, cx);
        });
    }

    /// Settings → Side panel's selects follow a change made in another
    /// window, as the General selects do.
    pub(crate) fn sync_side_panel_selects(&self, cx: &mut Context<Self>) {
        let settings = self.settings(cx);
        let (icons, markdown) = (settings.file_icons.key(), settings.markdown_open_as.key());
        let font = files_font_value(settings.files_font_family.as_ref());
        self.settings_page
            .file_icons
            .update(cx, |select, cx| select.set_value(icons, cx));
        self.settings_page
            .markdown_open_as
            .update(cx, |select, cx| select.set_value(markdown, cx));
        if self.settings_page.files_font.read(cx).value() != font {
            self.sync_files_font_options(cx);
        }
    }

    pub(crate) fn sync_terminal_scrollback_select(&self, cx: &mut Context<Self>) {
        let lines = self.settings(cx).terminal_scrollback_lines;
        self.settings_page
            .terminal_scrollback
            .update(cx, |select, cx| {
                select.set_options(terminal_scrollback_options(lines), cx);
                if select.value() != lines.to_string() {
                    select.set_value(lines.to_string(), cx);
                }
            });
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

    fn set_terminal_line_height(&mut self, line_height: f32, cx: &mut Context<Self>) {
        let line_height = normalize_terminal_line_height(
            line_height.clamp(TERMINAL_LINE_HEIGHT_MIN, TERMINAL_LINE_HEIGHT_MAX),
        );
        if self.update_app_settings(cx, true, |settings| {
            update_if_changed(&mut settings.terminal_line_height, line_height)
        }) {
            cx.notify();
        }
    }

    fn set_terminal_cursor_blink(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.update_app_settings(cx, true, |settings| {
            update_if_changed(&mut settings.terminal_cursor_blink, enabled)
        }) {
            cx.notify();
        }
    }

    fn set_terminal_copy_on_select(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.update_app_settings(cx, true, |settings| {
            update_if_changed(&mut settings.terminal_copy_on_select, enabled)
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
        if self.remote_client.is_active() {
            return;
        }
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
        self.sync_terminal_font_options(cx);
        self.apply_terminal_palette(cx);
        for chat in self.attached.values() {
            app_settings::configure_terminal(&chat.terminal, self.settings(cx));
        }
        self.remote_client.configure_remote_terminals(cx);
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

    pub(crate) fn open_claude_accounts(&self, cx: &mut Context<Self>) {
        if let Some(agents) = &self.settings_page.agents {
            agents.update(cx, |agents, cx| {
                agents.open_runtime(runner_core::protocol::Runtime::ClaudeCode, cx)
            });
        }
    }

    pub(crate) fn refresh_agents_tasks(
        &self,
        tasks: &[runner_core::protocol::RuntimeUpdateTask],
        cx: &mut Context<Self>,
    ) {
        if let Some(agents) = &self.settings_page.agents {
            agents.update(cx, |agents, cx| {
                agents.tasks = tasks.to_vec();
                cx.notify();
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
}

fn update_if_changed<T: PartialEq>(target: &mut T, value: T) -> bool {
    if *target == value {
        return false;
    }
    *target = value;
    true
}

#[cfg(test)]
mod tests;

#[cfg(all(test, debug_assertions))]
mod localization_tests;
