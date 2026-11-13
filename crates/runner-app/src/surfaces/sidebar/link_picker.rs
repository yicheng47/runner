//! The Link to chat picker (#748): a search field over the mission's agent
//! chats, opened as the submenu of a mission's Link to chat item, or at the
//! pointer after a drop on a tab that holds several chats.

use std::rc::Rc;

use gpui::{Bounds, KeyDownEvent, Pixels, ScrollHandle};
use runner_app::ui::menu::{context_menu_layer, popup_layer_sized, DismissHandler, PopupWidth};
use runner_app::ui::TextField;

use super::link::{picker_entries, ChatChoice, PickerEntry};
use crate::*;

type PickHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;

const PICKER_WIDTH: f32 = 280.;

pub(crate) struct LinkPicker {
    anchor: Bounds<Pixels>,
    /// Opened as a submenu, beside its menu row; else under the pointer.
    beside: bool,
    /// Escape in a submenu returns to its menu instead of closing everything.
    on_back: Option<DismissHandler>,
    /// The current link, marked Linked.
    linked: Option<String>,
    chats: Vec<ChatChoice>,
    query: Entity<TextField>,
    highlighted: usize,
    last_query: String,
    scroll: ScrollHandle,
    reveal: bool,
    on_pick: PickHandler,
    on_dismiss: DismissHandler,
    _query: Subscription,
}

impl LinkPicker {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        anchor: Bounds<Pixels>,
        beside: bool,
        on_back: Option<DismissHandler>,
        scope: String,
        linked: Option<String>,
        chats: Vec<ChatChoice>,
        on_pick: PickHandler,
        on_dismiss: DismissHandler,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|field_cx| {
            TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::messagef("format.search_chats_in", &[("scope", scope)]),
                false,
            )
        });
        query.update(cx, |field, field_cx| field.set_bare(true, field_cx));
        let observe = cx.observe(&query, |_, _, cx| cx.notify());
        Self {
            anchor,
            beside,
            on_back,
            linked,
            chats,
            query,
            highlighted: 0,
            last_query: String::new(),
            scroll: ScrollHandle::new(),
            reveal: false,
            on_pick,
            on_dismiss,
            _query: observe,
        }
    }

    pub(crate) fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        self.query.read(cx).focus_handle()
    }

    #[cfg(test)]
    pub(super) fn search_placeholder(&self, cx: &App) -> String {
        self.query.read(cx).placeholder().to_string()
    }

    fn shown(&self, cx: &App) -> Vec<ChatChoice> {
        picker_entries(&self.chats, self.query.read(cx).text())
            .into_iter()
            .filter_map(|entry| match entry {
                PickerEntry::Chat(chat) => Some(chat),
                PickerEntry::Heading(_) => None,
            })
            .collect()
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.shown(cx);
        let key = event.keystroke.key.as_str();
        let empty_query = self.query.read(cx).text().is_empty();
        match key {
            "up" => {
                self.highlighted = self.highlighted.saturating_sub(1);
                self.reveal = true;
            }
            "down" => {
                self.highlighted = (self.highlighted + 1).min(shown.len().saturating_sub(1));
                self.reveal = true;
            }
            "enter" => {
                if let Some(chat) = shown.get(self.highlighted) {
                    (self.on_pick)(chat.session_id.clone(), window, cx);
                }
            }
            "escape" | "left" if self.on_back.is_some() && (key == "escape" || empty_query) => {
                if let Some(back) = self.on_back.clone() {
                    back(window, cx);
                }
            }
            "escape" => (self.on_dismiss)(window, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn hint(&self, chat: &ChatChoice) -> (String, bool) {
        if self.linked.as_deref() == Some(chat.session_id.as_str()) {
            return (format!("✓ {}", crate::i18n::text("ui.linked")), true);
        }
        if chat.missions > 0 {
            return (crate::i18n::tn("count.missions", chat.missions), false);
        }
        (recency(chat.active_at, chrono::Local::now()), false)
    }
}

/// "now", "5m", "1h", "Yesterday" or a date: how long ago a chat was active.
pub(crate) fn recency(active_at: Option<i64>, now: chrono::DateTime<chrono::Local>) -> String {
    use chrono::TimeZone;
    let Some(at) = active_at.and_then(|at| chrono::Local.timestamp_millis_opt(at).single()) else {
        return String::new();
    };
    let elapsed = now.signed_duration_since(at);
    if elapsed.num_minutes() < 1 {
        crate::i18n::text("ui.link_age_now").to_owned()
    } else if elapsed.num_hours() < 1 {
        crate::i18n::tf(
            "format.link_age_minutes",
            &[("count", elapsed.num_minutes().to_string())],
        )
    } else if at.date_naive() == now.date_naive() {
        crate::i18n::tf(
            "format.link_age_hours",
            &[("count", elapsed.num_hours().to_string())],
        )
    } else if now.date_naive().pred_opt() == Some(at.date_naive()) {
        crate::i18n::text("ui.link_age_yesterday").to_owned()
    } else {
        at.format(crate::i18n::text("date.short")).to_string()
    }
}

impl Render for LinkPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.query.read(cx).text().to_owned();
        if query != self.last_query {
            self.highlighted = 0;
            self.reveal = true;
            self.last_query = query.clone();
        }
        let entries = picker_entries(&self.chats, &query);
        // The list's child holding the highlighted chat, headings included.
        let highlighted_child = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry, PickerEntry::Chat(_)))
            .nth(self.highlighted)
            .map(|(child, _)| child);
        if std::mem::take(&mut self.reveal) {
            // The first chat reveals the heading above it too.
            match highlighted_child {
                Some(_) if self.highlighted == 0 => self.scroll.scroll_to_item(0),
                Some(child) => self.scroll.scroll_to_item(child),
                None => {}
            }
        }
        let mut index = 0;
        let rows = entries
            .into_iter()
            .map(|entry| match entry {
                PickerEntry::Heading(key) => div()
                    .px(rems(8. / 16.))
                    .pt(rems(8. / 16.))
                    .pb(rems(4. / 16.))
                    .text_size(theme::text_caption())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme::faint())
                    .child(crate::i18n::text(key))
                    .into_any_element(),
                PickerEntry::Chat(chat) => {
                    let row_index = index;
                    index += 1;
                    let active = row_index == self.highlighted;
                    let (hint, accent) = self.hint(&chat);
                    let pick = self.on_pick.clone();
                    let session = chat.session_id.clone();
                    div()
                        .id(SharedString::from(format!(
                            "link-picker-{}",
                            chat.session_id
                        )))
                        .debug_selector({
                            let id = chat.session_id.clone();
                            move || format!("LINK_PICKER_CHAT {id}")
                        })
                        .px(rems(8. / 16.))
                        .py(rems(6. / 16.))
                        .flex()
                        .items_center()
                        .gap(rems(8. / 16.))
                        .rounded(rems(4. / 16.))
                        .cursor_pointer()
                        .when(active, |row| row.bg(theme::sidebar_selected()))
                        .hover(|row| row.bg(theme::sidebar_selected()))
                        .child(super::elements::sidebar_icon(
                            ChatIcon::for_runtime(&chat.runtime),
                            true,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(theme::text_body())
                                .text_color(theme::text())
                                .child(chat.title.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(theme::text_caption())
                                .text_color(if accent {
                                    theme::accent()
                                } else {
                                    theme::faint()
                                })
                                .child(hint),
                        )
                        .on_click(move |_, window, cx| pick(session.clone(), window, cx))
                        .into_any_element()
                }
            })
            .collect::<Vec<_>>();
        let empty = rows.is_empty();
        // A project can hold dozens of chats: the list scrolls within the window.
        let list_height = (window.viewport_size().height - px(160.)).clamp(px(96.), px(360.));
        let panel = div()
            .debug_selector(|| "LINK_PICKER".into())
            .w(px(PICKER_WIDTH))
            .flex()
            .flex_col()
            .gap(rems(1. / 16.))
            .p(rems(6. / 16.))
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::raised())
            .shadow_2xl()
            // A press inside the picker is the picker's; the layer's backdrop
            // (and, as a submenu, its menu's) would otherwise close it first.
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(gpui::MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(Self::on_key_down))
            .child(
                div()
                    .px(rems(8. / 16.))
                    .py(rems(6. / 16.))
                    .flex()
                    .items_center()
                    .gap(rems(8. / 16.))
                    .rounded(rems(6. / 16.))
                    .border_1()
                    .border_color(theme::accent())
                    .child(
                        gpui::svg()
                            .path("search.svg")
                            .size(rems(12. / 16.))
                            .flex_none()
                            .text_color(theme::faint()),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(self.query.clone())),
            )
            .children(empty.then(|| {
                div()
                    .px(rems(8. / 16.))
                    .py(rems(6. / 16.))
                    .text_size(theme::text_body())
                    .text_color(theme::faint())
                    .child(crate::i18n::text("ui.no_matching_chats"))
            }))
            .child(
                div()
                    .id("link-picker-list")
                    .debug_selector(|| "LINK_PICKER_LIST".into())
                    .flex()
                    .flex_col()
                    .gap(rems(1. / 16.))
                    .max_h(list_height)
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows),
            )
            .children(query.trim().is_empty().then(|| {
                div()
                    .mt(rems(4. / 16.))
                    .px(rems(8. / 16.))
                    .pt(rems(6. / 16.))
                    .border_t_1()
                    .border_color(theme::border())
                    .text_size(theme::text_caption())
                    .text_color(theme::faint())
                    .child(crate::i18n::tf(
                        "format.type_to_search_all_chats",
                        &[("count", self.chats.len().to_string())],
                    ))
            }))
            .into_any_element();
        if self.beside {
            return context_menu_layer(
                gpui::point(self.anchor.right() + px(4.), self.anchor.top() - px(6.)),
                window,
                px(PICKER_WIDTH),
                panel,
                self.on_dismiss.clone(),
            );
        }
        popup_layer_sized(
            self.anchor,
            window,
            PopupWidth::Fixed(px(PICKER_WIDTH)),
            panel,
            self.on_dismiss.clone(),
        )
    }
}
