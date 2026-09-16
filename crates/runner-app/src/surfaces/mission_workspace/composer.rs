use std::rc::Rc;

use gpui::prelude::*;
use gpui::{canvas, div, px, rems, AnyElement, CursorStyle, FontWeight, Window};

use super::*;
use crate::surfaces::mission_composer::{
    key_down as composer_key_down, mention_options, select_target as select_composer_target,
    ComposerPost, ComposerState, RosterEntry as ComposerRosterEntry,
};
use crate::*;

impl MissionWorkspace {
    pub(super) fn mission_composer_roster(&self) -> Vec<ComposerRosterEntry> {
        self.roster
            .iter()
            .map(|member| ComposerRosterEntry {
                handle: member.slot.slot_handle.clone(),
                role: member.role.handle.clone(),
                runtime: member
                    .slot
                    .runtime_override
                    .clone()
                    .unwrap_or_else(|| member.role.runtime.clone()),
            })
            .collect()
    }

    pub(super) fn render_mission_composer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let roster = self.mission_composer_roster();
        let options = mention_options(&self.composer, &roster);
        let picker_open = !options.is_empty();
        let active_index = self
            .composer
            .active_index
            .min(options.len().saturating_sub(1));
        let target = self.composer.target.clone();
        let posting = self.composer_posting;
        let can_send = !posting && !self.composer.draft.trim().is_empty();
        let root = cx.entity();
        let anchor_root = root.clone();
        let target_root = root.clone();
        let send_root = root.clone();
        let input = self.composer_input.clone();
        let mut field = div()
            .id("mission-composer-field")
            .relative()
            .flex()
            .items_center()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .px_4()
            .py_3()
            .children(target.map(|target| {
                let clear_root = target_root.clone();
                div()
                    .id("mission-composer-target")
                    .flex_none()
                    .rounded_sm()
                    .bg(theme::with_alpha(theme::accent(), 0.15))
                    .px_1()
                    .py(rems(2. / 16.))
                    .cursor_pointer()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_ui())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::accent())
                    .on_click(move |_, window, cx| {
                        clear_root.update(cx, |this, cx| {
                            this.clear_mission_composer_target(window, cx)
                        });
                    })
                    .child(format!("@{target}"))
            }))
            .child(div().min_w(px(0.)).flex_1().child(input))
            .child(
                div()
                    .id("mission-composer-send")
                    .flex_none()
                    .rounded_md()
                    .bg(theme::accent())
                    .map(|element| {
                        #[cfg(test)]
                        let element = crate::theme_snapshot::record_fill("MISSION_ACCENT", element);
                        element
                    })
                    .px_3()
                    .py_1()
                    .opacity(if can_send { 1. } else { 0.5 })
                    .cursor(if can_send {
                        CursorStyle::PointingHand
                    } else {
                        CursorStyle::OperationNotAllowed
                    })
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(theme::text_ui())
                    .text_color(theme::accent_ink())
                    .on_click(move |_, window, cx| {
                        send_root.update(cx, |this, cx| {
                            this.post_current_mission_composer(window, cx)
                        });
                    })
                    .child(if posting { "Sending…" } else { "Send" }),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        anchor_root.update(cx, |this, _| this.composer_anchor = Some(bounds));
                    },
                )
                .absolute()
                .inset_0(),
            );

        if let (true, Some(anchor)) = (picker_open, self.composer_anchor) {
            let picker_root = root.clone();
            let rows = options.into_iter().enumerate().map(|(index, entry)| {
                let option_root = picker_root.clone();
                let handle = entry.handle.clone();
                div()
                    .id(("mission-composer-option", index))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(if index == active_index {
                        theme::border_strong()
                    } else {
                        gpui::transparent_black()
                    })
                    .bg(if index == active_index {
                        theme::raised()
                    } else {
                        gpui::transparent_black()
                    })
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|row| row.bg(theme::raised()))
                    .on_click(move |_, window, cx| {
                        option_root.update(cx, |this, cx| {
                            this.select_mission_composer_target(handle.clone(), window, cx)
                        });
                    })
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::accent())
                            .child(format!("@{}", entry.handle)),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .truncate()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(format!("{} · {}", entry.role, entry.runtime)),
                    )
                    .children((index == active_index).then(|| {
                        div()
                            .ml_auto()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_caption())
                            .text_color(theme::faint())
                            .child("↵")
                    }))
            });
            let menu = div()
                .id("mission-composer-roster")
                .relative()
                .max_h(rems(240. / 16.))
                .overflow_hidden()
                .rounded_lg()
                .border_1()
                .border_color(theme::border_strong())
                .bg(theme::panel())
                .p_1()
                .shadow_xl()
                .child(
                    div()
                        .px_2()
                        .pt_1()
                        .pb(rems(2. / 16.))
                        .text_size(theme::text_caption())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::faint())
                        .child("ROSTER"),
                )
                .child(
                    div()
                        .id("mission-composer-roster-scroll")
                        .max_h(rems(205. / 16.))
                        .overflow_y_scroll()
                        .children(rows),
                )
                .into_any_element();
            let dismiss_root = root.clone();
            let dismiss: runner_app::ui::menu::DismissHandler = Rc::new(move |_, cx| {
                dismiss_root.update(cx, |this, cx| {
                    this.composer.picker_dismissed = true;
                    cx.notify();
                });
            });
            field = field.child(runner_app::ui::menu::popup_layer(
                anchor,
                window,
                px(380. * self.settings(cx).app_zoom),
                menu,
                dismiss,
            ));
        }

        div()
            .flex_none()
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .px(rems(40. / 16.))
            .pt(rems(14. / 16.))
            .pb_5()
            .child(field)
            .into_any_element()
    }

    pub(super) fn on_mission_composer_key_down(
        &mut self,
        key: &str,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let transition =
            composer_key_down(&self.composer, &self.mission_composer_roster(), key, shift);
        if !transition.prevent_default {
            return;
        }
        let draft_changed = transition.state.draft != self.composer.draft;
        self.composer = transition.state;
        if draft_changed {
            let draft = self.composer.draft.clone();
            self.composer_input
                .update(cx, |input, input_cx| input.reset(draft, input_cx));
        }
        if let Some(post) = transition.post {
            self.post_mission_composer(post, window, cx);
        } else {
            self.composer_input.read(cx).focus_handle().focus(window);
            cx.notify();
        }
    }

    fn select_mission_composer_target(
        &mut self,
        handle: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_posting || self.secondary_state(cx).secondary {
            return;
        }
        self.composer = select_composer_target(handle);
        self.composer_input
            .update(cx, |input, input_cx| input.reset("", input_cx));
        self.composer_input.read(cx).focus_handle().focus(window);
        cx.notify();
    }

    fn clear_mission_composer_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_posting || self.secondary_state(cx).secondary {
            return;
        }
        self.composer.target = None;
        self.composer_input.read(cx).focus_handle().focus(window);
        cx.notify();
    }

    fn post_current_mission_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.draft.trim().to_owned();
        if text.is_empty() {
            return;
        }
        self.post_mission_composer(
            ComposerPost {
                text,
                to: self.composer.target.clone(),
            },
            window,
            cx,
        );
    }

    fn post_mission_composer(
        &mut self,
        post: ComposerPost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mission_id) = self.mission_id.clone() else {
            return;
        };
        if self.composer_posting || self.secondary_state(cx).secondary {
            return;
        }
        self.composer_posting = true;
        self.composer_input
            .update(cx, |input, input_cx| input.set_disabled(true, input_cx));
        cx.notify();
        let generation = self.generation;
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_post_human_message_impl(
                &core,
                runner_backend::ops::mission::PostHumanMessageInput {
                    mission_id: mission_id.clone(),
                    text: post.text,
                    to: post.to,
                },
            )
            .await
            .map(|_| mission_id)
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let current = result
                    .as_ref()
                    .ok()
                    .is_some_and(|mission_id| this.is_current(mission_id, generation))
                    || result.is_err() && this.generation == generation;
                if !current {
                    return;
                }
                this.composer_posting = false;
                this.composer_input
                    .update(cx, |input, input_cx| input.set_disabled(false, input_cx));
                match result {
                    Ok(_) => {
                        this.composer = ComposerState::default();
                        this.composer_input
                            .update(cx, |input, input_cx| input.reset("", input_cx));
                    }
                    Err(error) => {
                        this.error = Some(action_failure("send the mission message", error));
                    }
                }
                this.composer_input.read(cx).focus_handle().focus(window);
                cx.notify();
            });
        })
        .detach();
    }
}
