use super::logic::crew_missions;
use super::logic::mission_duration;
use super::logic::missions_footer;
use super::logic::missions_header;
use super::logic::short_date;
use super::logic::MISSIONS_SHOWN;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, Context, Div, Entity, FontWeight, KeyDownEvent, SharedString,
};
use runner_app::ui::focus_ring;
use runner_backend::model::{Crew, Mission, MissionStatus};

use crate::surfaces::profile_page::{
    caption, card, card_column, card_meta, clamped_markdown, markdown_editor_body,
    markdown_mode_switch, prompt_meta,
};
use crate::*;

const CONVENTIONS_CAPTION: &str =
    "Added to every slot's prompt when this crew runs a mission. Direct chats ignore it.";

impl NativeRoot {
    /// The right column: the team conventions, their caption, and the crew's
    /// missions.
    pub(super) fn render_crew_cards(&self, crew: Option<&Crew>, cx: &mut Context<Self>) -> Div {
        let editing = self.crew_surfaces.editor.edit.is_some() || crew.is_none();
        card_column()
            .when(cfg!(test), |column| {
                column.debug_selector(|| "CREW_PAGE_CARDS".into())
            })
            .child(if editing {
                self.render_conventions_editor(cx)
            } else {
                self.render_conventions_card(crew.unwrap(), cx)
            })
            .child(caption(CONVENTIONS_CAPTION))
            .children(crew.map(|crew| {
                div()
                    .mt(rems(20. / 16.))
                    .when(editing, |missions| missions.opacity(0.4))
                    .child(self.render_missions_card(&crew.id, !editing, cx))
            }))
    }

    fn render_conventions_card(&self, crew: &Crew, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let conventions = crew
            .system_prompt_addendum
            .as_deref()
            .filter(|text| !text.trim().is_empty());
        let body = match conventions {
            None => div()
                .text_size(theme::text_body())
                .italic()
                .text_color(theme::faint())
                .child("No team conventions yet. Edit the crew to add them.")
                .into_any_element(),
            Some(text) => clamped_markdown(
                &format!("crew-conventions-{}", crew.id),
                text,
                self.crew_surfaces.editor.conventions_expanded,
                theme::panel(),
                "CREW_CONVENTIONS",
                Rc::new(move |_, cx| {
                    root.update(cx, |this, cx| {
                        let editor = &mut this.crew_surfaces.editor;
                        editor.conventions_expanded = !editor.conventions_expanded;
                        cx.notify();
                    });
                }),
                cx.entity_id(),
                cx,
            ),
        };
        conventions_card(card_meta(conventions.map(prompt_meta)))
            .child(div().px_5().py_4().child(body))
            .into_any_element()
    }

    fn render_conventions_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let editor = &self.crew_surfaces.editor;
        let conventions = if self.route == AppRoute::NewCrew
            || (self.route == AppRoute::Settings && self.settings_return_route == AppRoute::NewCrew)
        {
            &self
                .crew_surfaces
                .create
                .as_ref()
                .expect("creating crew form")
                .conventions
        } else {
            &editor.edit.as_ref().expect("crew edit form").conventions
        };
        let mode_focus = self
            .crew_surfaces
            .create
            .as_ref()
            .map(|form| &form.mode_focus)
            .unwrap_or_else(|| &editor.edit.as_ref().unwrap().mode_focus);
        let preview = editor.conventions_preview;
        let draft = conventions.read(cx).text();
        let meta = (self.crew_surfaces.create.is_some() || !draft.trim().is_empty())
            .then(|| prompt_meta(draft));
        let root = cx.entity();
        conventions_card(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .child(markdown_mode_switch(
                    "crew-conventions-mode",
                    preview,
                    mode_focus,
                    Rc::new(move |preview, cx| {
                        root.update(cx, |this, cx| {
                            if this.crew_surfaces.editor.conventions_preview != preview {
                                this.crew_surfaces.editor.conventions_preview = preview;
                                cx.notify();
                            }
                        });
                    }),
                ))
                .child(card_meta(meta))
                .into_any_element(),
        )
        .h(rems(496. / 16.))
        .child(markdown_editor_body(
            "crew-conventions-draft",
            conventions.clone(),
            preview,
            "CREW_CONVENTIONS",
            cx.entity_id(),
            cx,
        ))
        .into_any_element()
    }

    fn render_missions_card(
        &self,
        crew_id: &str,
        interactive: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let store = self.app_store.read(cx);
        let missions = crew_missions(&store.missions, crew_id);
        let expanded = self.crew_surfaces.editor.missions_expanded;
        let footer = missions_footer(missions.len(), expanded);
        let shown = if expanded {
            missions.len()
        } else {
            missions.len().min(MISSIONS_SHOWN)
        };
        let now = chrono::Utc::now();
        let local_now = chrono::Local::now();
        let root = cx.entity();
        let rows = missions[..shown]
            .iter()
            .map(|mission| mission_row(mission, now, &local_now, interactive, root.clone()))
            .collect::<Vec<_>>();
        let body = if missions.is_empty() {
            div()
                .px_4()
                .py_3()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child("No missions yet. Start one to see it here.")
                .into_any_element()
        } else {
            div()
                .when(cfg!(test), |rows| {
                    rows.debug_selector(|| "CREW_MISSION_ROWS".into())
                })
                .flex()
                .flex_col()
                .children(rows)
                .into_any_element()
        };
        let toggle_root = root;
        card(
            "flag.svg",
            "Missions",
            Some(missions_header(&missions)),
            div().into_any_element(),
        )
        .when(cfg!(test), |card| {
            card.debug_selector(|| "CREW_MISSIONS_CARD".into())
        })
        .child(body)
        .children(footer.map(|label| {
            let toggle = move |cx: &mut gpui::App| {
                toggle_root.update(cx, |this, cx| {
                    let editor = &mut this.crew_surfaces.editor;
                    editor.missions_expanded = !editor.missions_expanded;
                    cx.notify();
                });
            };
            let key_toggle = toggle.clone();
            div()
                .id("crew-missions-toggle")
                .when(cfg!(test), |footer| {
                    footer.debug_selector(|| "CREW_MISSIONS_TOGGLE".into())
                })
                .flex()
                .items_center()
                .gap_1()
                .px_4()
                .py(rems(8. / 16.))
                .border_t_1()
                .border_color(theme::border())
                .text_size(theme::text_ui())
                .text_color(theme::muted())
                .when(interactive, |footer| {
                    footer
                        .tab_index(0)
                        .cursor_pointer()
                        .hover(|footer| footer.text_color(theme::text()))
                        .focus_visible(|footer| {
                            footer
                                .text_color(theme::text())
                                .shadow(focus_ring(theme::border_strong()))
                        })
                        .on_click(move |_, _, cx| toggle(cx))
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_toggle(cx);
                            }
                        })
                })
                .child(label)
                .child(
                    svg()
                        .flex_none()
                        .path(if expanded {
                            "chevron-up.svg"
                        } else {
                            "chevron-down.svg"
                        })
                        .size(rems(12. / 16.))
                        .text_color(theme::faint()),
                )
        }))
        .into_any_element()
    }
}

fn conventions_card(header_right: AnyElement) -> Div {
    card("file-text.svg", "Team conventions", None, header_right).when(cfg!(test), |card| {
        card.debug_selector(|| "CREW_CONVENTIONS_CARD".into())
    })
}

fn mission_row(
    mission: &Mission,
    now: runner_backend::model::Timestamp,
    local_now: &chrono::DateTime<chrono::Local>,
    interactive: bool,
    root: Entity<NativeRoot>,
) -> AnyElement {
    let id = mission.id.clone();
    let key_id = id.clone();
    let key_root = root.clone();
    let status = match mission.status {
        MissionStatus::Completed => svg()
            .flex_none()
            .path("circle-check.svg")
            .size(rems(14. / 16.))
            .text_color(theme::faint())
            .into_any_element(),
        MissionStatus::Aborted => svg()
            .flex_none()
            .path("circle-x.svg")
            .size(rems(14. / 16.))
            .text_color(theme::danger())
            .into_any_element(),
        MissionStatus::Running => div()
            .flex_none()
            .size(rems(14. / 16.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(rems(7. / 16.))
                    .rounded_full()
                    .bg(theme::accent()),
            )
            .into_any_element(),
    };
    let tag = match mission.status {
        MissionStatus::Aborted => Some(("aborted", theme::danger())),
        MissionStatus::Running => Some(("live", theme::accent())),
        MissionStatus::Completed => None,
    };
    div()
        .id(SharedString::from(format!("crew-mission-{id}")))
        .flex()
        .items_center()
        .gap(rems(10. / 16.))
        .px_4()
        .py(rems(9. / 16.))
        .border_t_1()
        .border_color(theme::border())
        .text_size(theme::text_body())
        .child(status)
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text())
                        .child(mission.title.clone()),
                )
                .children(tag.map(|(tag, color)| {
                    div()
                        .flex_none()
                        .text_size(theme::text_ui())
                        .text_color(color)
                        .child(tag)
                })),
        )
        .child(
            div()
                .flex_none()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_caption())
                .text_color(theme::faint())
                .child(mission_duration(mission, now)),
        )
        .child(
            div()
                .flex_none()
                .w(rems(56. / 16.))
                .flex()
                .justify_end()
                .text_size(theme::text_ui())
                .text_color(theme::muted())
                .child(short_date(
                    &mission.started_at.with_timezone(&chrono::Local),
                    local_now,
                )),
        )
        .when(interactive, |row| {
            row.tab_index(0)
                .cursor_pointer()
                .hover(|row| row.bg(theme::raised()))
                .focus_visible(|row| row.bg(theme::raised()))
                .on_click(move |_, window, cx| {
                    let id = id.clone();
                    root.update(cx, |this, cx| this.open_mission(id, window, cx));
                })
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        let id = key_id.clone();
                        key_root.update(cx, |this, cx| this.open_mission(id, window, cx));
                    }
                })
        })
        .into_any_element()
}
