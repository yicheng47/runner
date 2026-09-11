use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, BoxShadow, Context, CursorStyle, Entity, FontWeight,
    KeyDownEvent, PathPromptOptions, ScrollHandle, SharedString, Subscription, Window,
};
use runner_app::ui::{
    working_dir_text_field, Button, ButtonSize, IconButton, IconButtonSize, PaneHeader, Scrollbar,
    SelectHandler, SelectOption, SettingsCard, SettingsRow, StepHandler, Stepper, StyledSelect,
    TextField, Toggle, WorkingDirField,
};

use super::*;
use crate::app_settings::{
    normalize_zoom, nudge_zoom, FileLinkEditor, MissionPermissionMode, TerminalCursorStyle,
    TerminalFontFamily, TerminalTheme, TERMINAL_FONT_SIZE_MAX, TERMINAL_FONT_SIZE_MIN,
    TERMINAL_SCROLLBACK_LINES, ZOOM_STEPS,
};
use crate::surfaces::app_shell::TITLEBAR_DRAG_HEIGHT;
use crate::theme::{DarkTheme, LightTheme, ThemeIntent};
use crate::*;

const SETTINGS_SAVE_DELAY_MS: u64 = 300;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SettingsPane {
    #[default]
    General,
    Missions,
    Appearance,
    Terminal,
    Shortcuts,
    Agents,
    Skills,
    Updates,
    Diagnostics,
    About,
    Archived,
}

impl SettingsPane {
    fn from_route(value: Option<&str>) -> Self {
        match value {
            Some("missions") => Self::Missions,
            Some("appearance") => Self::Appearance,
            Some("terminal") => Self::Terminal,
            Some("shortcuts") => Self::Shortcuts,
            Some("agents") | Some("mcp") => Self::Agents,
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
            Self::General => "general",
            Self::Missions => "missions",
            Self::Appearance => "appearance",
            Self::Terminal => "terminal",
            Self::Shortcuts => "shortcuts",
            Self::Agents => "agents",
            Self::Skills => "skills",
            Self::Updates => "updates",
            Self::Diagnostics => "diagnostics",
            Self::About => "about",
            Self::Archived => "archived",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Missions => "Missions",
            Self::Appearance => "Appearance",
            Self::Terminal => "Terminal",
            Self::Shortcuts => "Keyboard shortcuts",
            Self::Agents => "Agents",
            Self::Skills => "Skills",
            Self::Updates => "Updates",
            Self::Diagnostics => "Diagnostics",
            Self::About => "About",
            Self::Archived => "Archived",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::General => "settings.svg",
            Self::Missions => "rocket.svg",
            Self::Appearance => "sun.svg",
            Self::Terminal => "terminal.svg",
            Self::Shortcuts => "keyboard.svg",
            Self::Agents => "bot.svg",
            Self::Skills => "file-text.svg",
            Self::Updates => "refresh-cw.svg",
            Self::Diagnostics => "file-text.svg",
            Self::About => "info.svg",
            Self::Archived => "archive.svg",
        }
    }
}

const APP_PANES: &[SettingsPane] = &[
    SettingsPane::General,
    SettingsPane::Appearance,
    SettingsPane::Missions,
    SettingsPane::Terminal,
    SettingsPane::Shortcuts,
    SettingsPane::Archived,
];
const INTEGRATION_PANES: &[SettingsPane] = &[SettingsPane::Agents, SettingsPane::Skills];
const SYSTEM_PANES: &[SettingsPane] = &[
    SettingsPane::Updates,
    SettingsPane::Diagnostics,
    SettingsPane::About,
];
const NAV_GROUPS: &[(&str, &[SettingsPane])] = &[
    ("App", APP_PANES),
    ("Integrations", INTEGRATION_PANES),
    ("System", SYSTEM_PANES),
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
            (!panes.is_empty()).then_some((*label, panes))
        })
        .collect()
}

fn launch_dims_for(
    session_id: &str,
    direct_sizes: &HashMap<String, (u16, u16)>,
    mission_size: (u16, u16),
) -> Option<(u16, u16)> {
    direct_sizes.get(session_id).copied().or(Some(mission_size))
}

fn alpha(mut color: gpui::Hsla, value: f32) -> gpui::Hsla {
    color.a = value;
    color
}

#[derive(Clone, Copy)]
enum SettingsSelection {
    DefaultCrew,
    MissionPermissions,
    LightTheme,
    DarkTheme,
    TerminalTheme,
    TerminalFont,
    TerminalCursor,
    FileLinkEditor,
}

pub(crate) struct SettingsState {
    pane: SettingsPane,
    focus: FocusHandle,
    search_query: String,
    search: Entity<TextField>,
    shortcut_query: String,
    shortcut_search: Entity<TextField>,
    shortcut_recording: Option<&'static str>,
    shortcut_conflict: Option<ShortcutConflict>,
    shortcut_recording_focus: FocusHandle,
    default_crew: Entity<StyledSelect>,
    mission_permissions: Entity<StyledSelect>,
    default_working_dir: Entity<TextField>,
    working_dir_browse_focus: FocusHandle,
    light_theme: Entity<StyledSelect>,
    dark_theme: Entity<StyledSelect>,
    terminal_theme: Entity<StyledSelect>,
    terminal_font: Entity<StyledSelect>,
    terminal_cursor: Entity<StyledSelect>,
    file_link_editor: Entity<StyledSelect>,
    agents: Option<Entity<settings::agents::AgentsPane>>,
    skills: Option<Entity<settings::skills::SkillsPane>>,
    updates: Option<Entity<settings::updates::UpdatesPane>>,
    diagnostics: Option<Entity<settings::diagnostics::DiagnosticsPane>>,
    about: Option<Entity<settings::about::AboutPane>>,
    archived: Option<Entity<settings::archived::ArchivedPane>>,
    nav_scroll: ScrollHandle,
    nav_scrollbar: Entity<Scrollbar>,
    content_scroll: ScrollHandle,
    content_scrollbar: Entity<Scrollbar>,
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
            let mut input = TextField::new(input_cx.focus_handle(), "", "Search settings…", false)
                .text_size(theme::text_body());
            input.set_bare(true, input_cx);
            input
        });
        let shortcut_search = cx.new(|input_cx| {
            let mut input = TextField::new(input_cx.focus_handle(), "", "Search shortcuts", false)
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
            vec![SelectOption::new("", "No default")],
            SettingsSelection::DefaultCrew,
            cx,
        );
        let mission_permissions = settings_select(
            &root,
            "settings-mission-permissions",
            settings.mission_permission_mode.key(),
            MissionPermissionMode::ALL
                .into_iter()
                .map(|mode| {
                    SelectOption::new(mode.key(), mode.label())
                        .description(mission_permission_mode_description(mode))
                })
                .collect(),
            SettingsSelection::MissionPermissions,
            cx,
        );
        let light_theme = settings_select(
            &root,
            "settings-light-theme",
            light_theme_value(settings.light_app_theme),
            vec![
                SelectOption::new("codex", "Codex Light").swatch(0x339cff),
                SelectOption::new("catppuccin-latte", "Catppuccin Latte").swatch(0x8839ef),
            ],
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
        let terminal_theme = settings_select(
            &root,
            "settings-terminal-theme",
            terminal_theme_value(settings.terminal_theme),
            vec![
                SelectOption::new("runner", "Runner").swatch(0x00ff9c),
                SelectOption::new("catppuccin-mocha", "Catppuccin Mocha").swatch(0xcba6f7),
                SelectOption::new("monokai", "Monokai").swatch(0xff6188),
            ],
            SettingsSelection::TerminalTheme,
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
                        "block" => "Block",
                        "underline" => "Underline",
                        _ => "Bar",
                    };
                    SelectOption::new(value, label)
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
                .map(|editor| SelectOption::new(editor.key(), editor.label()))
                .collect(),
            SettingsSelection::FileLinkEditor,
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
            mission_permissions,
            default_working_dir,
            working_dir_browse_focus: cx.focus_handle(),
            light_theme,
            dark_theme,
            terminal_theme,
            terminal_font,
            terminal_cursor,
            file_link_editor,
            agents: None,
            skills: None,
            updates: None,
            diagnostics: None,
            about: None,
            archived: None,
            nav_scroll,
            nav_scrollbar,
            content_scroll,
            content_scrollbar,
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
    message: String,
}

/// The content column: the titlebar drag strip, the padded scroll container
/// with its centered column, and the scrollbar in an overlay inset by the
/// drag strip's height at both ends, so the thumb never runs behind the
/// traffic lights or into the window's bottom corner (#553). The overlay has
/// no id or handlers, so wheel and click pass through it to the scroll
/// container.
fn settings_content_column(
    titlebar_drag_area: impl IntoElement,
    content: Option<AnyElement>,
    scroll: &ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    zoom: f32,
) -> impl IntoElement {
    div()
        .relative()
        .min_w(px(0.))
        .h_full()
        .flex_1()
        .bg(theme::bg())
        .child(titlebar_drag_area)
        .child(
            div()
                .id("settings-content-scroll")
                .size_full()
                .overflow_y_scroll()
                .scrollbar_width(px(0.))
                .track_scroll(scroll)
                .px(rems(40. / 16.))
                .pb(rems(64. / 16.))
                .pt(rems(56. / 16.))
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
                .top(px(TITLEBAR_DRAG_HEIGHT * zoom))
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

fn missions_settings_pane(
    default_crew: Entity<StyledSelect>,
    mission_permissions: Entity<StyledSelect>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_5()
        .debug_selector(|| "SETTINGS_MISSIONS_PANE".into())
        .child(PaneHeader::new("Missions", ""))
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
                        .child("Defaults"),
                )
                .child(
                    div()
                        .debug_selector(|| "SETTINGS_MISSIONS_CARD".into())
                        .child(SettingsCard::new(vec![
                            SettingsRow::new(
                                "Default crew",
                                div()
                                    .debug_selector(|| "SETTINGS_DEFAULT_CREW".into())
                                    .child(default_crew),
                            )
                            .subtitle("Pre-selected when starting a new mission.")
                            .into_any_element(),
                            SettingsRow::new(
                                "Mission permissions",
                                div()
                                    .debug_selector(|| "SETTINGS_MISSION_PERMISSIONS".into())
                                    .child(mission_permissions),
                            )
                            .subtitle(
                                "Applied to every slot when a mission starts. Bypass never prompts — nobody is watching a mission slot to answer. Direct chats keep their runner's own mode.",
                            )
                            .into_any_element(),
                        ])),
                ),
        )
        .into_any_element()
}

fn mission_permission_mode_description(mode: MissionPermissionMode) -> &'static str {
    match mode {
        MissionPermissionMode::Bypass => "Default. No prompts; codex gets full access.",
        MissionPermissionMode::Auto => "The runtime's classifier decides; may stall on a prompt.",
        MissionPermissionMode::RunnerDefault => "Whatever each runner row carries.",
    }
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
    cx.new(|select_cx| {
        StyledSelect::new(
            id,
            select_cx.focus_handle(),
            value,
            options,
            handler,
            select_cx,
        )
    })
}

impl NativeRoot {
    pub(crate) fn focus_settings_page(&self, window: &mut Window) {
        self.settings_page.focus.focus(window);
    }

    pub(crate) fn shortcut_recording_active(&self) -> bool {
        self.settings_page.shortcut_recording.is_some()
    }

    fn refresh_settings_crews(&self, cx: &mut Context<Self>) {
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let conn = core.db.get().map_err(|error| error.to_string())?;
            runner_backend::ops::crew::list(&conn)
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
                let mut options = vec![SelectOption::new("", "No default")];
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

    pub(crate) fn start_launch_auto_resume(&self, window: &mut Window, cx: &mut Context<Self>) {
        let enabled = self.settings(cx).resume_on_launch;
        let mut direct_sizes = HashMap::new();
        for layout in self.tabs.tabs() {
            for leaf in layout.root.leaves() {
                let Some(session_id) = leaf.session_id.as_deref() else {
                    continue;
                };
                let size = self
                    .attached
                    .get(session_id)
                    .map(|chat| chat.terminal.size())
                    .unwrap_or_else(|| self.estimated_terminal_size(layout, &leaf.id, window, cx));
                direct_sizes.insert(session_id.to_owned(), size);
            }
        }
        let mission_size = self.estimated_mission_terminal_size(window, cx);
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_app::bootstrap::consume_resume_on_launch(&core, enabled, |session_id| {
                launch_dims_for(session_id, &direct_sizes, mission_size)
            })
        });
        cx.spawn_in(window, async move |weak, cx| match task.await {
            Ok(report) => {
                for error in report.errors {
                    eprintln!("Runner launch auto-resume failed: {error}");
                }
                if !report.resumed.is_empty() {
                    let _ = weak.update_in(cx, |this, _, cx| {
                        this.refresh_sessions(cx);
                        this.refresh_store(StoreRefreshKind::All, cx);
                        cx.notify();
                    });
                }
            }
            Err(error) => eprintln!("Runner launch auto-resume queue failed: {error:#}"),
        })
        .detach();
    }

    pub(crate) fn enter_settings_pane(
        &mut self,
        pane: SettingsPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                    agents.update(cx, |pane, pane_cx| pane.refresh(pane_cx));
                }
            }
            SettingsPane::Skills => {
                if self.settings_page.skills.is_none() {
                    let app_store = self.app_store.clone();
                    self.settings_page.skills =
                        Some(cx.new(|cx| settings::skills::SkillsPane::new(app_store, cx)));
                }
                if let Some(skills) = self.settings_page.skills.clone() {
                    skills.update(cx, |pane, cx| pane.refresh(cx));
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
            | SettingsPane::Shortcuts => {}
        }
        window.focus(&self.settings_page.focus);
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
            SettingsSelection::DefaultCrew => {
                update_if_changed(&mut settings.default_crew_id, value.to_owned())
            }
            SettingsSelection::MissionPermissions => MissionPermissionMode::parse(value)
                .is_some_and(|value| {
                    update_if_changed(&mut settings.mission_permission_mode, value)
                }),
            SettingsSelection::LightTheme => parse_light_theme(value)
                .is_some_and(|value| update_if_changed(&mut settings.light_app_theme, value)),
            SettingsSelection::DarkTheme => parse_dark_theme(value)
                .is_some_and(|value| update_if_changed(&mut settings.dark_app_theme, value)),
            SettingsSelection::TerminalTheme => parse_terminal_theme(value)
                .is_some_and(|value| update_if_changed(&mut settings.terminal_theme, value)),
            SettingsSelection::TerminalFont => parse_terminal_font(value)
                .is_some_and(|value| update_if_changed(&mut settings.terminal_font_family, value)),
            SettingsSelection::TerminalCursor => parse_terminal_cursor(value)
                .is_some_and(|value| update_if_changed(&mut settings.terminal_cursor_style, value)),
            SettingsSelection::FileLinkEditor => FileLinkEditor::parse(value)
                .is_some_and(|value| update_if_changed(&mut settings.file_link_editor, value)),
        });
        if !changed {
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

    pub(crate) fn apply_terminal_settings(&self, cx: &mut Context<Self>) {
        let cursor = match self.settings(cx).terminal_cursor_style {
            TerminalCursorStyle::Block => alacritty_terminal::vte::ansi::CursorShape::Block,
            TerminalCursorStyle::Underline => alacritty_terminal::vte::ansi::CursorShape::Underline,
            TerminalCursorStyle::Bar => alacritty_terminal::vte::ansi::CursorShape::Beam,
        };
        for chat in self.attached.values() {
            chat.terminal
                .set_palette(self.settings(cx).terminal_theme.palette());
            chat.terminal.configure(TERMINAL_SCROLLBACK_LINES, cursor);
        }
    }

    fn browse_settings_working_dir(&mut self, cx: &mut Context<Self>) {
        let input = self.settings_page.default_working_dir.clone();
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Pick a working directory".into()),
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
        _window: &mut Window,
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
                .child("No matching settings.")
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
            .key_context("Settings")
            .track_focus(&self.settings_page.focus)
            .inset_0()
            .flex()
            .overflow_hidden()
            .bg(theme::bg())
            .occlude()
            .child(
                div()
                    .relative()
                    .w(px(width))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .bg(theme::sidebar())
                    .border_r_1()
                    .border_color(theme::border())
                    .child(self.render_titlebar_drag_area(
                        "settings-sidebar-titlebar-drag",
                        div().h(px(32. * self.settings(cx).app_zoom)).flex_none(),
                        cx,
                    ))
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
                                    .border_color(theme::sidebar_selected_border())
                                    .bg(theme::sidebar_selected())
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
                    )
                    .child(self.render_sidebar_resize_handle(cx)),
            )
            .child(settings_content_column(
                self.render_titlebar_drag_area(
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
            ))
            .when(active == SettingsPane::Skills, |takeover| {
                takeover.children(
                    self.settings_page
                        .skills
                        .as_ref()
                        .map(|pane| pane.read(cx).detail.clone()),
                )
            })
            .into_any_element()
    }

    fn render_settings_back_button(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("settings-back")
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
                    .border_color(theme::sidebar_selected_border())
                    .bg(alpha(theme::sidebar_selected(), 0.4))
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
            .child(div().text_size(theme::text_body()).child("Back to app"))
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
                theme::sidebar_selected_border()
            } else {
                gpui::transparent_black()
            })
            .bg(if active {
                theme::sidebar_selected()
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
            .when(active, |row| row.shadow_sm())
            .when(!active, |row| {
                row.hover(|row| {
                    row.border_color(theme::sidebar_selected_border())
                        .bg(alpha(theme::sidebar_selected(), 0.4))
                        .text_color(theme::text())
                })
            })
            .focus_visible(|row| {
                row.shadow(vec![BoxShadow {
                    color: theme::border_strong(),
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
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

    pub(crate) fn refresh_agents_pane(&self, cx: &mut Context<Self>) {
        if let Some(agents) = self.settings_page.agents.clone() {
            agents.update(cx, |pane, pane_cx| pane.refresh(pane_cx));
        }
    }

    pub(crate) fn pause_updates_pane(&self, cx: &mut Context<Self>) {
        if let Some(updates) = self.settings_page.updates.clone() {
            updates.update(cx, |pane, _| pane.pause());
        }
    }

    pub(crate) fn settings_confirm_overlay(
        &self,
        cx: &App,
    ) -> Option<Entity<settings::archived::ArchivedConfirmOverlay>> {
        (self.route == AppRoute::Settings)
            .then_some(self.settings_page.archived.as_ref())
            .flatten()
            .map(|archived| archived.read(cx).confirm_overlay())
    }

    fn render_shortcuts_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let overrides = self.settings(cx).keymap_overrides.clone();
        let query = self.settings_page.shortcut_query.trim().to_lowercase();
        let rows = keymap::entries()
            .iter()
            .filter(|entry| {
                if query.is_empty() {
                    return true;
                }
                let binding = keymap::effective_binding(entry.id, &overrides)
                    .map(|combo| keymap::format_combo(&combo))
                    .unwrap_or_else(|| "unassigned".into());
                entry.title.to_lowercase().contains(&query)
                    || entry.description.to_lowercase().contains(&query)
                    || binding.to_lowercase().contains(&query)
            })
            .map(|entry| self.render_shortcut_row(entry, cx))
            .collect::<Vec<_>>();
        let has_overrides = !overrides.is_empty();
        let reset_root = cx.entity();
        let reset = Button::new("reset-keymap", "Reset all to defaults")
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
                .child(format!(
                    "No shortcuts match “{}”.",
                    self.settings_page.shortcut_query.trim()
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
                    "Keyboard shortcuts",
                    crate::platform_ui::SHORTCUT_REQUIREMENTS,
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
        entry: &'static keymap::KeymapEntry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let binding = keymap::effective_binding(entry.id, &self.settings(cx).keymap_overrides);
        let overridden = self.settings(cx).keymap_overrides.contains_key(entry.id);
        let recording = self.settings_page.shortcut_recording == Some(entry.id);
        let conflict = self
            .settings_page
            .shortcut_conflict
            .as_ref()
            .filter(|conflict| conflict.id == entry.id)
            .map(|conflict| conflict.message.clone());
        let label = div()
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
                    .child(entry.title),
            )
            .child(
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(entry.description),
            )
            .children(conflict.map(|message| {
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::danger())
                    .child(message)
            }));

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
                        .id(SharedString::from(format!(
                            "shortcut-recorder-{}",
                            entry.id
                        )))
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
                        .child("Press keys…")
                        .on_key_down(move |event: &KeyDownEvent, window, cx| {
                            key_root.update(cx, |this, root_cx| {
                                this.record_shortcut_key(entry.id, event, window, root_cx)
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

        let binding_label = binding
            .as_ref()
            .map(keymap::format_combo)
            .unwrap_or_else(|| "Unassigned".into());
        let controls = if entry.fixed {
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .items_center()
                .child(shortcut_chip(binding_label, false))
                .into_any_element()
        } else {
            let chip_root = cx.entity();
            let edit_root = chip_root.clone();
            let restore_root = chip_root.clone();
            let mut controls = div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .items_center()
                .gap(rems(6. / 16.))
                .child(
                    shortcut_chip(binding_label, true)
                        .id(SharedString::from(format!("binding-shortcut-{}", entry.id)))
                        .on_click(move |_, window, cx| {
                            chip_root.update(cx, |this, root_cx| {
                                this.start_shortcut_recording(entry.id, window, root_cx)
                            });
                        }),
                )
                .child(
                    IconButton::new(format!("edit-shortcut-{}", entry.id), "pencil.svg")
                        .size(IconButtonSize::Sm)
                        .tooltip("Edit shortcut")
                        .on_press(move |window, cx| {
                            edit_root.update(cx, |this, root_cx| {
                                this.start_shortcut_recording(entry.id, window, root_cx)
                            });
                        }),
                );
            if overridden {
                controls = controls.child(
                    IconButton::new(format!("restore-shortcut-{}", entry.id), "rotate-ccw.svg")
                        .size(IconButtonSize::Sm)
                        .tooltip("Restore default")
                        .on_press(move |window, cx| {
                            restore_root.update(cx, |this, root_cx| {
                                this.restore_shortcut_default(entry.id, window, root_cx)
                            });
                        }),
                );
            }
            controls.into_any_element()
        };
        let unbind = if entry.fixed {
            div().w_6().flex_none().into_any_element()
        } else {
            let root = cx.entity();
            IconButton::new(format!("unbind-shortcut-{}", entry.id), "trash.svg")
                .size(IconButtonSize::Sm)
                .tooltip("Unbind shortcut")
                .on_press(move |window, cx| {
                    root.update(cx, |this, root_cx| {
                        this.set_shortcut_override(entry.id, None, window, root_cx)
                    });
                })
                .into_any_element()
        };

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
        self.settings_page.shortcut_recording_focus.focus(window);
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
        self.settings_page.focus.focus(window);
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
                message: format!("Already used by {}", conflict.title),
            });
            cx.notify();
            return;
        }
        self.set_shortcut_override(id, Some(combo), window, cx);
    }

    fn set_shortcut_override(
        &mut self,
        id: &'static str,
        value: Option<keymap::KeyCombo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if keymap::entry(id).is_none_or(|entry| entry.fixed) {
            return;
        }
        self.update_app_settings(cx, true, |settings| {
            if settings.keymap_overrides.get(id) == Some(&value) {
                return false;
            }
            settings.keymap_overrides.insert(id.into(), value);
            true
        });
        self.settings_page.shortcut_recording = None;
        self.settings_page.shortcut_conflict = None;
        self.rebuild_key_bindings(false, cx);
        self.settings_page.focus.focus(window);
        cx.notify();
    }

    fn restore_shortcut_default(
        &mut self,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut overrides = self.settings(cx).keymap_overrides.clone();
        match keymap::clear_override(id, &mut overrides) {
            Err(conflict) => {
                self.settings_page.shortcut_conflict = Some(ShortcutConflict {
                    id,
                    message: format!("Already used by {}", conflict.title),
                });
                cx.notify();
            }
            Ok(false) => {}
            Ok(true) => {
                self.update_app_settings(cx, true, |settings| {
                    settings.keymap_overrides = overrides;
                    true
                });
                self.settings_page.shortcut_conflict = None;
                self.rebuild_key_bindings(false, cx);
                self.settings_page.focus.focus(window);
                cx.notify();
            }
        }
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
        self.settings_page.focus.focus(window);
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
                .font_family(theme::SYSTEM_MONOSPACE_FONT)
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
        let shell_env = self
            .app_store
            .read(cx)
            .core
            .runtime_shell_env
            .read()
            .map(|env| env.clone())
            .unwrap_or_default();
        let (file_link_hint, file_link_warns) = file_link_hint(
            file_link_editor,
            crate::file_links::cli_found(file_link_editor, &shell_env),
        );
        let file_link_row = SettingsRow::new(
            "Open file links in",
            self.settings_page.file_link_editor.clone(),
        )
        .subtitle(file_link_hint);
        let file_link_row = if file_link_warns {
            file_link_row.subtitle_color(theme::warning())
        } else {
            file_link_row
        };
        let toggle_root = cx.entity();

        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new("General", "Defaults and startup behavior."))
            .child(SettingsCard::new(vec![
                SettingsRow::new(
                    "Default working directory",
                    div().w(rems(280. / 16.)).child(working_dir),
                )
                .subtitle("Default for new chats. Leave blank to use your home directory.")
                .into_any_element(),
                file_link_row.into_any_element(),
                SettingsRow::new("App zoom", zoom)
                    .subtitle(
                        "Whole-app scale. Doesn't apply to the runner terminal canvas — see Terminal pane.",
                    )
                    .into_any_element(),
            ]))
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
                            .child("STARTUP"),
                    )
                    .child(SettingsCard::new(vec![SettingsRow::new(
                        "Resume running agents on launch",
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
                        "Reopen chats and mission agents that were live when Runner quit.",
                    )
                    .into_any_element()])),
            )
            .into_any_element()
    }

    fn render_missions_settings(&self, _cx: &mut Context<Self>) -> AnyElement {
        missions_settings_pane(
            self.settings_page.default_crew.clone(),
            self.settings_page.mission_permissions.clone(),
        )
    }

    fn render_appearance_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new("Appearance", "Theme, palette, and font."))
            .child(SettingsCard::new(vec![
                SettingsRow::new("Theme", self.render_theme_segmented(cx))
                    .subtitle("Match the OS, or pin to light or dark.")
                    .into_any_element(),
                SettingsRow::new("Light theme", self.settings_page.light_theme.clone())
                    .subtitle("Picked when the OS is light or Theme = Light.")
                    .into_any_element(),
                SettingsRow::new("Dark theme", self.settings_page.dark_theme.clone())
                    .subtitle("Picked when the OS is dark or Theme = Dark.")
                    .into_any_element(),
            ]))
            .into_any_element()
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
                    (ThemeIntent::Auto, "Auto", "monitor.svg"),
                    (ThemeIntent::Light, "Light", "sun.svg"),
                    (ThemeIntent::Dark, "Dark", "moon.svg"),
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
                .font_family(theme::SYSTEM_MONOSPACE_FONT)
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
            .child(PaneHeader::new(
                "Terminal",
                "xterm appearance settings for the runner terminal.",
            ))
            .child(SettingsCard::new(vec![
                SettingsRow::new("Theme", self.settings_page.terminal_theme.clone())
                    .subtitle("ANSI palette for the embedded terminal.")
                    .into_any_element(),
                SettingsRow::new("Font family", self.settings_page.terminal_font.clone())
                    .subtitle("Typeface used by the embedded terminal.")
                    .into_any_element(),
                SettingsRow::new("Terminal font size", size_stepper)
                    .subtitle("Glyph size for the embedded terminal.")
                    .into_any_element(),
                SettingsRow::new("Cursor style", self.settings_page.terminal_cursor.clone())
                    .subtitle("Block, underline, or bar — affects the prompt caret only.")
                    .into_any_element(),
            ]))
            .into_any_element()
    }
}

fn shortcut_chip(label: String, editable: bool) -> gpui::Div {
    div()
        .px_2()
        .py_1()
        .rounded(rems(4. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::raised())
        .font_family(theme::SYSTEM_MONOSPACE_FONT)
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
            format!(
                "{}-click opens the file in its default app. The cited line is lost.",
                crate::platform_ui::PRIMARY_MODIFIER
            ),
            false,
        ),
        (editor, Some(false)) => (
            format!(
                "{} not found on PATH — {}.",
                editor.cli().unwrap_or_default(),
                editor.install_hint().unwrap_or_default()
            ),
            true,
        ),
        (editor, _) => (
            format!(
                "{}-click opens the file at the cited line via {}.",
                crate::platform_ui::PRIMARY_MODIFIER,
                editor.cli().unwrap_or_default()
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

fn light_theme_value(value: LightTheme) -> &'static str {
    match value {
        LightTheme::Codex => "codex",
        LightTheme::CatppuccinLatte => "catppuccin-latte",
    }
}

fn parse_light_theme(value: &str) -> Option<LightTheme> {
    match value {
        "codex" => Some(LightTheme::Codex),
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

fn terminal_theme_value(value: TerminalTheme) -> &'static str {
    match value {
        TerminalTheme::Runner => "runner",
        TerminalTheme::CatppuccinMocha => "catppuccin-mocha",
        TerminalTheme::Monokai => "monokai",
    }
}

fn parse_terminal_theme(value: &str) -> Option<TerminalTheme> {
    match value {
        "runner" => Some(TerminalTheme::Runner),
        "catppuccin-mocha" => Some(TerminalTheme::CatppuccinMocha),
        "monokai" => Some(TerminalTheme::Monokai),
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
    fn settings_routes_fall_back_to_general() {
        assert_eq!(SettingsPane::from_route(None), SettingsPane::General);
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
        assert_eq!(SettingsPane::Missions.key(), "missions");
        assert_eq!(SettingsPane::Missions.label(), "Missions");
        assert_eq!(SettingsPane::Missions.icon(), "rocket.svg");
    }

    #[test]
    fn nav_search_filters_labels_and_removes_empty_groups() {
        let all = filtered_nav_groups("");
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().map(|(_, panes)| panes.len()).sum::<usize>(), 11);
        assert_eq!(
            all[0],
            (
                "App",
                vec![
                    SettingsPane::General,
                    SettingsPane::Appearance,
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
                vec![SettingsPane::Agents, SettingsPane::Skills]
            )
        );
        assert_eq!(
            SettingsPane::from_route(Some("skills")),
            SettingsPane::Skills
        );
        assert_eq!(SettingsPane::from_route(Some("mcp")), SettingsPane::Agents);
        assert!(filtered_nav_groups("mcp").is_empty());
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
    fn missions_pane_renders_both_rows_at_two_rem_sizes() {
        use gpui::{size, Render, TestAppContext, VisualTestContext};

        struct PaneHost {
            default_crew: Entity<StyledSelect>,
            mission_permissions: Entity<StyledSelect>,
        }
        impl Render for PaneHost {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().p_6().child(missions_settings_pane(
                    self.default_crew.clone(),
                    self.mission_permissions.clone(),
                ))
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
            let mission_permissions = cx.new(|cx| {
                StyledSelect::new(
                    "settings-mission-permissions",
                    cx.focus_handle(),
                    MissionPermissionMode::Auto.key(),
                    MissionPermissionMode::ALL
                        .into_iter()
                        .map(|mode| {
                            SelectOption::new(mode.key(), mode.label())
                                .description(mission_permission_mode_description(mode))
                        })
                        .collect(),
                    noop,
                    cx,
                )
            });
            PaneHost {
                default_crew,
                mission_permissions,
            }
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
            let permissions = window.debug_bounds("SETTINGS_MISSION_PERMISSIONS").unwrap();
            assert!(
                heading.size.height >= px(rem) && heading.bottom() <= card.top(),
                "{rem}: {heading:?} {card:?}"
            );
            assert!(
                card.size.width > px(0.) && card.size.width <= pane.size.width,
                "{rem}: {card:?} {pane:?}"
            );
            assert!(
                crew.bottom() <= permissions.top(),
                "{rem}: {crew:?} {permissions:?}"
            );
            assert!(
                permissions.bottom() <= card.bottom() && permissions.right() <= card.right(),
                "{rem}: {permissions:?} {card:?}"
            );
            assert!(
                permissions.size.width > px(rem * 4.) && permissions.size.height >= px(rem * 1.5),
                "{rem}: {permissions:?}"
            );
        }
        host.update(&mut window, |host, _, cx| {
            assert_eq!(host.mission_permissions.read(cx).value(), "auto");
        })
        .unwrap();
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
            mission_permissions: Entity<StyledSelect>,
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
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(px(TITLEBAR_DRAG_HEIGHT * zoom)),
                        Some(missions_settings_pane(
                            self.default_crew.clone(),
                            self.mission_permissions.clone(),
                        )),
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
            let mission_permissions = cx.new(|cx| {
                StyledSelect::new(
                    "settings-mission-permissions",
                    cx.focus_handle(),
                    MissionPermissionMode::Auto.key(),
                    MissionPermissionMode::ALL
                        .into_iter()
                        .map(|mode| SelectOption::new(mode.key(), mode.label()))
                        .collect(),
                    noop,
                    cx,
                )
            });
            let content_scroll = ScrollHandle::new();
            let owner = cx.entity_id();
            let content_scrollbar = cx.new(|_| Scrollbar::app(content_scroll.clone(), owner));
            ColumnHost {
                default_crew,
                mission_permissions,
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
                window.simulate_resize(size(px(width), px(HEIGHT)));
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
                assert!(
                    near(track.top(), px(TITLEBAR_DRAG_HEIGHT * zoom)),
                    "{label}"
                );
                assert!(
                    near(track.bottom(), px(HEIGHT - TITLEBAR_DRAG_HEIGHT * zoom)),
                    "{label}"
                );

                // The scrollbar entity's root carries no selector, so prove its
                // right-anchored, top-inset track by where a click jumps the
                // scroll: above the thumb jumps to the top, below it to the
                // bottom, and a click outside the gutter changes nothing.
                let max = scroll.max_offset().height;
                assert!(max > px(0.), "{label}: the pane should overflow");
                let inside_left = px(width - GUTTER * zoom + 1.);
                let outside_left = px(width - GUTTER * zoom - 1.);
                let inside_right = px(width - 1.);
                let below_top = track.top() + px(1.);
                let above_top = track.top() - px(1.);
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
                assert!(
                    near(click_from(-max, point(inside_right, above_top)), -max),
                    "{label}: a click in the drag strip should not scroll"
                );
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
