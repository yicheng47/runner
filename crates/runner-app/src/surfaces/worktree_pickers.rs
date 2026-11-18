use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, App, Entity, FontWeight, MouseButton, ScrollHandle,
    SharedString, Window,
};
use runner_app::theme;
use runner_app::ui::TextField;
use runner_core::protocol::files::WorktreeKind;

use super::changes::model::{BasePickerItem, BasePickerSection, CheckoutPickerItem};

pub(crate) const BASE_PICKER_WIDTH: f32 = 300.;
pub(crate) const CHECKOUT_PICKER_WIDTH: f32 = 420.;

pub(crate) struct PickerListView<'a> {
    pub(crate) id_prefix: &'static str,
    pub(crate) selector_prefix: &'static str,
    pub(crate) title: SharedString,
    pub(crate) field: Entity<TextField>,
    pub(crate) visible: &'a [usize],
    pub(crate) highlighted: usize,
    pub(crate) scroll: &'a ScrollHandle,
    pub(crate) reveal: &'a Cell<bool>,
}

pub(crate) type PickBaseCallback = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;
pub(crate) type PickCheckoutCallback = Rc<dyn Fn(PathBuf, &mut Window, &mut App)>;

struct PickerShell<'a> {
    selector: &'static str,
    title: SharedString,
    field: Entity<TextField>,
    list_id: &'static str,
    list_selector: &'static str,
    list_max_h: f32,
    scroll: &'a ScrollHandle,
    rows: Vec<AnyElement>,
    footer: Option<AnyElement>,
}

fn picker_shell(shell: PickerShell<'_>) -> AnyElement {
    let PickerShell {
        selector,
        title,
        field,
        list_id,
        list_selector,
        list_max_h,
        scroll,
        rows,
        footer,
    } = shell;
    div()
        .when(cfg!(test), |el| el.debug_selector(move || selector.into()))
        .flex()
        .flex_col()
        .rounded(rems(8. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::raised())
        .shadow_2xl()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex_none()
                .px(px(12.))
                .pt(px(8.))
                .pb(px(2.))
                .text_size(theme::text_meta())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::muted())
                .child(title),
        )
        .child(
            div().flex_none().px(px(8.)).pt(px(4.)).pb(px(6.)).child(
                div()
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .child(
                        svg()
                            .flex_none()
                            .size(px(12.))
                            .path("search.svg")
                            .text_color(theme::faint()),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(field)),
            ),
        )
        .child(
            div()
                .id(list_id)
                .when(cfg!(test), |el| {
                    el.debug_selector(move || list_selector.into())
                })
                .max_h(px(list_max_h))
                .overflow_y_scroll()
                .track_scroll(scroll)
                .px(px(4.))
                .pb(px(if footer.is_some() { 4. } else { 6. }))
                .flex()
                .flex_col()
                .children(rows),
        )
        .children(footer)
        .into_any_element()
}

pub(crate) fn base_picker_card(
    view: PickerListView<'_>,
    items: &[BasePickerItem],
    on_pick: PickBaseCallback,
    card_selector: &'static str,
    list_id: &'static str,
    list_selector: &'static str,
) -> AnyElement {
    let mut rows = Vec::new();
    let mut last_section: Option<&BasePickerSection> = None;
    for (position, index) in view.visible.iter().copied().enumerate() {
        let item = &items[index];
        let changed_section = last_section.is_some() && last_section != Some(&item.section);
        if changed_section {
            rows.push(
                div()
                    .my(px(4.))
                    .h(px(1.))
                    .bg(theme::border())
                    .into_any_element(),
            );
        }
        if last_section != Some(&item.section) {
            let header = match &item.section {
                BasePickerSection::Auto => None,
                BasePickerSection::Local => {
                    Some(crate::i18n::text("changes.local_branches").to_owned())
                }
                BasePickerSection::Remote(remote) => Some(remote.to_uppercase()),
            };
            if let Some(header) = header {
                rows.push(
                    div()
                        .px(px(8.))
                        .pt(px(4.))
                        .pb(px(2.))
                        .text_size(theme::text_caption())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::faint())
                        .child(header)
                        .into_any_element(),
                );
            }
        }
        last_section = Some(&item.section);

        let highlighted = position == view.highlighted;
        if highlighted && view.reveal.replace(false) {
            view.scroll.scroll_to_item(rows.len());
        }
        let base_ref = item.base_ref.clone();
        let check_el = if item.current {
            svg()
                .flex_none()
                .size(px(12.))
                .path("check.svg")
                .text_color(theme::accent())
                .into_any_element()
        } else {
            div().flex_none().size(px(12.)).into_any_element()
        };
        let on_pick = on_pick.clone();
        rows.push(
            div()
                .id((view.id_prefix, position))
                .when(cfg!(test), {
                    let label = item.label.clone();
                    let prefix = view.selector_prefix;
                    move |el| el.debug_selector(move || format!("{prefix} {position} {label}"))
                })
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(highlighted, |el| el.bg(theme::sidebar_selected()))
                .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                .on_click(move |_, window, cx| on_pick(base_ref.clone(), window, cx))
                .child(check_el)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme::text_ui())
                        .text_color(theme::text())
                        .child(item.label.clone()),
                )
                .when_some(item.hint.as_ref(), |el, hint| {
                    el.child(
                        div()
                            .flex_none()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(hint.clone()),
                    )
                })
                .into_any_element(),
        );
    }
    if rows.is_empty() {
        rows.push(
            div()
                .px(px(8.))
                .py(px(10.))
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(crate::i18n::text("changes.no_matching_branches"))
                .into_any_element(),
        );
    }
    picker_shell(PickerShell {
        selector: card_selector,
        title: view.title,
        field: view.field,
        list_id,
        list_selector,
        list_max_h: 320.,
        scroll: view.scroll,
        rows,
        footer: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn checkout_picker_card(
    view: PickerListView<'_>,
    items: &[CheckoutPickerItem],
    show_main_tag_when_idle: bool,
    footer: Option<AnyElement>,
    on_pick: PickCheckoutCallback,
    card_selector: &'static str,
    list_id: &'static str,
    list_selector: &'static str,
) -> AnyElement {
    let mut rows = Vec::new();
    for (position, index) in view.visible.iter().copied().enumerate() {
        let item = &items[index];
        let highlighted = position == view.highlighted;
        if highlighted && view.reveal.replace(false) {
            view.scroll.scroll_to_item(rows.len());
        }
        let check_el = if item.current {
            svg()
                .flex_none()
                .size(px(12.))
                .path("check.svg")
                .text_color(theme::accent())
                .into_any_element()
        } else {
            div().flex_none().size(px(12.)).into_any_element()
        };
        let kind_icon = match item.kind {
            WorktreeKind::Main => "house.svg",
            WorktreeKind::Linked => "folder-git-2.svg",
        };
        let tag_label = item.session_label.clone().or_else(|| {
            (show_main_tag_when_idle && item.kind == WorktreeKind::Main).then(|| {
                crate::surfaces::changes::model::CheckoutSessionLabel {
                    text: crate::i18n::text("ui.main_checkout").to_owned(),
                    is_own: false,
                }
            })
        });
        let pick_path = item.path.clone();
        let line2 = div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .items_center()
            .gap(px(5.))
            .text_size(theme::text_caption())
            .text_color(theme::faint())
            .when_some(item.commit.as_ref(), |el, sha| {
                el.child(
                    div()
                        .flex_none()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .child(sha.clone()),
                )
                .child(div().flex_none().child("·"))
            })
            .when_some(item.uncommitted, |el, count| {
                let (text, color) = if count == 0 {
                    (
                        crate::i18n::text("changes.clean").to_owned(),
                        theme::faint(),
                    )
                } else {
                    (
                        crate::i18n::tn("changes.uncommitted_count", count),
                        theme::warning(),
                    )
                };
                el.child(div().flex_none().text_color(color).child(text))
                    .child(div().flex_none().child("·"))
            })
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .child(item.short_path.clone()),
            );
        let on_pick = on_pick.clone();
        rows.push(
            div()
                .id((view.id_prefix, position))
                .when(cfg!(test), {
                    let branch = item.branch_label.clone();
                    let prefix = view.selector_prefix;
                    move |el| el.debug_selector(move || format!("{prefix} {position} {branch}"))
                })
                .px(px(8.))
                .py(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(highlighted, |el| el.bg(theme::sidebar_selected()))
                .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                .on_click(move |_, window, cx| on_pick(pick_path.clone(), window, cx))
                .child(check_el)
                .child(
                    svg()
                        .flex_none()
                        .size(px(13.))
                        .path(kind_icon)
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .truncate()
                                        .text_size(theme::text_ui())
                                        .text_color(theme::text())
                                        .child(item.branch_label.clone()),
                                )
                                .when_some(tag_label, |row, label| {
                                    row.child(
                                        div()
                                            .flex_none()
                                            .text_size(theme::text_caption())
                                            .text_color(if label.is_own {
                                                theme::muted()
                                            } else {
                                                theme::faint()
                                            })
                                            .child(label.text),
                                    )
                                }),
                        )
                        .child(line2),
                )
                .into_any_element(),
        );
    }
    if rows.is_empty() {
        rows.push(
            div()
                .px(px(8.))
                .py(px(10.))
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(crate::i18n::text("changes.no_matching_checkouts"))
                .into_any_element(),
        );
    }
    picker_shell(PickerShell {
        selector: card_selector,
        title: view.title,
        field: view.field,
        list_id,
        list_selector,
        list_max_h: 340.,
        scroll: view.scroll,
        rows,
        footer,
    })
}
