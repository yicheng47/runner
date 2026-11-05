//! Find in terminal: each terminal view's floating find bar and the matches
//! it highlights. Searching runs in the app against the session's mirror
//! `Term`; the view owns the state because one session can be shown in more
//! than one place.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::Line;
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::{Term, TermMode};
use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, App, BoxShadow, Context, Entity, FocusHandle, FontWeight,
    KeyDownEvent, SharedString, Subscription, Task, WeakEntity, Window,
};

use runner_app::theme;
use runner_app::ui::{TextField, Tooltip};
use runner_terminal::terminal::find::{
    self as search, FindCount, FindPattern, FindQuery, ScrollMark,
};
use runner_terminal::terminal::{EventProxy, TerminalMirror, ViewportHold};

use crate::FindInTerminal;

/// Output recomputes the matches at most this often.
const REFRESH_INTERVAL: Duration = Duration::from_millis(120);

/// The narrowest the query field gets, in rem-scaled pixels: small enough
/// that the longest count, "1000+ of 1000+", still fits one line at the
/// bar's full width.
const QUERY_MIN_WIDTH: f32 = 80.;

/// The search icon, the field and the two toggles, with their gaps.
const QUERY_GROUP_MIN_WIDTH: f32 = 13. + 6. + QUERY_MIN_WIDTH + 6. + 22. + 6. + 22.;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Status {
    #[default]
    Empty,
    Invalid,
    Ready,
}

/// What the terminal paints for find: every visible match, oldest first,
/// and the current one.
pub(crate) struct FindHighlights {
    pub(crate) matches: Vec<Match>,
    pub(crate) current: Option<Match>,
}

pub(crate) struct TerminalFind {
    terminal: Arc<TerminalMirror>,
    terminal_focus: FocusHandle,
    field: Entity<TextField>,
    open: bool,
    query: String,
    match_case: bool,
    regex: bool,
    status: Status,
    /// The last pattern that built: an invalid one keeps its matches.
    search: Option<Box<RegexSearch>>,
    /// The current match, at the positions `mark` describes.
    current: Option<Match>,
    mark: ScrollMark,
    count: FindCount,
    hold: Option<ViewportHold>,
    alt_screen: bool,
    /// The viewport from before find opened, restored on close.
    opened_at: Option<(usize, ScrollMark)>,
    searched_at: Option<Instant>,
    refresh: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl TerminalFind {
    pub(crate) fn new(
        terminal: Arc<TerminalMirror>,
        terminal_focus: FocusHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let find = cx.entity().downgrade();
        let field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("terminal.find_placeholder"),
                false,
            )
            .text_size(theme::text_body())
            .key_interceptor(Rc::new(move |event, window, cx| {
                find.upgrade().is_some_and(|find| {
                    find.update(cx, |find, find_cx| find.on_key_down(event, window, find_cx))
                })
            }));
            field.set_bare(true, field_cx);
            field.set_right_padding(0., field_cx);
            field
        });
        let mut subscriptions = vec![cx.observe(&field, |this, field, cx| {
            let query = field.read(cx).text();
            if query != this.query {
                this.query = query.to_owned();
                this.search_query(cx);
            }
        })];
        if let Some(updates) = cx
            .try_global::<crate::app_store::GlobalAppStore>()
            .map(|store| store.0.read(cx).terminal_updates.clone())
        {
            subscriptions.push(cx.subscribe(&updates, |this, _, id: &String, cx| {
                if this.open && id == this.terminal.session_id() {
                    this.on_output(cx);
                }
            }));
        }
        Self {
            terminal,
            terminal_focus,
            field,
            open: false,
            query: String::new(),
            match_case: false,
            regex: false,
            status: Status::Empty,
            search: None,
            current: None,
            mark: ScrollMark::default(),
            count: FindCount::default(),
            hold: None,
            alt_screen: false,
            opened_at: None,
            searched_at: None,
            refresh: None,
            _subscriptions: subscriptions,
        }
    }

    /// Opens the bar with any terminal selection as its query, or with the
    /// bar already open selects the query.
    pub(crate) fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let seed = (!self.open).then(|| {
            self.terminal
                .selection_text()
                .and_then(|text| text.lines().next().map(str::to_owned))
                .filter(|line| !line.trim().is_empty())
                .unwrap_or_default()
        });
        if seed.is_some() {
            self.open = true;
            self.opened_at = Some((
                self.terminal.scroll_state().display_offset,
                self.terminal.scroll_mark(),
            ));
            self.alt_screen = self.terminal.mode().contains(TermMode::ALT_SCREEN);
        }
        self.field.update(cx, |field, field_cx| {
            if let Some(seed) = seed {
                field.set_text(seed, field_cx);
            }
            field.select_all(field_cx);
            field.focus_handle().focus(window, field_cx);
        });
        cx.notify();
    }

    /// Closes the bar, clears its matches and highlights, puts the viewport
    /// back where it was before find opened and returns focus to the terminal.
    /// The field is emptied when the bar next opens: Esc arrives while the
    /// field is handling its own key.
    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.discard();
        self.terminal_focus.focus(window, cx);
        cx.notify();
    }

    /// Closes find for a view the window is leaving, without moving focus.
    pub(crate) fn leave(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.discard();
            cx.notify();
        }
    }

    fn discard(&mut self) {
        self.open = false;
        self.query.clear();
        self.clear_matches();
        self.status = Status::Empty;
        self.search = None;
        self.refresh = None;
        self.restore_viewport();
    }

    /// Puts the viewport back where it was before find opened: at the bottom,
    /// or on the same content when it was scrolled back.
    fn restore_viewport(&mut self) {
        let Some((offset, mark)) = self.opened_at.take() else {
            return;
        };
        let moved = mark
            .moved_since(self.terminal.scroll_mark())
            .unwrap_or_default();
        let offset = if offset == 0 {
            0
        } else {
            offset + moved.max(0) as usize
        };
        self.terminal.scroll_to_display_offset(offset);
    }

    fn clear_matches(&mut self) {
        self.count = FindCount::default();
        self.set_current(None);
    }

    fn set_current(&mut self, current: Option<Match>) {
        match (&current, &self.hold) {
            (Some(_), None) => self.hold = Some(self.terminal.hold_viewport()),
            (None, Some(_)) => self.hold = None,
            _ => {}
        }
        self.current = current;
    }

    /// The current match carried to the terminal's latest positions.
    fn current_now(&self) -> Option<Match> {
        carry(
            self.current.as_ref(),
            self.mark,
            self.terminal.scroll_mark(),
        )
    }

    fn query(&self) -> FindQuery {
        FindQuery {
            text: self.query.clone(),
            match_case: self.match_case,
            regex: self.regex,
        }
    }

    fn search_query(&mut self, cx: &mut Context<Self>) {
        match self.query().pattern() {
            FindPattern::Empty => {
                self.status = Status::Empty;
                self.search = None;
                self.clear_matches();
            }
            FindPattern::Invalid => self.status = Status::Invalid,
            FindPattern::Ready(regex) => {
                self.status = Status::Ready;
                self.search = Some(regex);
                self.run(true);
                self.reveal_current();
            }
        }
        cx.notify();
    }

    fn on_output(&mut self, cx: &mut Context<Self>) {
        let alt_screen = self.terminal.mode().contains(TermMode::ALT_SCREEN);
        if alt_screen != self.alt_screen {
            self.alt_screen = alt_screen;
            cx.notify();
        }
        self.schedule_refresh(cx);
    }

    fn schedule_refresh(&mut self, cx: &mut Context<Self>) {
        if self.refresh.is_some() || self.search.is_none() {
            return;
        }
        let wait = self.searched_at.map_or(Duration::ZERO, |at| {
            REFRESH_INTERVAL.saturating_sub(at.elapsed())
        });
        self.refresh = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                this.refresh = None;
                if this.open && this.search.is_some() {
                    this.run(false);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Recomputes the current match and the count. A new query starts at the
    /// newest match at or above the viewport's bottom; output keeps the
    /// current match on its text, or moves to the nearest one when that text
    /// has gone.
    fn run(&mut self, new_query: bool) {
        self.select(
            |term, regex, carried| match carried.filter(|_| !new_query) {
                Some(carried) => search::locate(term, regex, &carried),
                None => search::newest_at_or_above(term, regex, viewport_bottom(term)),
            },
        );
    }

    fn step(&mut self, older: bool, cx: &mut Context<Self>) {
        if !self.can_step() {
            return;
        }
        self.select(|term, regex, carried| match carried {
            Some(carried) => search::step_match(term, regex, &carried, older),
            None => search::newest_at_or_above(term, regex, viewport_bottom(term)),
        });
        self.reveal_current();
        cx.notify();
    }

    /// Picks the current match with `pick`, given the old one carried to the
    /// latest positions, and counts the matches under one term lock.
    fn select(
        &mut self,
        pick: impl FnOnce(&Term<EventProxy>, &mut RegexSearch, Option<Match>) -> Option<Match>,
    ) {
        let Some(regex) = self.search.as_mut() else {
            return;
        };
        let term = self.terminal.term.lock();
        let mark = self.terminal.scroll_mark();
        let carried = carry(self.current.as_ref(), self.mark, mark);
        let (newest, more) = search::newest_matches(&term, regex);
        let current = (!newest.is_empty())
            .then(|| pick(&term, regex, carried))
            .flatten();
        let count = FindCount::new(&newest, more, current.as_ref());
        self.alt_screen = term.mode().contains(TermMode::ALT_SCREEN);
        drop(term);
        self.count = count;
        self.mark = mark;
        self.searched_at = Some(Instant::now());
        self.set_current(current);
    }

    fn can_step(&self) -> bool {
        self.status == Status::Ready && self.count.total > 0
    }

    /// Scrolls locally, never through a mouse report, and only when the
    /// current match is off screen.
    fn reveal_current(&self) {
        let state = self.terminal.scroll_state();
        let Some(offset) = self.current_now().and_then(|found| {
            search::centring_offset(
                &found,
                state.screen_lines,
                state.history_lines,
                state.display_offset,
            )
        }) else {
            return;
        };
        self.terminal.scroll_to_display_offset(offset);
    }

    fn toggle_match_case(&mut self, cx: &mut Context<Self>) {
        self.match_case = !self.match_case;
        self.search_query(cx);
    }

    fn toggle_regex(&mut self, cx: &mut Context<Self>) {
        self.regex = !self.regex;
        self.search_query(cx);
    }

    /// Enter, ⇧Enter, Esc and ⌘G / ⇧⌘G (F3 / ⇧F3 off macOS) are fixed keys of
    /// the bar's field.
    fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let plain = !modifiers.platform && !modifiers.control && !modifiers.alt;
        let step_key = if cfg!(target_os = "macos") {
            keystroke.key.eq_ignore_ascii_case("g")
                && modifiers.platform
                && !modifiers.control
                && !modifiers.alt
        } else {
            keystroke.key == "f3" && plain
        };
        if keystroke.key == "escape" && plain && !modifiers.shift {
            self.close(window, cx);
            true
        } else if (keystroke.key == "enter" && plain) || step_key {
            self.step(!modifiers.shift, cx);
            true
        } else {
            false
        }
    }

    /// The matches to paint over the viewport's `rows` lines.
    pub(crate) fn highlights(
        &mut self,
        term: &Term<EventProxy>,
        rows: usize,
    ) -> Option<FindHighlights> {
        if !self.open {
            return None;
        }
        let regex = self.search.as_mut()?;
        let top = -(term.grid().display_offset() as i32);
        let bottom = (top + rows.max(1) as i32 - 1).min(term.screen_lines() as i32 - 1);
        let matches = search::matches_in_lines(term, regex, Line(top), Line(bottom));
        Some(FindHighlights {
            matches,
            current: self.current_now(),
        })
    }

    fn count_label(&self) -> Option<(SharedString, bool)> {
        match self.status {
            Status::Empty => None,
            Status::Invalid => Some((crate::i18n::t("terminal.find_invalid_regex"), true)),
            Status::Ready if self.count.total == 0 => {
                Some((crate::i18n::t("terminal.find_no_results"), true))
            }
            Status::Ready => {
                let capped = format!("{}+", search::MATCH_LIMIT);
                let total = if self.count.more {
                    capped.clone()
                } else {
                    self.count.total.to_string()
                };
                let current = match (self.count.index, &self.current) {
                    (Some(index), _) => (index + 1).to_string(),
                    (None, Some(_)) => capped,
                    (None, None) => "0".into(),
                };
                Some((
                    crate::i18n::tf(
                        "terminal.find_count",
                        &[("current", current), ("total", total)],
                    )
                    .into(),
                    false,
                ))
            }
        }
    }
}

/// `current`, found at `found_at`, carried to `now`.
fn carry(current: Option<&Match>, found_at: ScrollMark, now: ScrollMark) -> Option<Match> {
    let lines = found_at.moved_since(now)?;
    current.map(|found| search::shift_match(found, lines))
}

fn viewport_bottom(term: &Term<EventProxy>) -> Line {
    Line(term.screen_lines() as i32 - 1 - term.grid().display_offset() as i32)
}

/// The terminal surface `surface` with its find bar floating over the
/// top-right corner. The bar sits outside the surface so its keys never
/// reach the terminal.
pub(crate) fn with_find_bar(surface: impl IntoElement, find: &Entity<TerminalFind>) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .child(surface)
        .child(find_bar(find))
        .into_any_element()
}

/// The find bar over the top-right corner of its parent, never wider than
/// the parent allows. It is always in the tree, empty while closed, so
/// opening it re-renders only the bar.
pub(crate) fn find_bar(find: &Entity<TerminalFind>) -> AnyElement {
    div()
        .absolute()
        .top(rems(10. / 16.))
        .left(rems(14. / 16.))
        .right(rems(14. / 16.))
        .flex()
        .justify_end()
        .child(find.clone())
        .into_any_element()
}

/// Leaving the tab or mission drops the view's attachment, and with it the
/// find state; that counts as closing find, so the viewport goes back. The
/// hold goes first, as in `discard`, so output arriving as the viewport
/// returns to the bottom is followed rather than held.
impl Drop for TerminalFind {
    fn drop(&mut self) {
        self.hold = None;
        self.restore_viewport();
    }
}

impl Render for TerminalFind {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let id = cx.entity_id();
        let root = cx.entity().downgrade();
        let action = |root: &WeakEntity<Self>,
                      act: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            let root = root.clone();
            move |window: &mut Window, cx: &mut App| {
                root.update(cx, |this, cx| act(this, window, cx)).ok();
            }
        };
        let can_step = self.can_step();
        let count = self.count_label();
        div()
            .id(("terminal-find", id))
            .key_context("TerminalFind")
            .on_action(cx.listener(|this, _: &FindInTerminal, window, cx| this.open(window, cx)))
            .occlude()
            .debug_selector(|| "TERMINAL_FIND_BAR".into())
            .w(rems(380. / 16.))
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::panel())
            .shadow(vec![BoxShadow {
                color: gpui::hsla(0., 0., 0., 0.35),
                offset: gpui::point(px(0.), px(8.)),
                blur_radius: px(24.),
                spread_radius: px(0.),
                inset: false,
            }])
            // In a narrow pane the count and the step and close buttons wrap
            // onto a second line, so the query field keeps a usable width.
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_end()
                    .gap(rems(6. / 16.))
                    .min_h(rems(34. / 16.))
                    .py(rems(6. / 16.))
                    .pl(rems(10. / 16.))
                    .pr(rems(6. / 16.))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(rems(QUERY_GROUP_MIN_WIDTH / 16.))
                            .items_center()
                            .gap(rems(6. / 16.))
                            .child(
                                svg()
                                    .flex_none()
                                    .path("search.svg")
                                    .size(rems(13. / 16.))
                                    .text_color(theme::faint()),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "TERMINAL_FIND_QUERY".into())
                                    .flex_1()
                                    .min_w(rems(QUERY_MIN_WIDTH / 16.))
                                    .child(self.field.clone()),
                            )
                            .child(bar_button(
                                "terminal-find-match-case",
                                BarContent::Label("Aa", false),
                                "terminal.find_match_case",
                                true,
                                self.match_case,
                                action(&root, |this, _, cx| this.toggle_match_case(cx)),
                            ))
                            .child(bar_button(
                                "terminal-find-regex",
                                BarContent::Label(".*", true),
                                "terminal.find_regex",
                                true,
                                self.regex,
                                action(&root, |this, _, cx| this.toggle_regex(cx)),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(rems(6. / 16.))
                            .children(count.map(|(label, danger)| {
                                div()
                                    .debug_selector(|| "TERMINAL_FIND_COUNT".into())
                                    .flex_none()
                                    .text_size(theme::text_ui())
                                    .text_color(if danger {
                                        theme::danger()
                                    } else {
                                        theme::muted()
                                    })
                                    .child(label)
                            }))
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(1.))
                                    .h(rems(14. / 16.))
                                    .bg(theme::border_strong()),
                            )
                            .child(bar_button(
                                "terminal-find-previous",
                                BarContent::Icon("chevron-up.svg", 14.),
                                "terminal.find_previous",
                                can_step,
                                false,
                                action(&root, |this, _, cx| this.step(false, cx)),
                            ))
                            .child(bar_button(
                                "terminal-find-next",
                                BarContent::Icon("chevron-down.svg", 14.),
                                "terminal.find_next",
                                can_step,
                                false,
                                action(&root, |this, _, cx| this.step(true, cx)),
                            ))
                            .child(bar_button(
                                "terminal-find-close",
                                BarContent::Icon("close.svg", 13.),
                                "terminal.find_close",
                                true,
                                false,
                                action(&root, |this, window, cx| this.close(window, cx)),
                            )),
                    ),
            )
            .when(self.alt_screen, |bar| {
                bar.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .pt(rems(6. / 16.))
                        .pb(rems(7. / 16.))
                        .px(rems(10. / 16.))
                        .border_t_1()
                        .border_color(theme::border())
                        .child(
                            svg()
                                .flex_none()
                                .path("info.svg")
                                .size(rems(12. / 16.))
                                .text_color(theme::faint()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(theme::text_meta())
                                .text_color(theme::muted())
                                .child(crate::i18n::t("terminal.find_alternate_screen")),
                        ),
                )
            })
            .into_any_element()
    }
}

enum BarContent {
    /// Text, and whether it is set in the monospace font.
    Label(&'static str, bool),
    /// An icon and its size in pixels.
    Icon(&'static str, f32),
}

/// A 22px bar button: muted at rest, the raised fill and full text colour on
/// hover, accent with an accent outline while toggled on, faint when it
/// cannot act.
fn bar_button(
    name: &'static str,
    content: BarContent,
    tooltip_key: &'static str,
    enabled: bool,
    active: bool,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let color = if active {
        theme::accent()
    } else if enabled {
        theme::muted()
    } else {
        theme::faint()
    };
    let hover_color = if active {
        theme::accent()
    } else {
        theme::text()
    };
    let content = match content {
        BarContent::Label(label, monospace) => div()
            .text_size(if monospace {
                theme::text_ui()
            } else {
                theme::text_meta()
            })
            .font_weight(FontWeight::SEMIBOLD)
            .when(monospace, |label| {
                label.font_family(theme::UI_MONOSPACE_FONT)
            })
            .text_color(color)
            .when(enabled, |label| {
                label.group_hover(name, |style| style.text_color(hover_color))
            })
            .child(label)
            .into_any_element(),
        BarContent::Icon(path, size) => svg()
            .path(path)
            .size(rems(size / 16.))
            .text_color(color)
            .when(enabled, |icon| {
                icon.group_hover(name, |style| style.text_color(hover_color))
            })
            .into_any_element(),
    };
    let button = div()
        .id(name)
        .debug_selector(move || name.to_uppercase())
        .group(name)
        .flex_none()
        .size(rems(22. / 16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rems(5. / 16.))
        .when(active, |button| {
            button
                .bg(theme::raised())
                .border_1()
                .border_color(theme::accent())
        })
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(theme::raised()))
                .on_click(move |_, window, cx| on_press(window, cx))
        })
        .child(content);
    Tooltip::new(
        format!("{name}-tooltip"),
        crate::i18n::t(tooltip_key),
        button,
    )
    .into_any_element()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::terminal::control::tests::{attached, mirror_named};
    use crate::AttachedChat;
    use alacritty_terminal::index::{Column, Point, Side};
    use alacritty_terminal::selection::SelectionType;
    use gpui::{TestAppContext, VisualTestContext};
    use runner_core::protocol::terminal::TerminalController;

    struct Host {
        chat: AttachedChat,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let find = self.chat.terminal_find.clone();
            let open = find.clone();
            div().size_full().relative().child(with_find_bar(
                div()
                    .absolute()
                    .inset_0()
                    .key_context("Terminal")
                    .track_focus(&self.chat.terminal_focus)
                    .on_action(move |_: &FindInTerminal, window, cx| {
                        open.update(cx, |find, cx| find.open(window, cx));
                    }),
                &find,
            ))
        }
    }

    /// Sixty numbered lines on an 80x24 terminal; the FAIL lines land at
    /// grid lines -26 (scrollback) and 14 (on screen).
    fn setup() -> (
        Arc<TerminalMirror>,
        VisualTestContext,
        Entity<TerminalFind>,
        FocusHandle,
    ) {
        let (terminal, visual, find, focus, _) = setup_in(TestAppContext::single(), None);
        (terminal, visual, find, focus)
    }

    type Frames = (
        std::sync::mpsc::Sender<runner_core::protocol::terminal::TerminalFrame>,
        std::sync::mpsc::Receiver<()>,
    );

    fn setup_in(
        mut cx: TestAppContext,
        store: Option<&std::path::Path>,
    ) -> (
        Arc<TerminalMirror>,
        VisualTestContext,
        Entity<TerminalFind>,
        FocusHandle,
        Frames,
    ) {
        if let Some(root) = store {
            crate::app_store::test_lifecycle_store(&mut cx, root);
        }
        let (_host, terminal, tx, ack) = mirror_named("find", TerminalController::Desktop);
        let bytes = (0..60)
            .map(|line| {
                let word = if line == 10 || line == 50 {
                    "FAIL"
                } else {
                    "pass"
                };
                format!("\r\n{word} {line}")
            })
            .collect::<String>();
        terminal.test_feed(1, bytes.as_bytes());
        cx.update(|cx| {
            crate::keymap::install_bindings(
                cx,
                &Default::default(),
                crate::keymap::TerminalCtrlKeys::RunnerFirst,
                false,
            )
        });
        let chat_terminal = terminal.clone();
        let window = cx.add_window(move |_, cx| Host {
            chat: attached(chat_terminal, cx, 80, 24),
        });
        let (find, focus) = window
            .read_with(&cx, |host, _| {
                (
                    host.chat.terminal_find.clone(),
                    host.chat.terminal_focus.clone(),
                )
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.update(|window, cx| focus.focus(window, cx));
        visual.run_until_parked();
        (terminal, visual, find, focus, (tx, ack))
    }

    fn open_with(visual: &mut VisualTestContext, query: &str) {
        visual.simulate_keystrokes(&crate::keymap::platform_default("cmd-f"));
        visual.run_until_parked();
        visual.simulate_input(query);
        visual.run_until_parked();
    }

    pub(crate) fn count(
        visual: &mut VisualTestContext,
        find: &Entity<TerminalFind>,
    ) -> Option<String> {
        visual.update(|_, cx| {
            find.read(cx)
                .count_label()
                .map(|(label, _)| label.to_string())
        })
    }

    fn current_text(
        terminal: &TerminalMirror,
        visual: &mut VisualTestContext,
        find: &Entity<TerminalFind>,
    ) -> String {
        let found = visual.update(|_, cx| find.read(cx).current_now()).unwrap();
        terminal
            .term
            .lock()
            .bounds_to_string(*found.start(), *found.end())
    }

    #[test]
    fn find_opens_from_the_terminal_steps_and_closes_back_to_it() {
        let (terminal, mut visual, find, focus) = setup();
        open_with(&mut visual, "fail");
        let field_focused = visual.update(|window, cx| {
            let find = find.read(cx);
            assert!(find.open);
            assert_eq!(find.query, "fail");
            find.field.read(cx).focus_handle().is_focused(window)
        });
        assert!(field_focused);
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 2"));
        assert_eq!(terminal.scroll_state().display_offset, 0);

        visual.simulate_keystrokes("enter");
        assert_eq!(count(&mut visual, &find).as_deref(), Some("2 of 2"));
        assert_eq!(terminal.scroll_state().display_offset, 37);
        assert_eq!(current_text(&terminal, &mut visual, &find), "FAIL");

        visual.simulate_keystrokes("enter");
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 2"));
        assert_eq!(terminal.scroll_state().display_offset, 0);
        visual.simulate_keystrokes("shift-enter");
        assert_eq!(count(&mut visual, &find).as_deref(), Some("2 of 2"));
        let step = if cfg!(target_os = "macos") {
            "shift-cmd-g"
        } else {
            "shift-f3"
        };
        visual.simulate_keystrokes(step);
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 2"));

        visual.simulate_keystrokes("enter escape");
        visual.run_until_parked();
        assert_eq!(terminal.scroll_state().display_offset, 0);
        let (open, query, terminal_focused) = visual.update(|window, cx| {
            let find = find.read(cx);
            (find.open, find.query.clone(), focus.is_focused(window))
        });
        assert!(!open);
        assert!(query.is_empty());
        assert!(terminal_focused);
        let highlights = visual.update(|_, cx| {
            find.update(cx, |find, _| {
                find.highlights(&terminal.term.lock(), 24).is_some()
            })
        });
        assert!(!highlights);

        visual.simulate_keystrokes(&crate::keymap::platform_default("cmd-f"));
        visual.run_until_parked();
        let (open, text) = visual.update(|_, cx| {
            let find = find.read(cx);
            (find.open, find.field.read(cx).text().to_owned())
        });
        assert!(open);
        assert!(text.is_empty());
        assert_eq!(count(&mut visual, &find), None);
    }

    #[test]
    fn closing_restores_a_scrolled_back_viewport_on_its_content() {
        let (terminal, mut visual, find, _) = setup();
        terminal.scroll_to_display_offset(10);
        open_with(&mut visual, "fail");
        assert_eq!(count(&mut visual, &find).as_deref(), Some("2 of 2"));
        assert_eq!(terminal.scroll_state().display_offset, 37);
        terminal.test_feed(2, b"\r\nmore\r\nmore");
        assert_eq!(terminal.scroll_state().display_offset, 39);
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert_eq!(terminal.scroll_state().display_offset, 12);
        assert!(!visual.update(|_, cx| find.read(cx).open));
    }

    #[test]
    fn a_selection_with_a_combining_mark_finds_its_text() {
        let (terminal, mut visual, find, _) = setup();
        terminal.test_feed(2, "\r\nresume cafe\u{301} here".as_bytes());
        let line = alacritty_terminal::index::Line(23);
        terminal.start_selection(
            SelectionType::Simple,
            Point::new(line, Column(7)),
            Side::Left,
        );
        terminal.update_selection(Point::new(line, Column(10)), Side::Right);
        visual.simulate_keystrokes(&crate::keymap::platform_default("cmd-f"));
        visual.run_until_parked();
        assert_eq!(
            visual.update(|_, cx| find.read(cx).query.clone()),
            "cafe\u{301}"
        );
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 1"));
        assert_eq!(current_text(&terminal, &mut visual, &find), "cafe\u{301}");
    }

    #[test]
    fn a_selection_fills_the_query_and_reopening_selects_it() {
        let (terminal, mut visual, find, focus) = setup();
        let line = alacritty_terminal::index::Line(14);
        terminal.start_selection(
            SelectionType::Simple,
            Point::new(line, Column(0)),
            Side::Left,
        );
        terminal.update_selection(Point::new(line, Column(3)), Side::Right);
        visual.simulate_keystrokes(&crate::keymap::platform_default("cmd-f"));
        visual.run_until_parked();
        assert_eq!(visual.update(|_, cx| find.read(cx).query.clone()), "FAIL");
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 2"));

        visual.simulate_input("pass 5");
        visual.run_until_parked();
        assert_eq!(visual.update(|_, cx| find.read(cx).query.clone()), "pass 5");
        assert!(terminal.selection_text().is_some());
        for focused in [true, false] {
            if !focused {
                visual.update(|window, cx| focus.focus(window, cx));
            }
            visual.simulate_keystrokes(&crate::keymap::platform_default("cmd-f"));
            visual.run_until_parked();
            assert_eq!(visual.update(|_, cx| find.read(cx).query.clone()), "pass 5");
        }
        visual.simulate_input("FAIL");
        visual.run_until_parked();
        assert_eq!(visual.update(|_, cx| find.read(cx).query.clone()), "FAIL");
    }

    #[test]
    fn past_a_thousand_matches_the_count_caps_but_selection_follows_the_viewport() {
        let (terminal, mut visual, find, _) = setup();
        let lines = (0..1500)
            .map(|line| format!("\r\nhit {line}"))
            .collect::<String>();
        terminal.test_feed(2, lines.as_bytes());
        terminal.scroll_to_display_offset(1200);
        open_with(&mut visual, "hit");
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1000+ of 1000+"));
        assert_eq!(terminal.scroll_state().display_offset, 1200);
        let current = visual.update(|_, cx| find.read(cx).current_now()).unwrap();
        assert_eq!(current.start().line.0, 23 - 1200);
        visual.simulate_keystrokes("shift-enter");
        let newer = visual.update(|_, cx| find.read(cx).current_now()).unwrap();
        assert_eq!(newer.start().line.0, 24 - 1200);
    }

    fn lines(word: &str, range: std::ops::Range<usize>) -> Vec<u8> {
        range
            .map(|line| format!("\r\n{word} {line}"))
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn stepping_before_a_refresh_from_a_match_that_left_the_scrollback() {
        let (terminal, mut visual, find, _) = setup();
        terminal.configure(40, alacritty_terminal::vte::ansi::CursorShape::Block, false);
        terminal.test_feed(2, &lines("hit", 0..100));
        open_with(&mut visual, "hit");
        visual.simulate_keystrokes("shift-enter");
        let oldest = visual.update(|_, cx| find.read(cx).current_now()).unwrap();
        assert_eq!(oldest.start().line.0, -40);
        terminal.test_feed(3, &lines("hit", 100..101));
        visual.simulate_keystrokes("enter");
        let newest = visual.update(|_, cx| find.read(cx).current_now()).unwrap();
        assert_eq!(newest.start().line.0, 23);
        assert_eq!(terminal.scroll_state().display_offset, 0);
        visual.simulate_keystrokes("shift-enter");
        let oldest = visual.update(|_, cx| find.read(cx).current_now()).unwrap();
        assert_eq!(oldest.start().line.0, -40);
    }

    #[test]
    fn a_burst_the_length_of_the_ring_clamps_the_held_view_and_closing() {
        // 10,000 lines of scrollback on 24 rows: the ring holds 10,024 rows.
        let (terminal, mut visual, _, _) = setup();
        open_with(&mut visual, "FAIL");
        terminal.test_feed(2, &lines("more", 0..10_024));
        assert_eq!(terminal.scroll_state().display_offset, 10_000);
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert_eq!(terminal.scroll_state().display_offset, 0);

        terminal.scroll_to_display_offset(10);
        open_with(&mut visual, "more");
        assert_eq!(terminal.scroll_state().display_offset, 10);
        terminal.test_feed(3, &lines("later", 0..10_026));
        assert_eq!(terminal.scroll_state().display_offset, 10_000);
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert_eq!(terminal.scroll_state().display_offset, 10_000);
    }

    #[test]
    fn a_resize_refreshes_the_matches_through_terminal_updates() {
        let temp = tempfile::tempdir().unwrap();
        let (terminal, mut visual, find, _, (tx, ack)) =
            setup_in(TestAppContext::single(), Some(temp.path()));
        open_with(&mut visual, "FAIL");
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 2"));
        crate::terminal::control::tests::change(&tx, &ack, 2, TerminalController::Desktop);
        assert_eq!(terminal.size(), (91, 31));
        assert!(visual.update(|_, cx| find.read(cx).current_now()).is_none());
        visual.update(|_, cx| {
            let updates = cx
                .global::<crate::app_store::GlobalAppStore>()
                .0
                .read(cx)
                .terminal_updates
                .clone();
            updates.update(cx, |_, cx| cx.emit("find".to_string()));
        });
        visual.executor().advance_clock(REFRESH_INTERVAL);
        visual.run_until_parked();
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 2"));
        assert_eq!(current_text(&terminal, &mut visual, &find), "FAIL");
        visual.simulate_keystrokes("enter");
        assert_eq!(count(&mut visual, &find).as_deref(), Some("2 of 2"));
        assert_eq!(current_text(&terminal, &mut visual, &find), "FAIL");
    }

    #[test]
    fn output_keeps_the_current_match_and_holds_the_viewport() {
        let (terminal, mut visual, find, _) = setup();
        open_with(&mut visual, "FAIL");
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 2"));
        let more = (0..5)
            .map(|line| format!("\r\nnew {line}"))
            .collect::<String>();
        terminal.test_feed(2, more.as_bytes());
        assert_eq!(terminal.scroll_state().display_offset, 5);
        assert_eq!(current_text(&terminal, &mut visual, &find), "FAIL");
        terminal.test_feed(3, b"\r\nFAIL newest");
        visual.update(|_, cx| find.update(cx, |find, _| find.run(false)));
        assert_eq!(count(&mut visual, &find).as_deref(), Some("2 of 3"));
        assert_eq!(current_text(&terminal, &mut visual, &find), "FAIL");
        assert_eq!(terminal.scroll_state().display_offset, 6);
    }

    #[test]
    fn invalid_regex_keeps_the_last_matches_and_disables_stepping() {
        let (_terminal, mut visual, find, _) = setup();
        open_with(&mut visual, "fail");
        visual.update(|_, cx| find.update(cx, |find, cx| find.toggle_regex(cx)));
        visual.simulate_input("(");
        visual.run_until_parked();
        assert_eq!(count(&mut visual, &find).as_deref(), Some("Invalid regex"));
        let (matches, can_step) =
            visual.update(|_, cx| (find.read(cx).count.total, find.read(cx).can_step()));
        assert_eq!(matches, 2);
        assert!(!can_step);
        visual.simulate_keystrokes("backspace");
        visual.run_until_parked();
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 2"));
        visual.update(|_, cx| find.update(cx, |find, cx| find.toggle_match_case(cx)));
        assert_eq!(count(&mut visual, &find).as_deref(), Some("No results"));
    }

    #[test]
    fn the_alternate_screen_shows_the_hint_and_highlights_cover_the_viewport() {
        let (terminal, mut visual, find, _) = setup();
        open_with(&mut visual, "pass");
        let highlighted = visual.update(|_, cx| {
            find.update(cx, |find, _| {
                let term = terminal.term.lock();
                find.highlights(&term, 24).unwrap().matches.len()
            })
        });
        assert_eq!(highlighted, 23);
        assert!(!visual.update(|_, cx| find.read(cx).alt_screen));
        terminal.test_feed(2, b"\x1b[?1049h\x1b[Hpass on the alternate screen");
        visual.update(|_, cx| find.update(cx, |find, _| find.run(false)));
        assert!(visual.update(|_, cx| find.read(cx).alt_screen));
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 1"));
    }

    #[test]
    fn a_selection_seeds_find_while_the_program_reports_the_mouse() {
        let (terminal, mut visual, find, _) = setup();
        terminal.test_feed(
            2,
            b"\x1b[?1049h\x1b[?1000h\x1b[HREADY on the alternate screen",
        );
        assert!(terminal
            .mode()
            .contains(TermMode::ALT_SCREEN | TermMode::MOUSE_REPORT_CLICK));
        let line = alacritty_terminal::index::Line(0);
        terminal.start_selection(
            SelectionType::Simple,
            Point::new(line, Column(0)),
            Side::Left,
        );
        terminal.update_selection(Point::new(line, Column(4)), Side::Right);
        visual.simulate_keystrokes(&crate::keymap::platform_default("cmd-f"));
        visual.run_until_parked();
        assert_eq!(visual.update(|_, cx| find.read(cx).query.clone()), "READY");
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 1"));
    }

    struct SizedHost {
        chat: AttachedChat,
        width: f32,
    }

    impl Render for SizedHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let find = self.chat.terminal_find.clone();
            let open = find.clone();
            div().size_full().child(
                div()
                    .debug_selector(|| "FIND_HOST".into())
                    .relative()
                    .w(px(self.width))
                    .h(px(300.))
                    .child(with_find_bar(
                        div()
                            .absolute()
                            .inset_0()
                            .key_context("Terminal")
                            .track_focus(&self.chat.terminal_focus)
                            .on_action(move |_: &FindInTerminal, window, cx| {
                                open.update(cx, |find, cx| find.open(window, cx));
                            }),
                        &find,
                    )),
            )
        }
    }

    #[test]
    fn the_bar_fits_a_narrow_pane_and_keeps_its_field_and_controls() {
        use crate::i18n::Locale;
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            for zoom in [1., 1.25] {
                for width in [240. * zoom, 1200.] {
                    crate::i18n::testing::with_locale(locale, || {
                        check_bar_layout(width, zoom, locale)
                    });
                }
            }
        }
    }

    fn check_bar_layout(width: f32, zoom: f32, locale: crate::i18n::Locale) {
        let (_host, terminal, _tx, _ack) = mirror_named("find", TerminalController::Desktop);
        terminal.test_feed(1, &lines("hit", 0..1100));
        let mut cx = TestAppContext::single();
        cx.update(|cx| {
            crate::keymap::install_bindings(
                cx,
                &Default::default(),
                crate::keymap::TerminalCtrlKeys::RunnerFirst,
                false,
            )
        });
        let chat_terminal = terminal.clone();
        let window = cx.add_window(move |_, cx| SizedHost {
            chat: attached(chat_terminal, cx, 80, 24),
            width,
        });
        let (find, focus) = window
            .read_with(&cx, |host, _| {
                (
                    host.chat.terminal_find.clone(),
                    host.chat.terminal_focus.clone(),
                )
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.update(|window, cx| {
            window.set_rem_size(px(16. * zoom));
            focus.focus(window, cx);
        });
        open_with(&mut visual, "hit");
        // Shift+Enter from the newest match wraps to the oldest, past the cap.
        visual.simulate_keystrokes("shift-enter");
        let capped = crate::i18n::tf(
            "terminal.find_count",
            &[("current", "1000+".into()), ("total", "1000+".into())],
        );
        assert_eq!(count(&mut visual, &find), Some(capped));
        for state in ["count", "invalid"] {
            if state == "invalid" {
                visual.update(|_, cx| find.update(cx, |find, cx| find.toggle_regex(cx)));
                visual.simulate_input("(");
                visual.run_until_parked();
                assert_eq!(
                    count(&mut visual, &find),
                    Some(crate::i18n::t("terminal.find_invalid_regex").to_string())
                );
            }
            let pane = visual.debug_bounds("FIND_HOST").unwrap();
            let bar = visual.debug_bounds("TERMINAL_FIND_BAR").unwrap();
            let query = visual.debug_bounds("TERMINAL_FIND_QUERY").unwrap();
            let context = format!("{locale:?} {zoom}x {width} {state}: pane {pane:?} bar {bar:?}");
            assert!(
                bar.left() >= pane.left() && bar.right() <= pane.right(),
                "{context}"
            );
            assert!(
                query.size.width >= px(QUERY_MIN_WIDTH * zoom) - px(0.5),
                "{context} {query:?}"
            );
            for control in [
                "TERMINAL_FIND_QUERY",
                "TERMINAL-FIND-MATCH-CASE",
                "TERMINAL-FIND-REGEX",
                "TERMINAL_FIND_COUNT",
                "TERMINAL-FIND-PREVIOUS",
                "TERMINAL-FIND-NEXT",
                "TERMINAL-FIND-CLOSE",
            ] {
                let bounds = visual.debug_bounds(control).unwrap();
                assert!(
                    bounds.left() >= bar.left()
                        && bounds.right() <= bar.right()
                        && bounds.top() >= bar.top()
                        && bounds.bottom() <= bar.bottom(),
                    "{context} {control} {bounds:?}"
                );
            }
            let close = visual.debug_bounds("TERMINAL-FIND-CLOSE").unwrap();
            if width > 1000. {
                assert_eq!(bar.size.width, px(380. * zoom), "{context}");
                assert!(bar.size.height <= px(36. * zoom + 0.5), "{context}");
            } else {
                assert!(bar.size.width <= px(width - 28. * zoom), "{context}");
                assert!(close.top() > query.bottom(), "{context} {close:?}");
            }
        }
    }

    /// A mirror whose waker, once armed, delivers three more lines. The
    /// viewport restore calls the waker right after releasing the term, so
    /// this is output arriving while find closes. The term's own events also
    /// call the waker while it is locked; those calls leave it armed.
    pub(crate) fn injecting_mirror() -> (Arc<TerminalMirror>, Arc<std::sync::atomic::AtomicBool>) {
        use std::sync::atomic::{AtomicBool, Ordering};
        let target = Arc::new(std::sync::OnceLock::<std::sync::Weak<TerminalMirror>>::new());
        let armed = Arc::new(AtomicBool::new(false));
        let waker: Arc<dyn Fn() + Send + Sync> = {
            let (target, armed) = (target.clone(), armed.clone());
            let seq = std::sync::atomic::AtomicU64::new(1_000_000);
            Arc::new(move || {
                let Some(mirror) = target.get().and_then(std::sync::Weak::upgrade) else {
                    return;
                };
                if mirror.term.try_lock_unfair().is_none() {
                    return;
                }
                if armed.swap(false, Ordering::SeqCst) {
                    let seq = seq.fetch_add(1, Ordering::SeqCst);
                    mirror.test_feed(seq, &lines("late", 0..3));
                }
            })
        };
        let (_host, mirror, _tx, _ack) = crate::terminal::control::tests::mirror_waking(
            "find",
            TerminalController::Desktop,
            waker,
        );
        target.set(Arc::downgrade(&mirror)).unwrap();
        (mirror, armed)
    }

    pub(crate) fn needle_output() -> Vec<u8> {
        let mut output = lines("out", 0..5);
        output.extend_from_slice(b"\r\nthe needle line");
        output.extend_from_slice(&lines("out", 5..60));
        output
    }

    struct DroppableHost {
        chat: Option<AttachedChat>,
    }

    impl Render for DroppableHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let Some(chat) = &self.chat else {
                return div().into_any_element();
            };
            let find = chat.terminal_find.clone();
            let open = find.clone();
            div()
                .size_full()
                .relative()
                .child(with_find_bar(
                    div()
                        .absolute()
                        .inset_0()
                        .key_context("Terminal")
                        .track_focus(&chat.terminal_focus)
                        .on_action(move |_: &FindInTerminal, window, cx| {
                            open.update(cx, |find, cx| find.open(window, cx));
                        }),
                    &find,
                ))
                .into_any_element()
        }
    }

    #[test]
    fn output_as_find_closes_is_followed_after_leave_and_after_a_dropped_view() {
        use std::sync::atomic::Ordering;
        let (terminal, armed) = injecting_mirror();
        terminal.test_feed(1, &needle_output());
        let mut cx = TestAppContext::single();
        cx.update(|cx| {
            crate::keymap::install_bindings(
                cx,
                &Default::default(),
                crate::keymap::TerminalCtrlKeys::RunnerFirst,
                false,
            )
        });
        let chat_terminal = terminal.clone();
        let window = cx.add_window(move |_, cx| DroppableHost {
            chat: Some(attached(chat_terminal, cx, 80, 24)),
        });
        let (find, focus) = window
            .read_with(&cx, |host, _| {
                let chat = host.chat.as_ref().unwrap();
                (chat.terminal_find.clone(), chat.terminal_focus.clone())
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.update(|window, cx| focus.focus(window, cx));

        open_with(&mut visual, "needle");
        assert!(terminal.scroll_state().display_offset > 0);
        let mark = terminal.scroll_mark();
        armed.store(true, Ordering::SeqCst);
        visual.update(|_, cx| find.update(cx, |find, cx| find.leave(cx)));
        assert_eq!(mark.moved_since(terminal.scroll_mark()), Some(3));
        assert_eq!(terminal.scroll_state().display_offset, 0);

        visual.update(|window, cx| focus.focus(window, cx));
        open_with(&mut visual, "needle");
        assert!(terminal.scroll_state().display_offset > 0);
        let mark = terminal.scroll_mark();
        let weak = find.downgrade();
        drop(find);
        armed.store(true, Ordering::SeqCst);
        window
            .update(&mut cx, |host, _, cx| {
                host.chat = None;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        // gpui frees an entity whose last handle has gone at its next update.
        visual.update(|_, _| {});
        assert!(weak.upgrade().is_none());
        assert_eq!(mark.moved_since(terminal.scroll_mark()), Some(3));
        assert_eq!(terminal.scroll_state().display_offset, 0);
    }

    /// Opens find with a query typed into its field, for hosts that cannot
    /// focus the terminal the way a keystroke would.
    pub(crate) fn open_with_query(
        find: &Entity<TerminalFind>,
        query: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        find.update(cx, |find, cx| {
            find.open(window, cx);
            find.field.update(cx, |field, cx| field.set_text(query, cx));
        });
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Leave {
        Tab,
        /// The shell's session has ended first, so the window keeps its
        /// attachment while the tab is hidden.
        EndedTab,
        /// The chat route is left, which drops every attachment.
        Route,
    }

    #[test]
    fn leaving_the_tab_closes_find_and_restores_the_scroll_position() {
        leave(Leave::Tab);
    }

    #[test]
    fn leaving_the_tab_closes_find_in_an_ended_session_it_keeps_attached() {
        leave(Leave::EndedTab);
    }

    #[test]
    fn leaving_the_chat_route_closes_find_when_its_attachment_drops() {
        leave(Leave::Route);
    }

    /// Opens find in a shell tab on an old match, then leaves it.
    fn leave(how: Leave) {
        let ended = how == Leave::EndedTab;
        use crate::{AppRoute, NativeRoot};
        use runner_app::pane_layout::PaneLayout;

        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let shell = store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let core = store.test_core.clone();
            let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
                Arc::new(core.session_events());
            let conn = core.db.get().unwrap();
            for (index, id) in ["busy-shell", "direct-agent"].into_iter().enumerate() {
                core.sessions
                    .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
                    .unwrap();
                store.bridge.attach(id).unwrap();
                let layout = PaneLayout::single(Some(id), &[]);
                runner_daemon::repo::node::create_tab(
                    &conn,
                    None,
                    "",
                    index as i64,
                    &layout.serialize().unwrap(),
                )
                .unwrap();
            }
            store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
            store.bridge.session("busy-shell").unwrap()
        });
        let mut output = lines("out", 0..5);
        output.extend_from_slice(b"\r\nthe needle line");
        output.extend_from_slice(&lines("out", 5..60));
        shell.test_feed(1_000_000, &output);
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "find-tab-switch".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate_session("busy-shell"));
            root.route = AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            root.ensure_active_tab_attached(window, cx).unwrap();
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.run_until_parked();
        let find = host
            .update(&mut cx, |root, window, cx| {
                let find = root.attached["busy-shell"].terminal_find.clone();
                find.update(cx, |find, cx| {
                    find.open(window, cx);
                    find.field
                        .update(cx, |field, cx| field.set_text("needle", cx));
                });
                find
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(count(&mut visual, &find).as_deref(), Some("1 of 1"));
        assert!(shell.scroll_state().display_offset > 0);
        assert!(visual.debug_bounds("TERMINAL_FIND_BAR").is_some());
        if ended {
            store.update(&mut cx, |store, _| {
                let entry = store
                    .sessions
                    .iter_mut()
                    .find(|entry| entry.session_id == "busy-shell")
                    .unwrap();
                entry.status = runner_core::protocol::model::SessionStatus::Stopped;
            });
        }

        if how == Leave::Route {
            let weak = find.downgrade();
            drop(find);
            host.update(&mut cx, |root, window, cx| {
                root.set_route(AppRoute::Settings, cx);
                assert!(root.attached.is_empty());
                window.refresh();
            })
            .unwrap();
            visual.run_until_parked();
            assert!(visual.debug_bounds("TERMINAL_FIND_BAR").is_none());
            assert!(weak.upgrade().is_none());
            assert_eq!(shell.scroll_state().display_offset, 0);
            return;
        }
        host.update(&mut cx, |root, window, cx| {
            assert!(root.tabs.activate_session("direct-agent"));
            root.ensure_active_tab_attached(window, cx).unwrap();
            assert_eq!(root.attached.contains_key("busy-shell"), ended);
        })
        .unwrap();
        visual.run_until_parked();
        let (open, query) = visual.update(|_, cx| {
            let find = find.read(cx);
            (find.open, find.query.clone())
        });
        assert!(!open);
        assert!(query.is_empty());
        assert_eq!(shell.scroll_state().display_offset, 0);
        if ended {
            host.update(&mut cx, |root, window, _| {
                assert!(!root.attached["busy-shell"]
                    .terminal_focus
                    .is_focused(window));
            })
            .unwrap();
        }

        host.update(&mut cx, |root, window, cx| {
            assert!(root.tabs.activate_session("busy-shell"));
            root.ensure_active_tab_attached(window, cx).unwrap();
        })
        .unwrap();
        visual.run_until_parked();
        let (open, query) = host
            .read_with(&cx, |root, cx| {
                let find = root.attached["busy-shell"].terminal_find.read(cx);
                (find.open, find.query.clone())
            })
            .unwrap();
        assert!(!open);
        assert!(query.is_empty());
        assert_eq!(shell.scroll_state().display_offset, 0);
    }
}
