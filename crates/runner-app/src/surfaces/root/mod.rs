use crate::*;

mod init;
mod store;

#[derive(Clone)]
pub(crate) struct GlobalNativePaths(pub(crate) NativePaths);

impl Global for GlobalNativePaths {}

pub(crate) const INITIAL_COLS: u16 = 100;

pub(crate) const INITIAL_ROWS: u16 = 30;

pub(crate) const PANE_HEADER_HEIGHT: f32 = 40.;

pub(crate) const WINDOW_STATE_SAVE_DELAY_MS: u64 = 300;

/// How long after landing on a tab a close does nothing. Terminal tabs close
/// with no prompt and cannot be restored, so a double ⌘W must not take the tab
/// the first press landed on.
pub(crate) const TAB_LANDING_CLOSE_GUARD: Duration = Duration::from_millis(500);

pub(crate) struct AttachedChat {
    pub(crate) terminal: Arc<TerminalMirror>,
    pub(crate) _terminal_view: TerminalView,
    pub(crate) terminal_interaction: Entity<TerminalInteraction>,
    pub(crate) terminal_scrollbar: Entity<Scrollbar>,
    pub(crate) terminal_input: Entity<TerminalInput>,
    pub(crate) _terminal_input_subscription: Subscription,
    pub(crate) _terminal_focus_subscription: Option<Subscription>,
    pub(crate) terminal_focus: FocusHandle,
    pub(crate) terminal_find: Entity<crate::terminal::TerminalFind>,
}

#[derive(Clone)]
pub(crate) struct SplitResizeDrag {
    pub(crate) split_id: String,
    pub(crate) orientation: SplitOrientation,
}

#[derive(Clone)]
pub(crate) struct PaneDrag {
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    pub(crate) label: String,
    pub(crate) icon: ChatIcon,
    pub(crate) live: bool,
}

impl Render for PaneDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(rems(220. / 16.))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(rems(6. / 16.))
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .shadow_lg()
            .text_size(theme::text_ui())
            .text_color(theme::text())
            .child(self.icon.render(
                rems(12. / 16.),
                self.icon.color(theme::text(), self.live),
                self.live,
            ))
            .child(div().min_w(px(0.)).truncate().child(self.label.clone()))
    }
}

#[derive(Clone)]
pub(crate) struct DrawerResizeDrag;

impl Render for DrawerResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}

impl Render for SplitResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}

pub(crate) struct SidebarVisibilityTransition {
    pub(crate) start: f32,
    pub(crate) target: f32,
    pub(crate) started_at: Option<Instant>,
}

#[derive(Clone, Copy)]
pub(crate) struct ChatTransition {
    pub(crate) kind: surfaces::chat_lifecycle::TransitionKind,
    pub(crate) started_at: Instant,
    pub(crate) baseline_seq: u64,
    pub(crate) generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CloseTarget {
    ChatDrawer,
    MissionDrawer,
    Pane,
    Tab,
    None,
}

pub(crate) fn close_target(
    route: &AppRoute,
    leaves: usize,
    chat_drawer_focused: bool,
    mission_drawer_focused: bool,
) -> CloseTarget {
    if *route == AppRoute::Chat && chat_drawer_focused {
        CloseTarget::ChatDrawer
    } else if matches!(route, AppRoute::Mission(_)) && mission_drawer_focused {
        CloseTarget::MissionDrawer
    } else if *route == AppRoute::Chat && leaves > 1 {
        CloseTarget::Pane
    } else if *route == AppRoute::Chat && leaves == 1 {
        CloseTarget::Tab
    } else {
        CloseTarget::None
    }
}

pub(crate) struct PaneRename {
    pub(crate) session_id: String,
    pub(crate) original: String,
    pub(crate) input: Entity<runner_app::ui::TextField>,
}

pub(crate) enum TerminalCloseTarget {
    ArchiveChatPane {
        session_id: String,
        close: PendingPaneClose,
    },
    ArchiveAll {
        session_ids: Vec<String>,
        source: ArchiveAllSource,
        confirmation_body: crate::i18n::Text,
    },
}

pub(crate) struct TerminalCloseConfirm {
    pub(crate) target: TerminalCloseTarget,
    pub(crate) focus: FocusHandle,
    pub(crate) previous_focus: Option<FocusHandle>,
}

impl TerminalCloseConfirm {
    pub(crate) fn open(
        pending: &mut Option<Self>,
        target: TerminalCloseTarget,
        window: &mut Window,
        cx: &mut App,
    ) {
        if pending.is_some() {
            return;
        }
        let previous_focus = window.focused(cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        *pending = Some(Self {
            target,
            focus,
            previous_focus,
        });
    }

    pub(crate) fn restore_focus(&self, window: &mut Window, cx: &mut App) {
        if let Some(previous_focus) = &self.previous_focus {
            previous_focus.focus(window, cx);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingPaneClose {
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingTabClose {
    pub(crate) tab_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ForkConfirm {
    pub(crate) session_id: String,
    pub(crate) pending: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum ArchiveAllSource {
    Chat,
    Sidebar,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeLocation {
    Chat(String),
    Mission(String),
    Entity(AppRoute),
}

pub(crate) const RUNTIME_NAVIGATION_HISTORY_LIMIT: usize = 64;

#[derive(Clone)]
pub(crate) struct ChatPanelResizeDrag;

impl Render for ChatPanelResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}

impl SidebarVisibilityTransition {
    pub(crate) fn new(visible: bool) -> Self {
        let value = if visible { 1. } else { 0. };
        Self {
            start: value,
            target: value,
            started_at: None,
        }
    }

    pub(crate) fn animate_to(
        &mut self,
        target: f32,
        now: Instant,
        duration: Duration,
    ) -> (f32, bool) {
        let current = self.value_at(now, duration);
        if self.target != target {
            self.start = current;
            self.target = target;
            self.started_at = Some(now);
        }
        let Some(started_at) = self.started_at else {
            return (self.target, false);
        };
        if now.saturating_duration_since(started_at) >= duration {
            self.start = self.target;
            self.started_at = None;
            return (self.target, false);
        }
        (self.value_at(now, duration), true)
    }

    pub(crate) fn value_at(&self, now: Instant, duration: Duration) -> f32 {
        let Some(started_at) = self.started_at else {
            return self.target;
        };
        let delta = (now.saturating_duration_since(started_at).as_secs_f32()
            / duration.as_secs_f32())
        .clamp(0., 1.);
        self.start + (self.target - self.start) * gpui::ease_in_out(delta)
    }
}

pub(crate) struct NativeRoot {
    pub(crate) composition: Entity<surfaces::render_cache::ShellComposition>,
    pub(crate) remote_client: surfaces::remote_client::RemoteClient,
    pub(crate) render_regions: HashMap<String, Entity<surfaces::render_cache::CachedRegion<Self>>>,
    pub(crate) window_label: String,
    pub(crate) log_dir: PathBuf,
    pub(crate) closing: bool,
    pub(crate) app_store: Entity<AppStore>,
    pub(crate) store_revisions: StoreRevisions,
    pub(crate) tabs: TabSet,
    pub(crate) attached: HashMap<String, AttachedChat>,
    pub(crate) root_focus: FocusHandle,
    pub(crate) chat_focus: FocusHandle,
    pub(crate) drawer_focus: FocusHandle,
    pub(crate) sidebar: Entity<Sidebar>,
    pub(crate) launchers:
        HashMap<String, Entity<crate::surfaces::start_chat::launcher::ChatLauncher>>,
    pub(crate) start_chat_modal: Option<StartChatModal>,
    pub(crate) project_modal: Option<ProjectModal>,
    pub(crate) project_delete_confirm: Option<String>,
    pub(crate) project_delete_busy: bool,
    pub(crate) stopping_sessions: HashSet<String>,
    pub(crate) chat_transitions: HashMap<String, ChatTransition>,
    pub(crate) next_chat_transition_generation: u64,
    pub(crate) session_exit_codes: HashMap<String, Option<i32>>,
    pub(crate) chat_error: Option<crate::i18n::Text>,
    pub(crate) daemon_retrying: bool,
    pub(crate) chat_warning: Option<String>,
    pub(crate) active_chat_detail: Option<DirectSessionEntry>,
    pub(crate) archived_chat_detail: Option<DirectSessionEntry>,
    pub(crate) session_key_copy: Entity<CopyValueButton>,
    pub(crate) archived_session_key_copy: Entity<CopyValueButton>,
    pub(crate) pane_action_menus: HashMap<String, Entity<PopoverMenu>>,
    pub(crate) split_menus: HashMap<SplitMenuKey, Entity<PopoverMenu>>,
    pub(crate) pane_bounds: HashMap<PaneKey, Size<Pixels>>,
    /// Tab id → the pane maximized in it; view state only, never persisted.
    pub(crate) maximized_panes: HashMap<String, String>,
    pub(crate) pane_drop: Option<(PaneKey, DropSide)>,
    pub(crate) pane_rename: Option<PaneRename>,
    pub(crate) _pane_rename_focus_subscription: Option<Subscription>,
    pub(crate) _pane_close_focus_subscription: Option<Subscription>,
    pub(crate) terminal_close_confirm: Option<TerminalCloseConfirm>,
    pub(crate) pending_pane_closes: HashMap<String, PendingPaneClose>,
    pub(crate) pending_tab_close: Option<PendingTabClose>,
    pub(crate) tab_landed_at: Option<Instant>,
    pub(crate) update_popover: surfaces::update_ui::UpdatePopoverState,
    pub(crate) quit_dialog: Option<Entity<surfaces::quit_dialog::QuitDialog>>,
    pub(crate) agent_tasks: surfaces::background_tasks::BackgroundTasks,
    pub(crate) agent_update: Option<Entity<surfaces::agent_update::AgentUpdateDialog>>,
    pub(crate) fork_confirm: Option<ForkConfirm>,
    pub(crate) forking_sessions: HashMap<String, String>,
    pub(crate) last_focused_role_id: Option<String>,
    pub(crate) split_sizes_dirty: bool,
    pub(crate) drawer_resizing: bool,
    pub(crate) sidebar_resizing: bool,
    pub(crate) chat_panel_resizing: bool,
    pub(crate) resizing_split: Option<String>,
    pub(crate) chat_secondaries: HashMap<String, String>,
    pub(crate) dismissed_duplicate_chats: HashSet<String>,
    pub(crate) error: Option<crate::i18n::Text>,
    pub(crate) settings_page: SettingsState,
    pub(crate) route: AppRoute,
    pub(crate) settings_return_route: AppRoute,
    pub(crate) usage_open: bool,
    pub(crate) usage_refresh_task: Option<gpui::Task<()>>,
    pub(crate) mission_usage: crate::surfaces::app_shell::MissionUsageState,
    pub(crate) mission_control: crate::surfaces::mission_control::MissionControlState,
    pub(crate) claude_account_picker: Option<Entity<runner_app::ui::select::StyledSelect>>,
    pub(crate) usage_anchor: Option<Bounds<Pixels>>,
    pub(crate) usage_installed: Vec<runner_core::protocol::model::Runtime>,
    pub(crate) usage_checked: Option<Vec<runner_core::Runtime>>,
    /// Enabled agents with an update dot the sidebar Settings gear.
    pub(crate) agent_updates: Vec<runner_core::protocol::model::Runtime>,
    pub(crate) runtime_navigation_history: Vec<RuntimeLocation>,
    pub(crate) runtime_navigation_index: Option<usize>,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) sidebar_visibility: SidebarVisibilityTransition,
    pub(crate) chat_panel_visibility: SidebarVisibilityTransition,
    pub(crate) chat_panel_prompt_session: Option<String>,
    pub(crate) chat_panel_prompt_expanded: bool,
    pub(crate) chat_surfaces: crate::surfaces::side_panel::PanelSurfaces,
    pub(crate) chat_panel_tab: Option<String>,
    pub(crate) chat_panel_open: Option<bool>,
    pub(crate) chat_panels_by_tab: HashMap<String, crate::surfaces::side_panel::SavedPanel>,
    pub(crate) chat_files: crate::surfaces::files::FilesPanel,
    pub(crate) panel_chat_by_tab: HashMap<String, String>,
    pub(crate) command_palette: Entity<CommandPaletteState>,
    pub(crate) sidebar_preview_open: bool,
    pub(crate) sidebar_preview_peeking: bool,
    pub(crate) sidebar_preview_trigger_hovered: bool,
    pub(crate) sidebar_preview_dwell: u64,
    pub(crate) sidebar_preview_dwell_generation: u64,
    pub(crate) titlebar_drag_armed: bool,
    pub(crate) window_state: window_state::WindowState,
    pub(crate) window_state_save_generation: u64,
    pub(crate) toasts: ToastHost,
    pub(crate) role_surfaces: RoleSurfaces,
    pub(crate) crew_surfaces: CrewSurfaces,
    pub(crate) mission_workspace: Entity<MissionWorkspace>,
    pub(crate) _appearance_subscription: Option<Subscription>,
    pub(crate) _activation_subscription: Option<Subscription>,
    pub(crate) _bounds_subscription: Option<Subscription>,
    pub(crate) _project_cwd_subscription: Option<Subscription>,
    pub(crate) _updater_subscription: Subscription,
    pub(crate) _store_subscription: Subscription,
}

impl NativeRoot {
    /// The `OSC 0/2` title the session's program is reporting, for surfaces
    /// that label a session by what it says it is doing (#587).
    pub(crate) fn attached_title(&self, session_id: &str, cx: &App) -> Option<String> {
        self.app_store
            .read(cx)
            .bridge
            .session(session_id)
            .map(|terminal| terminal.title())
            .or_else(|| {
                self.attached
                    .get(session_id)
                    .map(|chat| chat.terminal.title())
            })
    }

    /// Every attached session's reported title, for surfaces that must tell a
    /// title change apart from ordinary terminal output: a wake fires on every
    /// burst of bytes, and only a few of those carry new words.
    pub(crate) fn attached_titles(&self, cx: &App) -> HashMap<String, String> {
        self.attached
            .iter()
            .map(|(id, chat)| (id.clone(), chat.terminal.title()))
            .chain(self.app_store.read(cx).bridge.titles())
            .collect()
    }

    pub(crate) fn request_model_catalog(&self, runtime: &str, cx: &Context<Self>) {
        if let Some(runtime) = runner_core::protocol::model::Runtime::parse(runtime)
            .filter(|runtime| self.settings(cx).model_runtimes().contains(runtime))
        {
            let _ = self.core(cx).runtime_request_models(&[runtime]);
        }
    }

    pub(crate) fn core<'a>(&self, cx: &'a App) -> &'a DaemonClient {
        &self.app_store.read(cx).client
    }

    pub(crate) fn settings<'a>(&self, cx: &'a App) -> &'a AppSettings {
        &self.app_store.read(cx).settings
    }

    pub(crate) fn refresh_sessions(&self, cx: &mut Context<Self>) {
        self.app_store
            .update(cx, |store, store_cx| store.refresh_sessions(store_cx));
    }

    pub(crate) fn refresh_store(&self, refresh: StoreRefreshKind, cx: &mut Context<Self>) {
        self.app_store
            .update(cx, |store, store_cx| store.refresh(refresh, store_cx));
    }

    pub(crate) fn reload_tabs(&mut self, cx: &mut Context<Self>) -> Result<()> {
        self.app_store
            .update(cx, |store, store_cx| store.refresh_nodes(store_cx))?;
        self.apply_tab_rows(cx);
        Ok(())
    }

    pub(crate) fn replace_tab_rows(
        &mut self,
        rows: &[runner_core::protocol::node::NodeRow],
        cx: &App,
    ) {
        let had_active = self.tabs.active().is_some();
        let closed = self
            .pending_tab_close
            .take_if(|close| !rows.iter().any(|row| row.id == close.tab_id));
        let landing =
            closed.is_some_and(|close| self.tabs.active_tab_id() == Some(close.tab_id.as_str()));
        self.tabs.replace_rows(rows);
        self.chat_panels_by_tab
            .retain(|id, _| rows.iter().any(|row| &row.id == id));
        self.panel_chat_by_tab
            .retain(|id, _| rows.iter().any(|row| &row.id == id));
        if landing {
            self.land_on_recent_tab(cx);
        }
        if had_active
            && self.tabs.active().is_none()
            && self.route != AppRoute::Launcher
            && self.remote_client.is_local()
        {
            self.reset_launcher_page(cx);
        }
    }

    /// Moves off a tab that closed, or a mission that was archived, to the
    /// last viewed tab. With no tab left, the launcher shows.
    pub(crate) fn land_on_recent_tab(&mut self, cx: &App) {
        match self.recently_viewed_tab_id(cx) {
            Some(tab) => {
                self.tabs.activate(&tab);
                self.tab_landed_at = Some(Instant::now());
            }
            None => self.tabs.clear_active(),
        }
    }

    pub(crate) fn apply_tab_rows(&mut self, cx: &mut Context<Self>) {
        let store = self.app_store.clone();
        self.replace_tab_rows(&store.read(cx).nodes, cx);
        self.sync_active_project_from_active_tab(cx);
        self.prune_sidebar_collapse_state(cx);
    }

    pub(crate) fn session_entry<'a>(
        &self,
        session_id: &str,
        cx: &'a App,
    ) -> Option<&'a DirectSessionEntry> {
        self.app_store
            .read(cx)
            .sessions
            .iter()
            .find(|entry| entry.session_id == session_id)
    }
}

impl Render for NativeRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.composition.clone()
    }
}

#[cfg(test)]
mod native_root_tests {
    use super::*;

    struct CloseConfirmKeyboardTest {
        background_focus: FocusHandle,
        confirm: Option<TerminalCloseConfirm>,
        confirmed: usize,
        cancelled: usize,
        background_keys: usize,
        busy: bool,
    }

    impl CloseConfirmKeyboardTest {
        fn dismiss(&mut self, confirmed: bool, window: &mut Window, cx: &mut Context<Self>) {
            self.confirm.take().unwrap().restore_focus(window, cx);
            if confirmed {
                self.confirmed += 1;
            } else {
                self.cancelled += 1;
            }
            cx.notify();
        }
    }

    impl Render for CloseConfirmKeyboardTest {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let dialog = self.confirm.as_ref().map(|confirm| {
                let root = cx.entity();
                let confirm_root = root.clone();
                ConfirmDialog::new(
                    "Archive chat?",
                    "Chat is still running.",
                    "Archive chat",
                    "Archiving…",
                    self.busy,
                    Rc::new(move |window, cx| {
                        confirm_root.update(cx, |this, cx| this.dismiss(true, window, cx));
                    }),
                    Rc::new(move |window, cx| {
                        root.update(cx, |this, cx| this.dismiss(false, window, cx));
                    }),
                )
                .focus_handle(confirm.focus.clone())
            });
            div()
                .size_full()
                .child(
                    div()
                        .track_focus(&self.background_focus)
                        .on_key_down(cx.listener(|this, _: &KeyDownEvent, _, cx| {
                            this.background_keys += 1;
                            cx.stop_propagation();
                        })),
                )
                .children(dialog)
        }
    }

    #[test]
    fn close_confirm_enter_submits_escape_cancels_and_restores_the_background_focus() {
        for (key, busy) in [
            ("enter", false),
            ("escape", false),
            ("enter", true),
            ("escape", true),
        ] {
            let mut cx = gpui::TestAppContext::single();
            let host = cx.add_window(|window, cx| {
                let background_focus = cx.focus_handle();
                background_focus.focus(window, cx);
                CloseConfirmKeyboardTest {
                    background_focus,
                    confirm: None,
                    confirmed: 0,
                    cancelled: 0,
                    background_keys: 0,
                    busy,
                }
            });
            cx.run_until_parked();
            host.update(&mut cx, |this, window, cx| {
                TerminalCloseConfirm::open(
                    &mut this.confirm,
                    TerminalCloseTarget::ArchiveChatPane {
                        session_id: "chat".into(),
                        close: PendingPaneClose {
                            tab_id: "tab".into(),
                            pane_id: "p1".into(),
                        },
                    },
                    window,
                    cx,
                );
                assert!(this.confirm.as_ref().unwrap().focus.is_focused(window));
                let original_focus = this.confirm.as_ref().unwrap().focus.clone();
                TerminalCloseConfirm::open(
                    &mut this.confirm,
                    TerminalCloseTarget::ArchiveChatPane {
                        session_id: "chat".into(),
                        close: PendingPaneClose {
                            tab_id: "tab".into(),
                            pane_id: "p1".into(),
                        },
                    },
                    window,
                    cx,
                );
                assert!(original_focus.is_focused(window));
                cx.notify();
            })
            .unwrap();
            cx.run_until_parked();
            let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
            visual.simulate_keystrokes(key);
            cx.run_until_parked();
            host.update(&mut cx, |this, window, _| {
                assert_eq!(this.confirmed, usize::from(!busy && key == "enter"));
                assert_eq!(this.cancelled, usize::from(!busy && key == "escape"));
                assert_eq!(this.background_keys, 0);
                assert_eq!(this.confirm.is_some(), busy);
                if !busy {
                    assert!(this.background_focus.is_focused(window));
                }
            })
            .unwrap();
            if !busy {
                visual.simulate_keystrokes("a");
                cx.run_until_parked();
                host.update(&mut cx, |this, _, _| assert_eq!(this.background_keys, 1))
                    .unwrap();
            }
        }
    }

    #[test]
    fn reopen_before_startup_ignores_missing_globals() {
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            handle_reopen(cx);
            assert!(cx.windows().is_empty());
        });
    }

    #[test]
    fn close_tab_hides_focused_drawers_then_closes_panes_or_tabs() {
        assert_eq!(
            close_target(&AppRoute::Chat, 2, true, false),
            CloseTarget::ChatDrawer
        );
        assert_eq!(
            close_target(&AppRoute::Mission("mission".into()), 0, false, true),
            CloseTarget::MissionDrawer
        );
        assert_eq!(
            close_target(&AppRoute::Mission("mission".into()), 0, false, false),
            CloseTarget::None
        );
        assert_eq!(
            close_target(&AppRoute::Chat, 2, false, false),
            CloseTarget::Pane
        );
        assert_eq!(
            close_target(&AppRoute::Chat, 1, false, false),
            CloseTarget::Tab
        );
        assert_eq!(
            close_target(&AppRoute::Roles, 0, false, false),
            CloseTarget::None
        );
        assert_eq!(
            close_target(&AppRoute::Crews, 0, false, false),
            CloseTarget::None
        );
        for route in [
            AppRoute::Chat,
            AppRoute::NewRole,
            AppRoute::RoleDetail("role".into()),
            AppRoute::NewCrew,
            AppRoute::CrewEditor("crew".into()),
            AppRoute::ArchivedChat,
            AppRoute::Settings,
        ] {
            assert_eq!(close_target(&route, 0, false, false), CloseTarget::None);
        }
        for route in [
            AppRoute::Roles,
            AppRoute::Crews,
            AppRoute::Settings,
            AppRoute::ArchivedChat,
            AppRoute::Mission("mission".into()),
        ] {
            for leaves in [0, 1, 2] {
                assert_eq!(close_target(&route, leaves, true, false), CloseTarget::None);
            }
        }
    }

    #[cfg(unix)]
    fn tab_rows(ids: [&str; 3]) -> Vec<runner_core::protocol::node::NodeRow> {
        use runner_core::protocol::node::{NodeRow, NodeType};

        ids.map(|id| NodeRow {
            id: id.into(),
            parent_id: None,
            position: 0,
            node_type: NodeType::Tab,
            name: None,
            ref_id: None,
            layout: Some(PaneLayout::single(None, &[]).serialize().unwrap()),
            pinned_position: None,
            last_completed_at: None,
            last_viewed_at: None,
            created_at: String::new(),
        })
        .to_vec()
    }

    #[cfg(unix)]
    #[test]
    fn closing_the_active_tab_lands_once_its_row_is_gone_and_only_an_empty_window_clears() {
        let rows = tab_rows(["first", "middle", "last"]);
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "close-selection".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            for closing in ["first", "middle", "last"] {
                root.tabs = TabSet::from_rows(&rows);
                root.tabs.activate(closing);
                root.tab_landed_at = None;
                root.pending_tab_close = Some(PendingTabClose {
                    tab_id: closing.into(),
                });
                root.replace_tab_rows(&rows, cx);
                assert_eq!(root.tabs.active_tab_id(), Some(closing));
                assert!(root.pending_tab_close.is_some());
                assert!(root.tab_landed_at.is_none());

                let remaining = rows
                    .iter()
                    .filter(|row| row.id != closing)
                    .cloned()
                    .collect::<Vec<_>>();
                root.replace_tab_rows(&remaining, cx);
                assert_eq!(root.tabs.active_tab_id(), Some(remaining[0].id.as_str()));
                assert!(root.pending_tab_close.is_none());
                assert!(root.tab_landed_at.is_some());
                root.replace_tab_rows(&remaining, cx);
                assert_eq!(root.tabs.active_tab_id(), Some(remaining[0].id.as_str()));
            }

            // With no tab left the launcher shows, and stays when one arrives.
            root.tabs = TabSet::from_rows(&rows[..1]);
            root.tab_landed_at = None;
            root.pending_tab_close = Some(PendingTabClose {
                tab_id: "first".into(),
            });
            root.replace_tab_rows(&[], cx);
            assert!(root.tabs.active().is_none());
            assert!(root.tab_landed_at.is_none());
            root.replace_tab_rows(&rows, cx);
            assert!(root.tabs.active().is_none());

            // Closing a tab that is not on screen changes nothing.
            root.tabs = TabSet::from_rows(&rows);
            root.tabs.activate("middle");
            root.pending_tab_close = Some(PendingTabClose {
                tab_id: "last".into(),
            });
            root.replace_tab_rows(&rows[..2], cx);
            assert_eq!(root.tabs.active_tab_id(), Some("middle"));
            assert!(root.tab_landed_at.is_none());
            root
        });
    }

    #[cfg(unix)]
    #[test]
    fn sidebar_node_results_and_store_reload_share_the_pending_close_rule() {
        use gpui::TestAppContext;

        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let rows = tab_rows(["first", "second", "closing"]);
        let remaining = &rows[..2];
        for node_result_first in [false, true] {
            for activate_another in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                cx.add_window(|window, cx| {
                    let mut root = NativeRoot::new(
                        "close-ordering".into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    );
                    root.tabs = TabSet::from_rows(&rows);
                    root.tabs.activate("closing");
                    root.prepare_tab_close(&[], cx);
                    assert!(root.pending_tab_close.is_some());
                    if activate_another {
                        root.tabs.activate("second");
                    }
                    if node_result_first {
                        root.replace_tab_rows(remaining, cx);
                    }
                    store.update(cx, |store, cx| store.replace_nodes(remaining.to_vec(), cx));
                    root.apply_tab_rows(cx);
                    if !node_result_first {
                        root.replace_tab_rows(remaining, cx);
                    }
                    assert!(root.pending_tab_close.is_none());
                    let expected = if activate_another { "second" } else { "first" };
                    assert_eq!(root.tabs.active_tab_id(), Some(expected));
                    root.apply_tab_rows(cx);
                    assert_eq!(root.tabs.active_tab_id(), Some(expected));
                    root
                });
            }
        }
    }

    #[test]
    fn terminal_repaints_only_when_the_chat_surface_is_visible() {
        assert!(AppRoute::Chat.terminal_visible());
        for route in [
            AppRoute::Roles,
            AppRoute::RoleDetail("role".into()),
            AppRoute::Crews,
            AppRoute::CrewEditor("crew".into()),
            AppRoute::Mission("mission".into()),
            AppRoute::ArchivedChat,
            AppRoute::Settings,
        ] {
            assert!(!route.terminal_visible());
        }
    }

    #[test]
    fn collapse_transition_eases_from_open_to_closed() {
        let start = Instant::now();
        let duration = Duration::from_millis(200);
        let mut transition = SidebarVisibilityTransition::new(true);

        assert_eq!(transition.animate_to(0., start, duration), (1., true));
        assert_eq!(
            transition.animate_to(0., start + Duration::from_millis(100), duration),
            (0.5, true)
        );
        assert_eq!(
            transition.animate_to(0., start + Duration::from_millis(200), duration),
            (0., false)
        );
    }

    #[test]
    fn reversing_transition_continues_from_the_current_width() {
        let start = Instant::now();
        let duration = Duration::from_millis(200);
        let midpoint = start + Duration::from_millis(100);
        let mut transition = SidebarVisibilityTransition::new(true);

        transition.animate_to(0., start, duration);
        assert_eq!(transition.animate_to(1., midpoint, duration), (0.5, true));
        assert_eq!(
            transition.animate_to(1., midpoint + Duration::from_millis(100), duration),
            (0.75, true)
        );
    }
}
