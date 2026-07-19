mod composer;
mod terminal_element;
mod theme;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use gpui::{
    actions, div, prelude::*, px, relative, size, AnyElement, App, Application, Bounds, Context,
    CursorStyle, DragMoveEvent, Entity, FocusHandle, KeyBinding, KeyDownEvent, Menu, MenuItem,
    MouseButton, ScrollDelta, ScrollWheelEvent, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use runner_app::model::{Runner, SessionStatus};
use runner_app::ops::session::DirectSessionEntry;
use runner_app::AppCore;
use runner_native::bootstrap::{
    boot_core, native_paths, stop_running_direct_sessions, NativePaths,
};
use runner_native::pane_layout::{
    PaneLayout, PaneLeaf, PaneNode, PresetKind, SplitOrientation, TabSet,
};
use runner_native::terminal::{TerminalBridge, TerminalSession};

use composer::Composer;
use terminal_element::TerminalElement;

actions!(runner_native_ui, [Quit, TermPaste, NewTab]);

const INITIAL_COLS: u16 = 100;
const INITIAL_ROWS: u16 = 30;
const SIDEBAR_WIDTH: f32 = 248.;
const WORKSPACE_HEADER_HEIGHT: f32 = 42.;
const PANE_HEADER_HEIGHT: f32 = 34.;
const COMPOSER_HEIGHT: f32 = 38.;

struct AttachedChat {
    terminal: Arc<TerminalSession>,
    composer: Entity<Composer>,
    terminal_focus: FocusHandle,
    scroll_accumulator: f32,
}

#[derive(Clone)]
enum NewChatTarget {
    NewTab,
    Pane { tab_id: String, pane_id: String },
}

#[derive(Clone)]
struct SplitResizeDrag {
    split_id: String,
    orientation: SplitOrientation,
}

impl Render for SplitResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}

struct NativeRoot {
    core: AppCore,
    bridge: Arc<TerminalBridge>,
    sessions: Vec<DirectSessionEntry>,
    runners: Vec<Runner>,
    tabs: TabSet,
    attached: HashMap<String, AttachedChat>,
    root_focus: FocusHandle,
    waker: Arc<dyn Fn() + Send + Sync>,
    new_chat_target: Option<NewChatTarget>,
    layout_picker_open: bool,
    split_sizes_dirty: bool,
    error: Option<String>,
}

impl NativeRoot {
    fn new(core: AppCore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (wake_tx, mut wake_rx) = futures::channel::mpsc::unbounded::<()>();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake_tx.unbounded_send(());
        });
        let bridge =
            TerminalBridge::new(core.clone(), Arc::clone(&waker)).expect("start event bridge");

        cx.spawn(async move |weak, cx| {
            while wake_rx.next().await.is_some() {
                while wake_rx.try_recv().is_ok() {}
                if weak.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut errors = Vec::new();
        let sessions = match runner_app::ops::session::session_list_recent_direct(&core) {
            Ok(sessions) => sessions,
            Err(error) => {
                errors.push(error.to_string());
                Vec::new()
            }
        };
        let runners = match runner_app::ops::runner::runner_list(&core) {
            Ok(runners) => runners,
            Err(error) => {
                errors.push(error.to_string());
                Vec::new()
            }
        };
        let tabs = match runner_app::ops::tab::tab_list(&core)
            .map_err(anyhow::Error::from)
            .and_then(|rows| TabSet::from_rows(&rows))
        {
            Ok(tabs) => tabs,
            Err(error) => {
                errors.push(error.to_string());
                TabSet::default()
            }
        };

        let root_focus = cx.focus_handle();
        let mut root = Self {
            core,
            bridge,
            sessions,
            runners,
            tabs,
            attached: HashMap::new(),
            root_focus,
            waker,
            new_chat_target: None,
            layout_picker_open: false,
            split_sizes_dirty: false,
            error: (!errors.is_empty()).then(|| errors.join("\n")),
        };
        if let Err(error) = root.ensure_active_tab_attached(window, cx) {
            root.error = Some(error.to_string());
        }
        root.focus_active_terminal(window);
        root
    }

    fn refresh_sessions(&mut self) {
        match runner_app::ops::session::session_list_recent_direct(&self.core) {
            Ok(sessions) => self.sessions = sessions,
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn reload_tabs(&mut self) -> Result<()> {
        let rows = runner_app::ops::tab::tab_list(&self.core)?;
        self.tabs.replace_rows(&rows)
    }

    fn session_entry(&self, session_id: &str) -> Option<&DirectSessionEntry> {
        self.sessions
            .iter()
            .find(|entry| entry.session_id == session_id)
    }

    fn ensure_active_tab_attached(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let Some(layout) = self.tabs.active().cloned() else {
            return Ok(());
        };
        let mut errors = Vec::new();
        for session_id in layout.session_ids() {
            if let Err(error) = self.ensure_attached(&layout, &session_id, window, cx) {
                errors.push(error.to_string());
            }
        }
        self.refresh_sessions();
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("\n"))
        }
    }

    fn ensure_attached(
        &mut self,
        layout: &PaneLayout,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let _entry = runner_app::ops::session::session_get(&self.core, session_id)?
            .with_context(|| format!("direct chat not found: {session_id}"))?;
        let pane_id = layout
            .root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.session_id.as_deref() == Some(session_id))
            .map(|leaf| leaf.id.as_str());
        let estimated = pane_id
            .map(|pane_id| self.estimated_terminal_size(layout, pane_id, window))
            .unwrap_or((INITIAL_COLS, INITIAL_ROWS));
        let size = self
            .attached
            .get(session_id)
            .map(|chat| chat.terminal.size())
            .unwrap_or(estimated);

        if self.attached.contains_key(session_id) {
            return Ok(());
        }

        let terminal = TerminalSession::attach(
            self.core.clone(),
            session_id.to_owned(),
            size.0,
            size.1,
            Arc::clone(&self.waker),
        )?;
        self.bridge.attach(Arc::clone(&terminal))?;
        let terminal_focus = cx.focus_handle();
        let composer_focus = cx.focus_handle();
        let composer = cx.new(|_| Composer::new(composer_focus, Arc::clone(&terminal)));
        self.attached.insert(
            session_id.to_owned(),
            AttachedChat {
                terminal,
                composer,
                terminal_focus,
                scroll_accumulator: 0.,
            },
        );
        Ok(())
    }

    fn estimated_terminal_size(
        &self,
        layout: &PaneLayout,
        pane_id: &str,
        window: &Window,
    ) -> (u16, u16) {
        let (width_fraction, height_fraction) =
            pane_fractions(&layout.root, pane_id).unwrap_or((1., 1.));
        let bounds = window.bounds().size;
        let pane_width = (f32::from(bounds.width) - SIDEBAR_WIDTH).max(200.) * width_fraction;
        let grouped = layout.root.leaves().len() > 1;
        let pane_height = (f32::from(bounds.height) - WORKSPACE_HEADER_HEIGHT).max(160.)
            * height_fraction
            - COMPOSER_HEIGHT
            - if grouped { PANE_HEADER_HEIGHT } else { 0. };
        let cell_width = terminal_element::FONT_SIZE * 0.6;
        let line_height =
            (terminal_element::FONT_SIZE * terminal_element::LINE_HEIGHT_FACTOR).round();
        (
            (pane_width / cell_width).floor().max(2.) as u16,
            (pane_height / line_height).floor().max(2.) as u16,
        )
    }

    fn active_focused_session_id(&self) -> Option<String> {
        self.tabs
            .active()
            .and_then(PaneLayout::focused_session_id)
            .map(str::to_owned)
    }

    fn focus_active_terminal(&self, window: &mut Window) {
        let Some(session_id) = self.active_focused_session_id() else {
            self.root_focus.focus(window);
            return;
        };
        if let Some(chat) = self.attached.get(&session_id) {
            chat.terminal_focus.focus(window);
        } else {
            self.root_focus.focus(window);
        }
    }

    fn activate_tab(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.activate(tab_id) {
            return;
        }
        self.new_chat_target = None;
        self.layout_picker_open = false;
        match self.ensure_active_tab_attached(window, cx) {
            Ok(()) => {
                self.error = None;
                self.focus_active_terminal(window);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn focus_pane(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        if self
            .tabs
            .active_mut()
            .is_some_and(|layout| layout.focus_pane(pane_id))
        {
            cx.notify();
        }
    }

    fn focus_terminal(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane(pane_id, cx);
        if let Some(chat) = self.attached.get(session_id) {
            chat.terminal_focus.focus(window);
        }
    }

    fn on_key_down(
        &mut self,
        session_id: &str,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.platform {
            return;
        }
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        let keystroke = &event.keystroke;
        match chat.terminal.send_key(
            &keystroke.key,
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            keystroke.key_char.as_deref(),
        ) {
            Ok(true) => {
                chat.terminal.scroll_to_bottom();
                self.error = None;
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn on_scroll(
        &mut self,
        session_id: &str,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.attached.get_mut(session_id) else {
            return;
        };
        let lines = match event.delta {
            ScrollDelta::Lines(point) => point.y,
            ScrollDelta::Pixels(point) => f32::from(point.y) / f32::from(window.line_height()),
        };
        chat.scroll_accumulator += lines;
        let whole = chat.scroll_accumulator.trunc() as i32;
        if whole != 0 {
            chat.scroll_accumulator -= whole as f32;
            chat.terminal.scroll(whole);
            cx.notify();
        }
    }

    fn on_paste(
        &mut self,
        session_id: &str,
        _: &TermPaste,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if let Err(error) = chat.terminal.paste(&text) {
            self.error = Some(error.to_string());
        }
    }

    fn begin_new_tab(&mut self, _: &NewTab, _: &mut Window, cx: &mut Context<Self>) {
        self.new_chat_target = Some(NewChatTarget::NewTab);
        self.layout_picker_open = false;
        cx.notify();
    }

    fn begin_pane_chat(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        let Some(tab_id) = self.tabs.active_tab_id().map(str::to_owned) else {
            return;
        };
        self.new_chat_target = Some(NewChatTarget::Pane {
            tab_id,
            pane_id: pane_id.to_owned(),
        });
        cx.notify();
    }

    fn start_chat(&mut self, runner_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.new_chat_target.clone() else {
            return;
        };
        let initial_size = match &target {
            NewChatTarget::NewTab => (INITIAL_COLS, INITIAL_ROWS),
            NewChatTarget::Pane { tab_id, pane_id }
                if self.tabs.active_tab_id() == Some(tab_id.as_str()) =>
            {
                self.tabs
                    .active()
                    .map(|layout| self.estimated_terminal_size(layout, pane_id, window))
                    .unwrap_or((INITIAL_COLS, INITIAL_ROWS))
            }
            NewChatTarget::Pane { .. } => {
                self.error = Some("The target tab is no longer active".into());
                return;
            }
        };
        let mut spawned_id = None;
        let result = (|| -> Result<String> {
            let spawned = runner_app::ops::session::session_start_direct(
                &self.core,
                runner_id.to_owned(),
                None,
                None,
                None,
                Some(initial_size.0),
                Some(initial_size.1),
            )?;
            spawned_id = Some(spawned.id.clone());
            self.refresh_sessions();
            match target {
                NewChatTarget::NewTab => {
                    self.reload_tabs()?;
                    self.tabs.activate_session(&spawned.id);
                }
                NewChatTarget::Pane { pane_id, .. } => {
                    self.tabs.assign_to_active(&pane_id, &spawned.id)?;
                    self.persist_active_tab()?;
                    self.reload_tabs()?;
                    self.tabs.activate_session(&spawned.id);
                }
            }
            self.new_chat_target = None;
            self.ensure_active_tab_attached(window, cx)?;
            Ok(spawned.id)
        })();
        match result {
            Ok(session_id) => {
                self.error = None;
                if let Some(chat) = self.attached.get(&session_id) {
                    chat.terminal_focus.focus(window);
                }
            }
            Err(error) => {
                if let Some(session_id) = spawned_id {
                    self.new_chat_target = None;
                    let _ = self.reload_tabs();
                    self.tabs.activate_session(&session_id);
                    let _ = self.ensure_active_tab_attached(window, cx);
                }
                self.error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    fn resume_chat(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = (|| -> Result<()> {
            let layout = self.tabs.active().cloned().context("no active tab")?;
            let size = self
                .attached
                .get(session_id)
                .map(|chat| chat.terminal.size())
                .unwrap_or_else(|| self.estimated_terminal_size(&layout, pane_id, window));
            runner_app::ops::session::session_resume(
                &self.core,
                session_id,
                Some(size.0),
                Some(size.1),
            )?;
            self.ensure_attached(&layout, session_id, window, cx)?;
            self.refresh_sessions();
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.error = None;
                self.focus_terminal(pane_id, session_id, window, cx);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn pick_preset(&mut self, preset: PresetKind, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> Result<Option<String>> {
            let Some(layout) = self.tabs.active_mut() else {
                return Ok(None);
            };
            layout.apply_preset(preset);
            let empty_pane_id = layout
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.session_id.is_none())
                .map(|leaf| leaf.id.clone());
            self.persist_active_tab()?;
            self.reload_tabs()?;
            self.ensure_active_tab_attached(window, cx)?;
            Ok(empty_pane_id)
        })();
        self.layout_picker_open = false;
        match result {
            Ok(empty_pane_id) => {
                self.error = None;
                if let Some(pane_id) = empty_pane_id {
                    self.begin_pane_chat(&pane_id, cx);
                } else {
                    self.focus_active_terminal(window);
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn persist_active_tab(&self) -> Result<()> {
        let input = self
            .tabs
            .active()
            .context("active tab is missing")?
            .upsert_input()?;
        runner_app::ops::tab::tab_upsert(&self.core, input)?;
        Ok(())
    }

    fn resize_split(
        &mut self,
        split_id: &str,
        orientation: SplitOrientation,
        event: &DragMoveEvent<SplitResizeDrag>,
        cx: &mut Context<Self>,
    ) {
        let position = event.event.position;
        let bounds = event.bounds;
        let ratio = match orientation {
            SplitOrientation::Row => {
                f32::from(position.x - bounds.left()) / f32::from(bounds.size.width)
            }
            SplitOrientation::Column => {
                f32::from(position.y - bounds.top()) / f32::from(bounds.size.height)
            }
        }
        .clamp(0.15, 0.85)
            * 100.;
        if self
            .tabs
            .active_mut()
            .is_some_and(|layout| layout.set_split_sizes(split_id, [ratio, 100. - ratio]))
        {
            self.split_sizes_dirty = true;
            cx.notify();
        }
    }

    fn finish_split_resize(&mut self) {
        if !self.split_sizes_dirty {
            return;
        }
        self.split_sizes_dirty = false;
        if let Err(error) = self.persist_active_tab() {
            self.error = Some(error.to_string());
        }
    }

    fn tab_label(&self, layout: &PaneLayout) -> String {
        if let Some(name) = &layout.name {
            return name.clone();
        }
        let labels = layout
            .session_ids()
            .into_iter()
            .filter_map(|session_id| self.session_entry(&session_id))
            .map(session_label)
            .collect::<Vec<_>>();
        if labels.is_empty() {
            "Empty tab".into()
        } else {
            labels.join(" + ")
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let active_tab_id = self.tabs.active_tab_id().map(str::to_owned);
        let tab_rows = self
            .tabs
            .tabs()
            .iter()
            .map(|layout| {
                (
                    layout.id.clone(),
                    self.tab_label(layout),
                    layout.root.leaves().len(),
                    layout.session_ids().iter().any(|session_id| {
                        self.session_entry(session_id)
                            .is_some_and(|entry| entry.status == SessionStatus::Running)
                    }),
                )
            })
            .collect::<Vec<_>>();

        let list: AnyElement = if let Some(target) = self.new_chat_target.as_ref() {
            let heading = match target {
                NewChatTarget::NewTab => "NEW TAB",
                NewChatTarget::Pane { .. } => "NEW CHAT IN PANE",
            };
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_4()
                        .pb_2()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_xs().text_color(theme::muted()).child(heading))
                        .child(
                            div()
                                .id("cancel-new-chat")
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .cursor_pointer()
                                .text_xs()
                                .text_color(theme::muted())
                                .hover(|button| button.bg(theme::border()))
                                .child("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.new_chat_target = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .id("new-chat-runner-list")
                        .flex_1()
                        .overflow_y_scroll()
                        .px_2()
                        .children(self.runners.iter().enumerate().map(|(index, runner)| {
                            let runner_id = runner.id.clone();
                            div()
                                .id(("new-chat-runner", index))
                                .w_full()
                                .mb_1()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|row| row.bg(theme::border()))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme::text())
                                        .child(format!("@{}", runner.handle)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::muted())
                                        .child(runner.display_name.clone()),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start_chat(&runner_id, window, cx);
                                }))
                        })),
                )
                .into_any_element()
        } else {
            div()
                .id("tab-list")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .px_2()
                .children(tab_rows.into_iter().enumerate().map(
                    |(index, (tab_id, label, pane_count, running))| {
                        let selected = active_tab_id.as_deref() == Some(&tab_id);
                        let click_id = tab_id.clone();
                        div()
                            .id(("direct-tab", index))
                            .w_full()
                            .mb_1()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .cursor_pointer()
                            .when(selected, |row| row.bg(theme::border()))
                            .hover(|row| row.bg(theme::border()))
                            .child(div().text_sm().text_color(theme::text()).child(label))
                            .child(div().text_xs().text_color(theme::muted()).child(format!(
                                "{} · {pane_count} {}",
                                if running { "running" } else { "stopped" },
                                if pane_count == 1 { "pane" } else { "panes" }
                            )))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_tab(&click_id, window, cx);
                            }))
                    },
                ))
                .into_any_element()
        };

        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme::composer_bg())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .px_4()
                    .pt_10()
                    .pb_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::muted())
                            .child("DIRECT CHAT TABS"),
                    )
                    .child(
                        div()
                            .id("new-tab-sidebar")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(theme::accent())
                            .hover(|button| button.bg(theme::border()))
                            .child("+ New")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.begin_new_tab(&NewTab, window, cx);
                            })),
                    ),
            )
            .child(list)
            .into_any_element()
    }

    fn render_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(layout) = self.tabs.active().cloned() else {
            return div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme::muted())
                .child(if self.sessions.is_empty() {
                    "No direct chats yet — press ⌘T"
                } else {
                    "No active tab"
                })
                .into_any_element();
        };
        let label = self.tab_label(&layout);
        let preset = layout.preset;
        let pane_tree = self.render_pane_node(&layout.root, &layout, window, cx);
        let picker = self
            .layout_picker_open
            .then(|| self.render_layout_picker(preset, cx));

        div()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .h(px(WORKSPACE_HEADER_HEIGHT))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .min_w(px(0.))
                            .text_sm()
                            .text_color(theme::text())
                            .child(label),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("layout-picker-toggle")
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(if self.layout_picker_open {
                                        theme::accent()
                                    } else {
                                        theme::border()
                                    })
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(theme::text())
                                    .hover(|button| button.bg(theme::border()))
                                    .child("Layout")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.layout_picker_open = !this.layout_picker_open;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("new-tab-header")
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .hover(|button| button.bg(theme::border()))
                                    .child("New tab  ⌘T")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.begin_new_tab(&NewTab, window, cx);
                                    })),
                            ),
                    ),
            )
            .child(div().flex_1().min_h(px(0.)).p_1().child(pane_tree))
            .children(picker)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.finish_split_resize()),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.finish_split_resize()),
            )
            .into_any_element()
    }

    fn render_layout_picker(&self, active: PresetKind, cx: &mut Context<Self>) -> AnyElement {
        div()
            .absolute()
            .top(px(WORKSPACE_HEADER_HEIGHT + 4.))
            .right(px(12.))
            .w(px(244.))
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::composer_bg())
            .shadow_lg()
            .child(
                div()
                    .pb_2()
                    .text_xs()
                    .text_color(theme::muted())
                    .child("LAYOUT"),
            )
            .child(
                div().flex().flex_wrap().gap_2().children(
                    PresetKind::ALL
                        .into_iter()
                        .enumerate()
                        .map(|(index, preset)| {
                            div()
                                .id(("layout-preset", index))
                                .w(px(104.))
                                .px_2()
                                .py_2()
                                .rounded_md()
                                .border_1()
                                .border_color(if preset == active {
                                    theme::accent()
                                } else {
                                    theme::border()
                                })
                                .cursor_pointer()
                                .text_xs()
                                .text_color(if preset == active {
                                    theme::accent()
                                } else {
                                    theme::text()
                                })
                                .hover(|tile| tile.bg(theme::border()))
                                .child(preset.label())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.pick_preset(preset, window, cx);
                                }))
                        }),
                ),
            )
            .child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(theme::muted())
                    .child("Drag pane dividers to resize"),
            )
            .into_any_element()
    }

    fn render_pane_node(
        &mut self,
        node: &PaneNode,
        layout: &PaneLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            PaneNode::Leaf(leaf) => self.render_pane(leaf, layout, window, cx),
            PaneNode::Split(split) => {
                let a = self.render_pane_node(&split.a, layout, window, cx);
                let b = self.render_pane_node(&split.b, layout, window, cx);
                let first = split.sizes[0] / 100.;
                let second = split.sizes[1] / 100.;
                let split_id = split.id.clone();
                let orientation = split.orientation;
                let drag = SplitResizeDrag {
                    split_id: split.id.clone(),
                    orientation,
                };
                let a_wrap = div()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .when(orientation == SplitOrientation::Row, |pane| {
                        pane.w(relative(first)).h_full()
                    })
                    .when(orientation == SplitOrientation::Column, |pane| {
                        pane.h(relative(first)).w_full()
                    })
                    .child(a);
                let b_wrap = div()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .when(orientation == SplitOrientation::Row, |pane| {
                        pane.w(relative(second)).h_full()
                    })
                    .when(orientation == SplitOrientation::Column, |pane| {
                        pane.h(relative(second)).w_full()
                    })
                    .child(b);
                let gutter = div()
                    .id(SharedString::from(format!("gutter-{}", split.id)))
                    .flex_none()
                    .when(orientation == SplitOrientation::Row, |gutter| {
                        gutter
                            .w(px(5.))
                            .h_full()
                            .cursor(CursorStyle::ResizeLeftRight)
                    })
                    .when(orientation == SplitOrientation::Column, |gutter| {
                        gutter.h(px(5.)).w_full().cursor(CursorStyle::ResizeUpDown)
                    })
                    .bg(theme::border())
                    .on_drag(drag, |drag: &SplitResizeDrag, _, _, cx: &mut App| {
                        cx.new(|_| drag.clone())
                    });
                div()
                    .id(SharedString::from(format!("split-{}", split.id)))
                    .size_full()
                    .flex()
                    .when(orientation == SplitOrientation::Column, |container| {
                        container.flex_col()
                    })
                    .child(a_wrap)
                    .child(gutter)
                    .child(b_wrap)
                    .on_drag_move::<SplitResizeDrag>(cx.listener(
                        move |this, event: &DragMoveEvent<SplitResizeDrag>, _, cx| {
                            let drag = event.drag(cx);
                            if drag.split_id == split_id {
                                this.resize_split(&split_id, drag.orientation, event, cx);
                            }
                        },
                    ))
                    .on_drop(cx.listener(|this, _: &SplitResizeDrag, _, _| {
                        this.finish_split_resize();
                    }))
                    .into_any_element()
            }
        }
    }

    fn render_pane(
        &mut self,
        leaf: &PaneLeaf,
        layout: &PaneLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focused = layout.focused_pane_id == leaf.id;
        let grouped = layout.root.leaves().len() > 1;
        let pane_id = leaf.id.clone();
        let pane_id_for_focus = pane_id.clone();
        let header = grouped.then(|| {
            let label = leaf
                .session_id
                .as_deref()
                .and_then(|session_id| self.session_entry(session_id))
                .map(session_label)
                .unwrap_or_else(|| "Empty pane".into());
            div()
                .flex_none()
                .h(px(PANE_HEADER_HEIGHT))
                .px_3()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(theme::border())
                .bg(theme::composer_bg())
                .child(
                    div()
                        .mr_2()
                        .text_xs()
                        .text_color(if focused {
                            theme::accent()
                        } else {
                            theme::muted()
                        })
                        .child("●"),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .text_sm()
                        .text_color(theme::text())
                        .child(label),
                )
        });

        let body: AnyElement = if let Some(session_id) = leaf.session_id.as_deref() {
            let status = self.session_entry(session_id).map(|entry| entry.status);
            if let Some(chat) = self.attached.get(session_id) {
                let terminal = Arc::clone(&chat.terminal);
                let composer = chat.composer.clone();
                let terminal_focus = chat.terminal_focus.clone();
                let terminal_focused = terminal_focus.is_focused(window);
                let key_session_id = session_id.to_owned();
                let scroll_session_id = session_id.to_owned();
                let paste_session_id = session_id.to_owned();
                let click_session_id = session_id.to_owned();
                let click_pane_id = pane_id.clone();
                let content = div().flex_1().min_h(px(0.)).flex().flex_col().child(
                    div()
                        .id(SharedString::from(format!("terminal-{session_id}")))
                        .key_context("Terminal")
                        .track_focus(&terminal_focus)
                        .flex_1()
                        .min_h(px(0.))
                        .p_2()
                        .on_key_down(cx.listener(move |this, event, window, cx| {
                            this.on_key_down(&key_session_id, event, window, cx);
                        }))
                        .on_scroll_wheel(cx.listener(move |this, event, window, cx| {
                            this.on_scroll(&scroll_session_id, event, window, cx);
                        }))
                        .on_action(cx.listener(move |this, action, window, cx| {
                            this.on_paste(&paste_session_id, action, window, cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.focus_terminal(&click_pane_id, &click_session_id, window, cx);
                            }),
                        )
                        .child(TerminalElement::new(terminal, terminal_focused)),
                );
                if status == Some(SessionStatus::Running) {
                    content.child(composer).into_any_element()
                } else {
                    let resume_pane_id = pane_id.clone();
                    let resume_session_id = session_id.to_owned();
                    let status_label = match status {
                        Some(SessionStatus::Crashed) => "Chat crashed",
                        _ => "Chat stopped",
                    };
                    content
                        .child(
                            div()
                                .flex_none()
                                .px_3()
                                .py_2()
                                .flex()
                                .items_center()
                                .justify_between()
                                .border_t_1()
                                .border_color(theme::border())
                                .bg(theme::composer_bg())
                                .text_sm()
                                .text_color(theme::muted())
                                .child(status_label)
                                .child(
                                    div()
                                        .id(SharedString::from(format!("resume-chat-{session_id}")))
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(theme::border())
                                        .cursor_pointer()
                                        .text_sm()
                                        .text_color(theme::accent())
                                        .hover(|button| button.bg(theme::border()))
                                        .child("Resume")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.resume_chat(
                                                &resume_pane_id,
                                                &resume_session_id,
                                                window,
                                                cx,
                                            );
                                        })),
                                ),
                        )
                        .into_any_element()
                }
            } else if status != Some(SessionStatus::Running) {
                let resume_pane_id = pane_id.clone();
                let resume_session_id = session_id.to_owned();
                let status_label = match status {
                    Some(SessionStatus::Crashed) => "Chat crashed",
                    _ => "Chat stopped",
                };
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme::muted())
                    .child(status_label)
                    .child(
                        div()
                            .id(SharedString::from(format!("resume-chat-{session_id}")))
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::border())
                            .cursor_pointer()
                            .text_sm()
                            .text_color(theme::accent())
                            .hover(|button| button.bg(theme::border()))
                            .child("Resume")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.resume_chat(&resume_pane_id, &resume_session_id, window, cx);
                            })),
                    )
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme::muted())
                    .child("Unable to attach chat")
                    .into_any_element()
            }
        } else {
            let new_chat_pane_id = pane_id.clone();
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id(SharedString::from(format!("new-chat-{pane_id}")))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .border_1()
                        .border_color(theme::border())
                        .cursor_pointer()
                        .text_sm()
                        .text_color(theme::accent())
                        .hover(|button| button.bg(theme::border()))
                        .child("New chat")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.begin_pane_chat(&new_chat_pane_id, cx);
                        })),
                )
                .into_any_element()
        };

        div()
            .id(SharedString::from(format!("pane-{pane_id}")))
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .border_1()
            .border_color(if focused {
                theme::accent()
            } else {
                theme::border()
            })
            .children(header)
            .child(body)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.focus_pane(&pane_id_for_focus, cx);
                }),
            )
            .into_any_element()
    }
}

impl Render for NativeRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.bridge.take_session_refresh() {
            self.refresh_sessions();
        }

        let sidebar = self.render_sidebar(cx);
        let workspace = self.render_active_tab(window, cx);
        div()
            .size_full()
            .flex()
            .track_focus(&self.root_focus)
            .bg(theme::bg())
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(workspace)
                    .children(self.error.as_ref().map(|error| {
                        div()
                            .flex_none()
                            .px_3()
                            .py_2()
                            .bg(gpui::rgb(0x3b1d2b))
                            .text_sm()
                            .text_color(gpui::rgb(0xf7768e))
                            .child(SharedString::from(error.clone()))
                    })),
            )
            .on_action(cx.listener(Self::begin_new_tab))
    }
}

fn pane_fractions(node: &PaneNode, pane_id: &str) -> Option<(f32, f32)> {
    match node {
        PaneNode::Leaf(leaf) => (leaf.id == pane_id).then_some((1., 1.)),
        PaneNode::Split(split) => {
            let orientation = split.orientation;
            let first = split.sizes[0] / 100.;
            let second = split.sizes[1] / 100.;
            pane_fractions(&split.a, pane_id)
                .map(|(width, height)| match orientation {
                    SplitOrientation::Row => (width * first, height),
                    SplitOrientation::Column => (width, height * first),
                })
                .or_else(|| {
                    pane_fractions(&split.b, pane_id).map(|(width, height)| match orientation {
                        SplitOrientation::Row => (width * second, height),
                        SplitOrientation::Column => (width, height * second),
                    })
                })
        }
    }
}

fn session_label(entry: &DirectSessionEntry) -> String {
    entry.title.clone().unwrap_or_else(|| {
        entry
            .handle
            .as_ref()
            .map(|handle| format!("@{handle}"))
            .unwrap_or_else(|| entry.display_name.clone())
    })
}

fn run() -> Result<()> {
    let paths = native_paths()?;
    let core = boot_core(&paths)?;
    print_startup_paths(&paths);
    let shutdown_core = core.clone();

    Application::new().run(move |cx: &mut App| {
        let quit_core = core.clone();
        cx.on_action(move |_: &Quit, cx| {
            let _ = stop_running_direct_sessions(&quit_core);
            cx.quit();
        });
        let close_core = core.clone();
        cx.on_window_closed(move |cx| {
            let _ = stop_running_direct_sessions(&close_core);
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-t", NewTab, None),
            KeyBinding::new("cmd-v", TermPaste, Some("Terminal")),
            KeyBinding::new("backspace", composer::Backspace, Some("Composer")),
            KeyBinding::new("delete", composer::Delete, Some("Composer")),
            KeyBinding::new("left", composer::Left, Some("Composer")),
            KeyBinding::new("right", composer::Right, Some("Composer")),
            KeyBinding::new("shift-left", composer::SelectLeft, Some("Composer")),
            KeyBinding::new("shift-right", composer::SelectRight, Some("Composer")),
            KeyBinding::new("cmd-a", composer::SelectAll, Some("Composer")),
            KeyBinding::new("home", composer::Home, Some("Composer")),
            KeyBinding::new("end", composer::End, Some("Composer")),
            KeyBinding::new("cmd-v", composer::Paste, Some("Composer")),
            KeyBinding::new("enter", composer::Submit, Some("Composer")),
            KeyBinding::new(
                "ctrl-cmd-space",
                composer::ShowCharacterPalette,
                Some("Composer"),
            ),
        ]);
        cx.set_menus(vec![Menu {
            name: "Runner Native".into(),
            items: vec![
                MenuItem::action("New Chat", NewTab),
                MenuItem::action("Quit", Quit),
            ],
        }]);

        let bounds = Bounds::centered(None, size(px(1200.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Runner Native".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| NativeRoot::new(core.clone(), window, cx)),
        )
        .expect("open Runner Native window");
        cx.activate(true);
    });

    stop_running_direct_sessions(&shutdown_core)?;
    Ok(())
}

fn print_startup_paths(paths: &NativePaths) {
    eprintln!(
        "Runner Native: database={} logs={}",
        paths.app_data_dir.join("runner.db").display(),
        paths.log_dir.display()
    );
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Runner Native failed: {error:#}");
        std::process::exit(1);
    }
}
