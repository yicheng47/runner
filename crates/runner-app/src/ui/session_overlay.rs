use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, rems, svg, Animation, AnimationExt as _, App, FontWeight, RenderOnce, SharedString, Window,
};

use crate::theme;
use crate::ui::button::{spinner, Button, ButtonVariant, PressHandler};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionOverlayKind {
    Starting,
    Resuming,
    Archiving,
    Ended,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EndedStyle {
    Chat,
    Slot,
    Mission,
}

#[derive(IntoElement)]
pub struct SessionOverlay {
    id: SharedString,
    kind: SessionOverlayKind,
    label: Option<SharedString>,
    title: Option<SharedString>,
    subtitle: Option<SharedString>,
    on_primary: Option<PressHandler>,
    on_secondary: Option<PressHandler>,
    ended_style: EndedStyle,
    primary_visible: bool,
    secondary_visible: bool,
    primary_selector: Option<SharedString>,
    subtitle_selector: Option<SharedString>,
    resume_shortcut: Option<SharedString>,
}

impl SessionOverlay {
    pub fn transition(id: impl Into<SharedString>, kind: SessionOverlayKind) -> Self {
        debug_assert!(kind != SessionOverlayKind::Ended);
        Self {
            id: id.into(),
            kind,
            label: None,
            title: None,
            subtitle: None,
            on_primary: None,
            on_secondary: None,
            primary_visible: true,
            secondary_visible: true,
            primary_selector: None,
            subtitle_selector: None,
            resume_shortcut: None,
            ended_style: EndedStyle::Chat,
        }
    }

    pub fn ended(
        id: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
        on_primary: impl Fn(&mut Window, &mut App) + 'static,
        on_secondary: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            kind: SessionOverlayKind::Ended,
            label: None,
            title: None,
            subtitle: Some(subtitle.into()),
            on_primary: Some(Rc::new(on_primary)),
            on_secondary: Some(Rc::new(on_secondary)),
            primary_visible: true,
            secondary_visible: true,
            primary_selector: None,
            subtitle_selector: None,
            resume_shortcut: None,
            ended_style: EndedStyle::Chat,
        }
    }

    pub fn actions_visible(mut self, primary: bool, secondary: bool) -> Self {
        self.primary_visible = primary;
        self.secondary_visible = secondary;
        self
    }

    pub fn primary_debug_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.primary_selector = Some(selector.into());
        self
    }

    pub fn subtitle_debug_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.subtitle_selector = Some(selector.into());
        self
    }

    pub fn resume_shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.resume_shortcut = Some(shortcut.into());
        self
    }

    pub fn mission_paused(mut self) -> Self {
        self.ended_style = EndedStyle::Mission;
        self
    }

    pub fn slot_stopped(mut self) -> Self {
        self.ended_style = EndedStyle::Slot;
        self.title = Some(crate::i18n::text("ui.slot_stopped").into());
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }
}

#[allow(clippy::too_many_arguments)]
fn stopped_chat_card(
    primary_label: SharedString,
    id: SharedString,
    title: SharedString,
    subtitle: SharedString,
    primary: PressHandler,
    shortcut: Option<SharedString>,
    primary_visible: bool,
    primary_selector: Option<SharedString>,
    subtitle_selector: Option<SharedString>,
) -> gpui::AnyElement {
    gpui::container_query(move |size, window, _| {
        let zoom = f32::from(window.rem_size()) / 16.;
        let card_width = (f32::from(size.width) - 32. * zoom).min(360. * zoom);
        let available_height = (f32::from(size.height) - 32. * zoom).max(0.);
        let measure = |text: &str, font_size: f32, weight, wrap: Option<f32>| {
            let mut style = window.text_style();
            style.font_size = rems(font_size / 16.).into();
            style.font_weight = weight;
            let font_size = gpui::px(font_size * zoom);
            let runs = [style.to_run(text.len())];
            let width = f32::from(
                window
                    .text_system()
                    .shape_line(text.to_owned().into(), font_size, &runs, None)
                    .width
                    .ceil(),
            );
            let lines = window
                .text_system()
                .shape_text(
                    text.to_owned().into(),
                    font_size,
                    &runs,
                    wrap.map(gpui::px),
                    None,
                )
                .map_or(1, |lines| {
                    lines
                        .iter()
                        .map(|line| line.wrap_boundaries.len() + 1)
                        .sum()
                });
            (
                width,
                lines,
                f32::from(style.line_height_in_pixels(window.rem_size())).ceil(),
            )
        };
        let (resume_width, _, resume_line) = measure(&primary_label, 14., FontWeight::MEDIUM, None);
        let (shortcut_width, _, shortcut_line) =
            shortcut.as_ref().map_or((0., 0, 0.), |shortcut| {
                measure(shortcut, 11., FontWeight::MEDIUM, None)
            });
        let inline_width = resume_width
            + 44. * zoom
            + if shortcut.is_some() {
                shortcut_width + 6. * zoom
            } else {
                0.
            };
        let shortcut_below = shortcut.is_some() && inline_width > card_width - 48. * zoom - 2.;
        let button_width = if shortcut_below {
            (resume_width + 20. * zoom).max(shortcut_width) + 24. * zoom
        } else {
            inline_width
        };
        let horizontal_padding = ((card_width - button_width - 2.) / 2.).clamp(0., 24. * zoom);
        let content_width = (card_width - horizontal_padding * 2. - 2.).max(1.);
        let (_, title_lines, title_line) =
            measure(&title, 15., FontWeight::SEMIBOLD, Some(content_width));
        let (_, reason_lines, _) = measure(&subtitle, 13., FontWeight::NORMAL, Some(content_width));
        let button_height = if !primary_visible {
            0.
        } else if shortcut_below {
            (resume_line + shortcut_line + 10. * zoom).ceil()
        } else {
            32. * zoom
        };
        let reason_line = 19.5 * zoom;
        let height = |badge: bool, title: bool, reason: usize| {
            let blocks = usize::from(badge)
                + usize::from(title)
                + usize::from(reason > 0)
                + usize::from(primary_visible);
            44. * zoom
                + 2.
                + if badge { 36. * zoom } else { 0. }
                + if title {
                    title_lines as f32 * title_line
                } else {
                    0.
                }
                + reason as f32 * reason_line
                + button_height
                + blocks.saturating_sub(1) as f32 * 10. * zoom
                + if primary_visible && blocks > 1 {
                    4. * zoom
                } else {
                    0.
                }
        };
        let mut badge = true;
        let mut reason_limit = reason_lines;
        let mut show_title = true;
        if height(badge, show_title, reason_limit) > available_height {
            badge = false;
        }
        if height(badge, show_title, reason_limit) > available_height {
            reason_limit = reason_limit.min(2);
        }
        if height(badge, show_title, reason_limit) > available_height {
            reason_limit = reason_limit.min(1);
        }
        if height(badge, show_title, reason_limit) > available_height {
            reason_limit = 0;
        }
        if height(badge, show_title, reason_limit) > available_height {
            show_title = false;
        }
        let padding_scale = if !badge && !show_title && reason_limit == 0 {
            ((available_height - button_height - 2.) / (44. * zoom)).clamp(0., 1.)
        } else {
            1.
        };
        div()
            .size_full()
            .flex()
            .justify_center()
            .items_center()
            .p_4()
            .bg(theme::with_alpha(theme::bg(), 0.7))
            .child(
                div()
                    .debug_selector(|| "SESSION_ENDED_CARD".into())
                    .w(gpui::px(card_width))
                    .max_h(gpui::px(available_height))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .text_center()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .rounded_xl()
                    .pt(gpui::px(24. * zoom * padding_scale))
                    .px(gpui::px(horizontal_padding))
                    .pb(gpui::px(20. * zoom * padding_scale))
                    .gap(rems(10. / 16.))
                    .shadow_lg()
                    .children(badge.then(|| {
                        div()
                            .debug_selector(|| "CHAT_STOPPED_BADGE".into())
                            .size(rems(36. / 16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(theme::raised())
                            .child(
                                svg()
                                    .path("square.svg")
                                    .size(rems(15. / 16.))
                                    .text_color(theme::muted()),
                            )
                    }))
                    .children(show_title.then(|| {
                        div()
                            .debug_selector({
                                let title = title.clone();
                                move || format!("CHAT_STOPPED_TITLE:{title}")
                            })
                            .w_full()
                            .flex_none()
                            .whitespace_normal()
                            .text_size(theme::text_lead())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(title)
                    }))
                    .children((reason_limit > 0).then(|| {
                        div()
                            .debug_selector(|| {
                                subtitle_selector
                                    .as_ref()
                                    .map(ToString::to_string)
                                    .unwrap_or_else(|| "SESSION_ENDED_SUBTITLE".into())
                            })
                            .w_full()
                            .min_w_0()
                            .flex_none()
                            .whitespace_normal()
                            .line_clamp(reason_limit)
                            .text_ellipsis()
                            .font_weight(FontWeight::NORMAL)
                            .text_size(theme::text_body())
                            .line_height(rems(19.5 / 16.))
                            .text_color(theme::muted())
                            .child(subtitle)
                    }))
                    .children(primary_visible.then(|| {
                        div()
                            .debug_selector(|| "SESSION_ENDED_ACTIONS".into())
                            .flex_none()
                            .when(badge || show_title || reason_limit > 0, |actions| {
                                actions.pt_1()
                            })
                            .child(
                                Button::new(
                                    SharedString::from(format!("{id}-resume")),
                                    primary_label.clone(),
                                )
                                .icon("play.svg")
                                .variant(ButtonVariant::Primary)
                                .shortcut_below(shortcut_below)
                                .when_some(shortcut, |button, shortcut| {
                                    button.shortcut(shortcut).shortcut_opacity(0.55)
                                })
                                .when_some(primary_selector, |button, selector| {
                                    button.debug_selector(selector)
                                })
                                .on_press(move |window, cx| primary(window, cx)),
                            )
                    })),
            )
    })
    .absolute()
    .inset_0()
    .into_any_element()
}

impl RenderOnce for SessionOverlay {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        match self.kind {
            SessionOverlayKind::Starting | SessionOverlayKind::Resuming => {
                let label = self.label.unwrap_or_else(|| {
                    if self.kind == SessionOverlayKind::Starting {
                        crate::i18n::text("ui.starting_chat").into()
                    } else {
                        crate::i18n::text("ui.resuming").into()
                    }
                });
                div()
                    .absolute()
                    .inset_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .rounded_full()
                            .border_1()
                            .border_color(theme::with_alpha(theme::info(), 0.4))
                            .bg(theme::with_alpha(theme::info(), 0.1))
                            .px_4()
                            .py_2()
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::info())
                            .shadow_lg()
                            .child(spinner(
                                SharedString::from(format!("{id}-spinner")),
                                16.,
                                theme::info(),
                            ))
                            .child(label),
                    )
                    .into_any_element()
            }
            SessionOverlayKind::Archiving => div()
                .debug_selector(|| "SESSION_ARCHIVING".into())
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::with_alpha(theme::bg(), 0.95))
                .child(
                    div()
                        .h(rems(30. / 16.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_full()
                        .border_1()
                        .border_color(theme::with_alpha(theme::warning(), 0.4))
                        .bg(theme::with_alpha(theme::warning(), 0.15))
                        .px_3()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_body())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::warning())
                        .child(
                            div()
                                .size(rems(8. / 16.))
                                .rounded_full()
                                .bg(theme::warning())
                                .with_animation(
                                    SharedString::from(format!("{id}-pulse")),
                                    Animation::new(Duration::from_millis(800)).repeat(),
                                    |dot, delta| {
                                        let opacity = if delta <= 0.5 {
                                            0.4 + delta * 1.2
                                        } else {
                                            1.6 - delta * 1.2
                                        };
                                        dot.opacity(opacity)
                                    },
                                ),
                        )
                        .child(crate::i18n::text("ui.archiving")),
                )
                .into_any_element(),
            SessionOverlayKind::Ended => {
                let primary = self.on_primary.expect("ended overlay primary action");
                let secondary = self.on_secondary.expect("ended overlay secondary action");
                let title = self
                    .title
                    .unwrap_or_else(|| crate::i18n::text("ui.chat_stopped").into());
                let subtitle = self.subtitle.expect("ended overlay subtitle");
                let slot = self.ended_style == EndedStyle::Slot;
                if self.ended_style == EndedStyle::Chat {
                    return stopped_chat_card(
                        self.label
                            .unwrap_or_else(|| crate::i18n::text("ui.resume").into()),
                        id,
                        title,
                        subtitle,
                        primary,
                        self.resume_shortcut,
                        self.primary_visible,
                        self.primary_selector,
                        self.subtitle_selector,
                    );
                }
                let primary_click = Rc::clone(&primary);
                let secondary_click = Rc::clone(&secondary);
                let header = {
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(10. / 16.))
                        .child(
                            svg()
                                .flex_none()
                                .path("pause.svg")
                                .size(rems(1.))
                                .text_color(theme::faint()),
                        )
                        .child(
                            div()
                                .text_size(theme::text_lead())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::text())
                                .child(title),
                        )
                        .into_any_element()
                };
                let actions = {
                    div()
                        .debug_selector(|| "SESSION_ENDED_ACTIONS".into())
                        .flex()
                        .items_center()
                        .gap_2()
                        .children(self.primary_visible.then(|| {
                            Button::new(
                                SharedString::from(format!("{id}-resume")),
                                if slot {
                                    crate::i18n::text("ui.resume_slot")
                                } else {
                                    crate::i18n::text("ui.resume")
                                },
                            )
                            .icon("play.svg")
                            .variant(ButtonVariant::Primary)
                            .when_some(self.primary_selector.clone(), |button, selector| {
                                button.debug_selector(selector)
                            })
                            .on_press(move |window, cx| primary_click(window, cx))
                        }))
                        .children(self.secondary_visible.then(|| {
                            Button::new(
                                SharedString::from(format!("{id}-archive")),
                                if slot {
                                    crate::i18n::text("ui.restart_slot")
                                } else {
                                    crate::i18n::text("ui.archive")
                                },
                            )
                            .icon(if slot {
                                "rotate-ccw.svg"
                            } else {
                                "archive.svg"
                            })
                            .on_press(move |window, cx| secondary_click(window, cx))
                        }))
                        .into_any_element()
                };
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .justify_center()
                    .px_4()
                    .bg(theme::with_alpha(theme::bg(), 0.7))
                    .items_end()
                    .pb(rems(56. / 16.))
                    .child(
                        div()
                            .debug_selector(|| "SESSION_ENDED_CARD".into())
                            .w_full()
                            .flex()
                            .flex_col()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::panel())
                            .max_w(rems(672. / 16.))
                            .gap(rems(14. / 16.))
                            .rounded_xl()
                            .p_5()
                            .shadow_lg()
                            .child(header)
                            .child(
                                div()
                                    .debug_selector(|| "SESSION_ENDED_SUBTITLE".into())
                                    .w_full()
                                    .min_w_0()
                                    .whitespace_normal()
                                    .text_size(theme::text_body())
                                    .line_height(rems(18. / 16.))
                                    .text_color(theme::muted())
                                    .child(subtitle),
                            )
                            .child(actions),
                    )
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, Context, Render, TestAppContext, VisualTestContext};

    struct EndedOverlayTest {
        shortcut: Option<&'static str>,
        title: Option<&'static str>,
        hits: Option<std::rc::Rc<std::cell::Cell<usize>>>,
    }

    impl Render for EndedOverlayTest {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let hits = self.hits.clone();
            div().size_full().relative().overflow_hidden().child(SessionOverlay::ended(
                "test-ended",
                "The PTY is closed. Resume to start a fresh agent process — there's no saved conversation to pick up from this row.",
                move |_, _| { if let Some(hits) = &hits { hits.set(hits.get() + 1); } },
                |_, _| {},
            ).primary_debug_selector("CHAT_RESUME").when_some(self.shortcut, |overlay, shortcut| overlay.resume_shortcut(shortcut)).when_some(self.title, |overlay, title| overlay.title(title)))
        }
    }

    #[test]
    fn centered_chat_card_preserves_bottom_padding() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, _| {
            window.set_rem_size(px(20.8));
            EndedOverlayTest {
                shortcut: None,
                title: None,
                hits: None,
            }
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let card = window
            .debug_bounds("SESSION_ENDED_CARD")
            .expect("card bounds");
        let actions = window
            .debug_bounds("SESSION_ENDED_ACTIONS")
            .expect("actions bounds");
        assert_eq!(card.bottom() - actions.bottom(), px(27.));
    }

    #[test]
    fn ended_overlay_subtitle_wraps_inside_the_card() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, _| {
            window.set_rem_size(px(20.8));
            EndedOverlayTest {
                shortcut: None,
                title: None,
                hits: None,
            }
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let card = window
            .debug_bounds("SESSION_ENDED_CARD")
            .expect("card bounds");
        let subtitle = window
            .debug_bounds("SESSION_ENDED_SUBTITLE")
            .expect("subtitle bounds");
        let line_height = px(20.8 * 19.5 / 16.);
        assert!(
            subtitle.size.height >= line_height * 2.,
            "subtitle did not wrap: {subtitle:?}"
        );
        assert!(subtitle.right() <= card.right(), "{subtitle:?} vs {card:?}");
    }

    #[test]
    fn stopped_chat_card_centers_and_fits_narrow_panes_with_one_resume_action() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for zoom in [1., 1.3] {
            for width in [240., 360., 800.] {
                let mut cx = TestAppContext::single();
                let host = cx.add_window(|window, _| {
                    window.set_rem_size(px(16. * zoom));
                    EndedOverlayTest {
                        shortcut: Some("F8"),
                        title: None,
                        hits: None,
                    }
                });
                let mut visual = VisualTestContext::from_window(host.into(), &cx);
                visual.simulate_resize(gpui::size(px(width * zoom), px(800. * zoom)));
                visual.run_until_parked();
                let card = visual.debug_bounds("SESSION_ENDED_CARD").unwrap();
                let badge = visual.debug_bounds("CHAT_STOPPED_BADGE").unwrap();
                let title = visual
                    .debug_bounds("CHAT_STOPPED_TITLE:Chat stopped")
                    .unwrap();
                let subtitle = visual.debug_bounds("SESSION_ENDED_SUBTITLE").unwrap();
                let actions = visual.debug_bounds("SESSION_ENDED_ACTIONS").unwrap();
                let resume = visual.debug_bounds("CHAT_RESUME").unwrap();
                assert!((card.center().x - px(width * zoom / 2.)).abs() < px(1.));
                assert!((card.center().y - px(400. * zoom)).abs() < px(1.));
                assert!((card.size.width - px((width - 32.).min(360.) * zoom)).abs() < px(1.));
                assert!((badge.size.width - px(36. * zoom)).abs() < px(1.));
                assert!((badge.top() - card.top() - px(24. * zoom + 1.)).abs() < px(1.));
                assert!((title.top() - badge.bottom() - px(10. * zoom)).abs() < px(1.));
                assert!((subtitle.top() - title.bottom() - px(10. * zoom)).abs() < px(1.));
                assert!((actions.top() - subtitle.bottom() - px(10. * zoom)).abs() < px(1.));
                assert!((resume.top() - actions.top() - px(4. * zoom)).abs() < px(1.));
                assert!((resume.center().x - card.center().x).abs() < px(1.));
                assert_eq!(actions.size.width, resume.size.width);
                assert!(visual.debug_bounds("BUTTON_SHORTCUT:F8").is_some());
            }
        }
        let mut cx = TestAppContext::single();
        let host = cx.add_window(|_, _| EndedOverlayTest {
            shortcut: None,
            title: Some("Custom reason"),
            hits: None,
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        assert!(visual
            .debug_bounds("CHAT_STOPPED_TITLE:Custom reason")
            .is_some());
        assert!(visual.debug_bounds("BUTTON_SHORTCUT:F8").is_none());
    }

    #[test]
    fn short_chat_cards_keep_resume_clickable_with_each_platform_shortcut() {
        use std::{cell::Cell, rc::Rc};
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for zoom in [1., 1.3] {
            for width in [160., 240., 360., 800.] {
                for height in [96., 118., 160., 240.] {
                    for shortcut in [Some("⇧⌘R"), Some("Ctrl+Shift+R"), Some("F8"), None] {
                        let mut cx = TestAppContext::single();
                        let hits = Rc::new(Cell::new(0));
                        let host = cx.add_window(|window, _| {
                            window.set_rem_size(px(16. * zoom));
                            EndedOverlayTest {
                                shortcut,
                                title: None,
                                hits: Some(hits.clone()),
                            }
                        });
                        let mut visual = VisualTestContext::from_window(host.into(), &cx);
                        visual.simulate_resize(gpui::size(px(width * zoom), px(height * zoom)));
                        visual.run_until_parked();
                        let card = visual.debug_bounds("SESSION_ENDED_CARD").unwrap();
                        let button = visual.debug_bounds("CHAT_RESUME").unwrap();
                        let case = format!("{width}x{height}, {zoom}, {shortcut:?}: card={card:?}, button={button:?}");
                        assert!(
                            card.left() >= px(16. * zoom) - px(1.)
                                && card.right() <= px((width - 16.) * zoom) + px(1.),
                            "{case}"
                        );
                        assert!(
                            card.top() >= px(16. * zoom) - px(1.)
                                && card.bottom() <= px((height - 16.) * zoom) + px(1.),
                            "{case}"
                        );
                        assert!(
                            button.left() >= card.left() && button.right() <= card.right(),
                            "{case}"
                        );
                        assert!(
                            button.top() >= card.top() && button.bottom() <= card.bottom(),
                            "{case}"
                        );
                        if let Some(shortcut) = shortcut {
                            let key = visual
                                .debug_bounds(match shortcut {
                                    "⇧⌘R" => "BUTTON_SHORTCUT:⇧⌘R",
                                    "Ctrl+Shift+R" => "BUTTON_SHORTCUT:Ctrl+Shift+R",
                                    _ => "BUTTON_SHORTCUT:F8",
                                })
                                .unwrap();
                            assert!(
                                key.left() >= button.left() && key.right() <= button.right(),
                                "{case}, key={key:?}"
                            );
                            assert!(
                                key.top() >= button.top() && key.bottom() <= button.bottom(),
                                "{case}, key={key:?}"
                            );
                        }
                        if let Some(label) = visual.debug_bounds("BUTTON_LABEL_ROW") {
                            assert!(
                                label.left() >= button.left() && label.right() <= button.right(),
                                "{case}, label={label:?}"
                            );
                        }
                        visual.simulate_click(button.center(), Default::default());
                        assert_eq!(hits.get(), 1, "{case}");
                    }
                }
            }
        }
    }

    #[test]
    fn short_chat_card_drops_badge_then_reason_then_title() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for zoom in [1., 1.3] {
            for (height, badge, reason_lines, title) in [
                (300., true, 0, true),
                (240., false, 0, true),
                (210., false, 2, true),
                (190., false, 1, true),
                (160., false, 0, true),
                (118., false, 0, false),
            ] {
                let mut cx = TestAppContext::single();
                let host = cx.add_window(|window, _| {
                    window.set_rem_size(px(16. * zoom));
                    EndedOverlayTest {
                        shortcut: Some("F8"),
                        title: None,
                        hits: None,
                    }
                });
                let mut visual = VisualTestContext::from_window(host.into(), &cx);
                visual.simulate_resize(gpui::size(px(360. * zoom), px(height * zoom)));
                visual.run_until_parked();
                assert_eq!(
                    visual.debug_bounds("CHAT_STOPPED_BADGE").is_some(),
                    badge,
                    "{height}, {zoom}"
                );
                assert_eq!(
                    visual
                        .debug_bounds("CHAT_STOPPED_TITLE:Chat stopped")
                        .is_some(),
                    title,
                    "{height}, {zoom}"
                );
                let reason = visual.debug_bounds("SESSION_ENDED_SUBTITLE");
                if height <= 160. {
                    assert!(reason.is_none());
                } else if reason_lines > 0 {
                    assert!(
                        (reason.unwrap().size.height - px(reason_lines as f32 * 19.5 * zoom)).abs()
                            < px(1.)
                    );
                }
            }
        }
    }

    struct MissionPausedOverlayTest;
    impl Render for MissionPausedOverlayTest {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                SessionOverlay::ended(
                    "test-mission",
                    "All slots are paused.",
                    |_, _| {},
                    |_, _| {},
                )
                .mission_paused()
                .title("Mission paused"),
            )
        }
    }

    #[test]
    fn mission_wide_card_keeps_its_bottom_placement_and_two_actions() {
        let mut cx = TestAppContext::single();
        let host = cx.add_window(|_, _| MissionPausedOverlayTest);
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(gpui::size(px(1000.), px(800.)));
        visual.run_until_parked();
        let card = visual.debug_bounds("SESSION_ENDED_CARD").unwrap();
        let actions = visual.debug_bounds("SESSION_ENDED_ACTIONS").unwrap();
        assert_eq!(card.size.width, px(672.));
        assert_eq!(card.bottom(), px(744.));
        assert_eq!(card.bottom() - actions.bottom(), px(21.));
        assert!(actions.size.width > px(150.));
        assert!(visual.debug_bounds("CHAT_STOPPED_BADGE").is_none());
    }

    struct SlotStoppedOverlayTest;
    impl Render for SlotStoppedOverlayTest {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(SessionOverlay::ended(
                "test-slot-stopped",
                "@worker's PTY is closed; 2 other slots are still running. Resume continues its conversation where it left off. Restart discards it and starts over with the brief, the same first turn a cold start gives the slot.",
                |_, _| {}, |_, _| {},
            ).slot_stopped())
        }
    }

    #[test]
    fn stopped_slot_card_wraps_copy_and_keeps_bottom_docked_actions() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, _| {
            window.set_rem_size(px(20.8));
            SlotStoppedOverlayTest
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let card = window.debug_bounds("SESSION_ENDED_CARD").unwrap();
        let subtitle = window.debug_bounds("SESSION_ENDED_SUBTITLE").unwrap();
        let actions = window.debug_bounds("SESSION_ENDED_ACTIONS").unwrap();
        assert!(subtitle.right() <= card.right());
        assert!(subtitle.bottom() <= actions.top());
        assert_eq!(card.bottom() - actions.bottom(), px(27.));
    }
}
