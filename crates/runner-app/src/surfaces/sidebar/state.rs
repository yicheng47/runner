use super::menus::sidebar_create_menu_entries;

use super::*;
use crate::*;
use gpui::WeakEntity;

impl Sidebar {
    pub(crate) fn new(
        shell: WeakEntity<NativeRoot>,
        app_store: Entity<AppStore>,
        active_project_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let scroll = ScrollHandle::new();
        let scroll_owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), scroll_owner));
        let root = cx.entity();
        let create_menu = cx.new(move |menu_cx| {
            let action_root = root.clone();
            let entries = sidebar_create_menu_entries();
            let actions = entries
                .iter()
                .map(|(_, action)| action.clone())
                .collect::<Vec<_>>();
            PopoverMenu::new(
                "sidebar-create",
                menu_cx.focus_handle(),
                entries.into_iter().map(|(item, _)| item).collect(),
                Rc::new(move |index, window, cx| {
                    let Some(action) = actions.get(index).cloned() else {
                        return;
                    };
                    action_root.update(cx, |this, cx| {
                        this.handle_sidebar_menu_action(action, window, cx)
                    });
                }),
                menu_cx,
            )
            .min_width(px(160.))
            .trigger_size(IconButtonSize::Sm)
            .trigger_icon("plus.svg")
            .without_trigger_tooltip()
        });
        let store_revisions = app_store.read(cx).revisions;
        Self {
            shell,
            app_store: app_store.clone(),
            store_revisions,
            scroll,
            scrollbar,
            create_menu,
            context_menu: None,
            rename: None,
            live_titles: HashMap::new(),
            archiving_sessions: HashSet::new(),
            archiving_missions: HashSet::new(),
            active_project_id,
            window_id: 0,
            dragged_id: None,
            drop_target: None,
            drop_marker: None,
            cmd_held_since: None,
            shortcut_key_pressed: false,
            show_shortcut_pills: false,
            numbered_shortcut_rows: Vec::new(),
            tab_index_by_node: HashMap::new(),
            _rename_focus_subscription: None,
            _store_subscription: cx.observe(&app_store, |this, _, cx| {
                this.handle_store_update(cx);
            }),
        }
    }

    pub(super) fn core<'a>(&self, cx: &'a App) -> &'a AppCore {
        &self.app_store.read(cx).core
    }

    pub(super) fn settings<'a>(&self, cx: &'a App) -> &'a AppSettings {
        &self.app_store.read(cx).settings
    }

    pub(super) fn update_app_settings(
        &self,
        cx: &mut Context<Self>,
        persist: bool,
        update: impl FnOnce(&mut AppSettings) -> bool,
    ) -> bool {
        self.app_store.update(cx, |store, store_cx| {
            store.update_settings(update, persist, store_cx)
        })
    }

    pub(super) fn refresh_store(&self, refresh: StoreRefreshKind, cx: &mut Context<Self>) {
        self.app_store
            .update(cx, |store, store_cx| store.refresh(refresh, store_cx));
    }

    fn handle_store_update(&mut self, cx: &mut Context<Self>) {
        let revisions = self.app_store.read(cx).revisions;
        let previous = self.store_revisions;
        self.store_revisions = revisions;
        let titles_changed =
            self.live_titles_changed(revisions.terminal_wake != previous.terminal_wake, cx);
        if revisions.nodes != previous.nodes
            || revisions.projects != previous.projects
            || revisions.missions != previous.missions
            || revisions.sessions != previous.sessions
            || revisions.activity != previous.activity
            || revisions.settings != previous.settings
            || titles_changed
        {
            cx.notify();
        }
    }

    /// Whether any attached session is reporting different words than the rail
    /// last drew. A terminal wake fires on every burst of output, so the rail
    /// must not repaint on the wake itself — only when a title really changed
    /// (#587).
    fn live_titles_changed(&mut self, woke: bool, cx: &App) -> bool {
        if !woke {
            return false;
        }
        let Some(shell) = self.shell.upgrade() else {
            return false;
        };
        let titles = shell.read(cx).attached_titles(cx);
        if titles == self.live_titles {
            return false;
        }
        self.live_titles = titles;
        true
    }

    pub(super) fn report_error(&self, error: String, cx: &mut Context<Self>) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        cx.defer(move |cx| {
            shell.update(cx, |shell, shell_cx| {
                shell.error = Some(error);
                shell_cx.notify();
            });
        });
    }

    pub(super) fn schedule_shell_notify(&self, cx: &mut Context<Self>) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        cx.defer(move |cx| {
            shell.update(cx, |_, shell_cx| shell_cx.notify());
        });
    }

    pub(super) fn focus_shell_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, shell_cx| {
                shell.focus_active_terminal(window, shell_cx);
            });
        });
    }
}
