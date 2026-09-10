use runner_backend::model::Runtime;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, Context, Entity, EventEmitter, FocusHandle, FontWeight,
    KeyDownEvent, Render, ScrollHandle, Subscription, Window,
};
use runner_app::ui::{
    Badge, Button, ButtonSize, ButtonVariant, ConfirmDialog, IconButton, IconButtonSize, Modal,
    OverlayWidth, PaneHeader, Scrollbar, SelectOption, SettingsCard, StyledSelect, TextField,
    Toggle, Tone, Tooltip,
};
use runner_backend::ops::skills;
use runner_backend::router::runtime::runtime_display_name;
use runner_backend::skills::{parse_skill_document, GlobalState, SkillCatalog, SkillEntry};

use crate::app_store::AppStore;
use crate::surfaces::mission_markdown;
use crate::theme;

const CLAUDE_CAPTION: &str = "Toggles hide a skill from every new Claude Code session, inside Runner or not; the only write is the skillOverrides key in ~/.claude/settings.json. Click a row to read a skill, hover it to edit. Bundled skills (code-review, loop, …) and project skills always load and are not listed.";
const CODEX_CAPTION: &str = "Toggles hide a skill from every new Codex session, inside Runner or not; the only write is a [[skills.config]] entry in ~/.codex/config.toml. Click a row to read a skill, hover it to edit. Codex's system skills (~/.codex/skills/.system) and plugin skills always load and are not listed.";

#[derive(Clone, Debug, PartialEq, Eq)]
struct SkillBadge {
    label: String,
    tone: Tone,
    hint: Option<String>,
}

fn skill_badges(entry: &SkillEntry) -> Vec<SkillBadge> {
    let mut badges = Vec::new();
    for (show, label) in [(entry.manual, "manual"), (entry.hidden, "hidden")] {
        if show {
            badges.push(SkillBadge {
                label: label.into(),
                tone: Tone::Warning,
                hint: None,
            });
        }
    }
    if let Some(target) = &entry.symlink {
        badges.push(SkillBadge {
            label: "symlink".into(),
            tone: Tone::Muted,
            hint: Some(target.display().to_string()),
        });
    }
    if let GlobalState::Other(value) = &entry.global {
        badges.push(SkillBadge {
            label: value.clone(),
            tone: Tone::Warning,
            hint: None,
        });
    }
    if let Some(problem) = &entry.problem {
        badges.push(SkillBadge {
            label: "problem".into(),
            tone: Tone::Danger,
            hint: Some(problem.clone()),
        });
    }
    badges
}

fn first_sentence(description: &str) -> String {
    let text = description.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = text
        .char_indices()
        .find_map(|(i, c)| {
            let next = i + c.len_utf8();
            (matches!(c, '.' | '!' | '?' | '。' | '！' | '？')
                && (c.len_utf8() > 1 || text[next..].starts_with(' ') || next == text.len()))
            .then_some(next)
        })
        .unwrap_or(text.len());
    text[..end].into()
}

fn matches_search(entry: &SkillEntry, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    entry.name.to_lowercase().contains(&query) || entry.description.to_lowercase().contains(&query)
}

fn catalog_meta(catalog: &SkillCatalog) -> String {
    let mut text = format!(
        "{} · {} skills",
        catalog
            .roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(" · "),
        catalog.entries.len()
    );
    if !catalog.entries.is_empty() {
        text.push_str(&format!(
            " · {} off",
            catalog
                .entries
                .iter()
                .filter(|e| e.global == GlobalState::Off)
                .count()
        ));
    }
    text
}

fn empty_catalog_text(catalog: &SkillCatalog) -> String {
    if !catalog.entries.is_empty() {
        return "No matching skills.".into();
    }
    let paths = catalog
        .roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join(" or ");
    let missing = if catalog.root_exists {
        ""
    } else if catalog.roots.len() > 1 {
        " (neither directory exists)"
    } else {
        " (directory does not exist)"
    };
    format!("No skills in {paths}{missing}.")
}

fn dirty_buffer(original: &str, buffer: &str) -> bool {
    original != buffer
}

fn frontmatter_rows(text: &str) -> Vec<(String, String)> {
    parse_skill_document(text).frontmatter
}

fn badges(entry: &SkillEntry) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(
            skill_badges(entry)
                .into_iter()
                .enumerate()
                .map(|(index, badge)| {
                    let chip = Badge::new(badge.label, badge.tone);
                    match badge.hint {
                        Some(hint) => {
                            Tooltip::new(("skill-badge", index), hint, chip).into_any_element()
                        }
                        None => chip.into_any_element(),
                    }
                }),
        )
        .into_any_element()
}

pub(crate) struct SkillsPane {
    app_store: Entity<AppStore>,
    catalogs: Vec<SkillCatalog>,
    runtime: Runtime,
    runtime_select: Entity<StyledSelect>,
    search: Entity<TextField>,
    loading: bool,
    error: Option<String>,
    pub(crate) detail: Entity<SkillDetail>,
    _subscriptions: Vec<Subscription>,
}

impl SkillsPane {
    pub(crate) fn new(app_store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let weak = cx.weak_entity();
        let runtime_select = cx.new(|cx| {
            StyledSelect::new(
                "skills-runtime",
                cx.focus_handle(),
                Runtime::ClaudeCode.key(),
                Vec::new(),
                Rc::new(move |value, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let Some(runtime) = Runtime::parse(&value) else {
                            return;
                        };
                        this.runtime = runtime;
                        cx.notify();
                    });
                }),
                cx,
            )
            .width(px(160.))
        });
        let search = cx.new(|cx| {
            TextField::new(cx.focus_handle(), "", "Search skills…", false).text_size(12.)
        });
        let detail = cx.new(|cx| SkillDetail::new(app_store.clone(), cx));
        let subscriptions = vec![
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.observe(&detail, |_, _, cx| cx.notify()),
            cx.subscribe(&detail, |this, _, event: &CatalogUpdate, cx| {
                match &event.0 {
                    Ok(catalogs) => {
                        this.catalogs = catalogs.clone();
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error.clone()),
                }
                cx.notify();
            }),
        ];
        Self {
            app_store,
            catalogs: Vec::new(),
            runtime: Runtime::ClaudeCode,
            runtime_select,
            search,
            loading: false,
            error: None,
            detail,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let core = self.app_store.read(cx).core.clone();
        let task = cx.background_spawn(async move { skills::skill_catalogs(&core) });
        cx.spawn(async move |weak, cx| {
            let catalogs = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.loading = false;
                this.runtime_select.update(cx, |select, cx| {
                    select.set_options(
                        catalogs
                            .iter()
                            .map(|c| {
                                SelectOption::new(
                                    c.runtime.key(),
                                    runtime_display_name(c.runtime.key()),
                                )
                            })
                            .collect(),
                        cx,
                    )
                });
                this.catalogs = catalogs;
                this.error = None;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn row(&self, entry: &SkillEntry, index: usize, cx: &Context<Self>) -> AnyElement {
        let detail = self.detail.clone();
        let edit_detail = detail.clone();
        let runtime = self.runtime;
        let edit_runtime = runtime;
        let path = entry.path.clone();
        let edit_path = path.clone();
        let toggle_detail = detail.clone();
        let toggle_runtime = runtime;
        let toggle_path = path.clone();
        let pending = self.detail.read(cx).pending_toggle.as_deref() == Some(entry.path.as_path());
        div()
            .id(("skill-row", index))
            .group("skill-row")
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .px_4()
            .py(rems(10. / 16.))
            .min_h(rems(58. / 16.))
            .cursor_pointer()
            .opacity(if entry.global == GlobalState::Off {
                0.5
            } else {
                1.
            })
            .hover(|row| row.bg(theme::raised()))
            .on_click(move |_, window, cx| {
                detail.update(cx, |detail, cx| {
                    detail.open(runtime, path.clone(), false, window, cx)
                })
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(entry.name.clone()),
                            )
                            .child(badges(entry)),
                    )
                    .child(
                        div()
                            .text_size(rems(11. / 16.))
                            .text_color(theme::faint())
                            .truncate()
                            .child(first_sentence(&entry.description)),
                    ),
            )
            .child(
                IconButton::new(("skill-edit", index), "square-pen.svg")
                    .size(IconButtonSize::Sm)
                    .tooltip("Edit skill")
                    .reveal_on_group_hover("skill-row")
                    .stop_click_propagation(true)
                    .disabled(!entry.files.contains(&entry.marker))
                    .on_press(move |window, cx| {
                        edit_detail.update(cx, |detail, cx| {
                            detail.open(edit_runtime, edit_path.clone(), true, window, cx)
                        })
                    }),
            )
            .child(
                Toggle::new(("skill-toggle", index), entry.global != GlobalState::Off)
                    .disabled(
                        pending
                            || (self.runtime == Runtime::Codex
                                && !entry.files.contains(&entry.marker)),
                    )
                    .on_change(move |enabled, _, cx| {
                        cx.stop_propagation();
                        toggle_detail.update(cx, |detail, cx| {
                            detail.set_enabled(toggle_runtime, toggle_path.clone(), enabled, cx)
                        });
                    }),
            )
            .into_any_element()
    }
}

impl Render for SkillsPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity();
        let catalog = self
            .catalogs
            .iter()
            .find(|catalog| catalog.runtime == self.runtime);
        let query = self.search.read(cx).text();
        let rows: Vec<_> = catalog
            .into_iter()
            .flat_map(|catalog| catalog.entries.iter())
            .filter(|entry| matches_search(entry, query))
            .enumerate()
            .map(|(i, entry)| self.row(entry, i, cx))
            .collect();
        let empty = catalog.map(empty_catalog_text).unwrap_or_else(|| {
            if self.loading {
                "Loading skills…".into()
            } else {
                "No skills catalogs available.".into()
            }
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                PaneHeader::new("Skills", "").action(
                    Button::new("skills-refresh", "Refresh")
                        .icon("refresh-cw.svg")
                        .size(ButtonSize::Sm)
                        .disabled(self.loading)
                        .on_press(move |_, cx| owner.update(cx, |this, cx| this.refresh(cx))),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.runtime_select.clone())
                    .child(div().flex_1().min_w_0().child(self.search.clone())),
            )
            .when_some(catalog, |pane, catalog| {
                pane.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(rems(11. / 16.))
                                .text_color(theme::faint())
                                .child(catalog_meta(catalog)),
                        )
                        .child(
                            div()
                                .text_size(rems(11. / 16.))
                                .line_height(rems(1.))
                                .text_color(theme::faint())
                                .child(if catalog.runtime == Runtime::ClaudeCode {
                                    CLAUDE_CAPTION
                                } else {
                                    CODEX_CAPTION
                                }),
                        ),
                )
            })
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::danger())
                    .child(error)
            }))
            .child(SettingsCard::new(if rows.is_empty() {
                vec![div()
                    .px_4()
                    .py_4()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::faint())
                    .child(empty)
                    .into_any_element()]
            } else {
                rows
            }))
    }
}

struct CatalogUpdate(Result<Vec<SkillCatalog>, String>);

struct OpenSkill {
    runtime: Runtime,
    entry: SkillEntry,
    text: String,
    read_error: Option<String>,
}

impl OpenSkill {
    fn from_read(runtime: Runtime, entry: SkillEntry, result: Result<String, String>) -> Self {
        let (text, read_error) = match result {
            Ok(text) => (text, None),
            Err(error) => (String::new(), Some(error)),
        };
        Self {
            runtime,
            entry,
            text,
            read_error,
        }
    }
}

pub(crate) struct SkillDetail {
    app_store: Entity<AppStore>,
    skill: Option<OpenSkill>,
    editor: Entity<TextField>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    overview_scroll: ScrollHandle,
    editing: bool,
    source: bool,
    confirming: bool,
    busy: bool,
    pending_toggle: Option<PathBuf>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CatalogUpdate> for SkillDetail {}

impl SkillDetail {
    fn new(app_store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            TextField::textarea(cx.focus_handle(), "", "", 16, true)
                .text_size(12.)
                .fill_height()
                .with_scrollbar(cx)
        });
        let focus = cx.focus_handle();
        let scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), owner));
        let overview_scroll = ScrollHandle::new();
        let weak = cx.weak_entity();
        let interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            let key = &event.keystroke;
            let save = key.key == "s"
                && if cfg!(target_os = "macos") {
                    key.modifiers.platform
                } else {
                    key.modifiers.control
                };
            if save && !key.modifiers.alt && !key.modifiers.shift {
                let _ = weak.update(cx, |this, cx| {
                    if this.editing && this.focus.contains_focused(window, cx) {
                        cx.stop_propagation();
                        if !this.confirming {
                            this.save(window, cx);
                        }
                    }
                });
            }
        });
        Self {
            app_store,
            skill: None,
            editor,
            focus,
            previous_focus: None,
            scroll,
            scrollbar,
            overview_scroll,
            editing: false,
            source: false,
            confirming: false,
            busy: false,
            pending_toggle: None,
            error: None,
            _subscriptions: vec![interceptor],
        }
    }

    fn open(
        &mut self,
        runtime: Runtime,
        path: PathBuf,
        editing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.previous_focus = window.focused(cx);
        let core = self.app_store.read(cx).core.clone();
        let task = cx.background_spawn(async move {
            let catalogs = skills::skill_catalogs(&core);
            let entry = catalogs
                .iter()
                .find(|c| c.runtime == runtime)
                .and_then(|c| c.entries.iter().find(|e| e.path == path))
                .cloned()
                .ok_or_else(|| "Skill is no longer in the catalog".to_owned())?;
            let read = skills::read_skill(&core, runtime, &path).map_err(|e| e.to_string());
            Ok::<_, String>((OpenSkill::from_read(runtime, entry, read), catalogs))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.finish_open(result, editing, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_open(
        &mut self,
        result: Result<(OpenSkill, Vec<SkillCatalog>), String>,
        editing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        match result {
            Ok((skill, catalogs)) => {
                let editing = editing && skill.read_error.is_none();
                self.editor
                    .update(cx, |editor, cx| editor.reset(skill.text.clone(), cx));
                self.skill = Some(skill);
                cx.emit(CatalogUpdate(Ok(catalogs)));
                self.editing = editing;
                self.source = false;
                self.confirming = false;
                self.scroll.set_offset(gpui::point(px(0.), px(0.)));
                self.overview_scroll.set_offset(gpui::point(px(0.), px(0.)));
                if editing {
                    self.editor.read(cx).focus_handle().focus(window);
                } else {
                    self.focus.focus(window);
                }
            }
            Err(error) => cx.emit(CatalogUpdate(Err(error))),
        }
        cx.notify();
    }

    fn set_enabled(
        &mut self,
        runtime: Runtime,
        path: PathBuf,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.pending_toggle = Some(path.clone());
        self.error = None;
        let core = self.app_store.read(cx).core.clone();
        let task = cx.background_spawn(async move {
            skills::set_global_enabled(&core, runtime, &path, enabled)
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(skills::skill_catalogs(&core))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.busy = false;
                this.pending_toggle = None;
                match &result {
                    Ok(catalogs) => {
                        if let Some(skill) = &mut this.skill {
                            if let Some(entry) = catalogs
                                .iter()
                                .find(|c| c.runtime == skill.runtime)
                                .and_then(|c| c.entries.iter().find(|e| e.path == skill.entry.path))
                            {
                                skill.entry = entry.clone();
                            }
                        }
                    }
                    Err(error) => this.error = Some(error.clone()),
                }
                cx.emit(CatalogUpdate(result));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(skill) = &self.skill {
            if skill.read_error.is_some() {
                return;
            }
            self.editor
                .update(cx, |editor, cx| editor.reset(skill.text.clone(), cx));
            self.editing = true;
            self.editor.read(cx).focus_handle().focus(window);
            cx.notify();
        }
    }

    fn request_dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.editing
            && self
                .skill
                .as_ref()
                .is_some_and(|skill| dirty_buffer(&skill.text, self.editor.read(cx).text()))
        {
            self.confirming = true;
            self.focus.focus(window);
            cx.notify();
        } else if self.editing {
            self.discard_edit(window, cx);
        } else {
            self.close(window, cx);
        }
    }

    fn discard_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(skill) = &self.skill {
            self.editor
                .update(cx, |editor, cx| editor.reset(skill.text.clone(), cx));
        }
        self.editing = false;
        self.confirming = false;
        self.source = false;
        self.error = None;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.focus.focus(window);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.skill = None;
        self.editing = false;
        self.confirming = false;
        self.error = None;
        if let Some(focus) = self.previous_focus.take() {
            focus.focus(window);
        }
        cx.notify();
    }

    fn cancel_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirming = false;
        self.editor.read(cx).focus_handle().focus(window);
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.editing || self.confirming {
            return;
        }
        let Some(skill) = &self.skill else {
            return;
        };
        let runtime = skill.runtime;
        let path = skill.entry.path.clone();
        let text = self.editor.read(cx).text().to_owned();
        let core = self.app_store.read(cx).core.clone();
        self.busy = true;
        self.error = None;
        self.editor
            .update(cx, |editor, cx| editor.set_disabled(true, cx));
        let task = cx.background_spawn(async move {
            let entry =
                skills::save_skill(&core, runtime, &path, &text).map_err(|e| e.to_string())?;
            Ok::<_, String>((
                OpenSkill {
                    runtime,
                    entry,
                    text,
                    read_error: None,
                },
                skills::skill_catalogs(&core),
            ))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.editor
                    .update(cx, |editor, cx| editor.set_disabled(false, cx));
                match result {
                    Ok((skill, catalogs)) => {
                        this.skill = Some(skill);
                        this.editing = false;
                        this.focus.focus(window);
                        this.source = false;
                        this.scroll.set_offset(gpui::point(px(0.), px(0.)));
                        cx.emit(CatalogUpdate(Ok(catalogs)));
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_modal(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let skill = self.skill.as_ref()?;
        let owner = cx.entity();
        let edit_owner = owner.clone();
        let close_owner = owner.clone();
        let backdrop_owner = owner.clone();
        let title = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().min_w_0().truncate().child(skill.entry.name.clone()))
            .child(badges(&skill.entry))
            .when(self.editing, |title| {
                title.child(Badge::new("editing", Tone::Accent))
            })
            .child(div().flex_1())
            .when(!self.editing, |title| {
                title
                    .child(
                        Button::new("skill-modal-edit", "Edit")
                            .icon("square-pen.svg")
                            .size(ButtonSize::Sm)
                            .disabled(self.busy || skill.read_error.is_some())
                            .on_press(move |window, cx| {
                                edit_owner.update(cx, |this, cx| this.edit(window, cx))
                            }),
                    )
                    .child(
                        IconButton::new("skill-modal-close", "close.svg")
                            .size(IconButtonSize::Sm)
                            .disabled(self.busy)
                            .on_press(move |window, cx| {
                                close_owner.update(cx, |this, cx| this.request_dismiss(window, cx))
                            }),
                    )
            });
        let path = skill.entry.path.clone();
        let path_text = match &skill.entry.symlink {
            Some(target) => format!("{} → {}", path.display(), target.display()),
            None => path.display().to_string(),
        };
        let toggle_owner = owner.clone();
        let runtime = skill.runtime;
        let toggle_path = skill.entry.path.clone();
        let overview = div()
            .id("skill-overview")
            .debug_selector(|| "SKILL_OVERVIEW".into())
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .scrollbar_width(px(0.))
            .track_scroll(&self.overview_scroll)
            .child(
                div()
                    .debug_selector(|| "SKILL_DESCRIPTION".into())
                    .text_size(rems(12. / 16.))
                    .line_height(rems(18. / 16.))
                    .text_color(theme::muted())
                    .child(skill.entry.description.clone()),
            )
            .child(
                div()
                    .debug_selector(|| "SKILL_PATH_ROW".into())
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .debug_selector(|| "SKILL_PATH".into())
                            .flex_1()
                            .min_w_0()
                            .text_size(rems(10. / 16.))
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_color(theme::faint())
                            .child(path_text),
                    )
                    .child(
                        Button::new("skill-reveal", if cfg!(windows) { "Reveal in Explorer" } else { "Reveal in Finder" })
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_press(move |_, cx| cx.reveal_path(&path)),
                    ),
            )
            .child(
                div().flex().flex_wrap().gap_1().children(
                    skill.entry.files.iter().map(|file| Badge::new(file.clone(), Tone::Muted)),
                ),
            )
            .child(
                    div()
                        .debug_selector(|| "SKILL_ENABLED_ROW".into())
                        .flex()
                        .items_center()
                        .gap_4()
                        .rounded(rems(6. / 16.))
                        .bg(theme::raised())
                        .px_3()
                        .py_2()
                        .child(
                            div().flex_1().min_w_0().flex().flex_col().gap_1()
                                .child(div().text_size(rems(12. / 16.)).child(format!("Enabled in {}", runtime_display_name(skill.runtime.key()))))
                                .child(
                                    div().text_size(rems(10. / 16.))
                                        .line_height(rems(15. / 16.))
                                        .text_color(theme::faint())
                                        .child(if skill.runtime == Runtime::Codex {
                                            "Applies to new Codex sessions. Writes only this skill’s [[skills.config]] entry in Codex config.toml; Claude Code is unchanged."
                                        } else {
                                            "Applies to every new Claude Code session, inside Runner or not. Writes only skillOverrides in ~/.claude/settings.json."
                                        }),
                                ),
                        )
                        .child(
                            Toggle::new("skill-detail-toggle", skill.entry.global != GlobalState::Off)
                                .disabled(self.busy || (skill.runtime == Runtime::Codex && !skill.entry.files.contains(&skill.entry.marker)))
                                .on_change(move |enabled, _, cx| {
                                    toggle_owner.update(cx, |this, cx| this.set_enabled(runtime, toggle_path.clone(), enabled, cx));
                                }),
                        ),
            );
        let overview = div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .max_h(gpui::relative(0.45))
            .child(overview);
        let preview_owner = owner.clone();
        let source_owner = owner.clone();
        let document_bar = div()
            .flex()
            .items_center()
            .gap_2()
            .flex_none()
            .pb_2()
            .child(
                div()
                    .flex_1()
                    .text_size(rems(10. / 16.))
                    .text_color(theme::faint())
                    .child(format!(
                        "{} · {} lines",
                        skill.entry.marker,
                        skill.text.lines().count()
                    )),
            )
            .when(!self.editing && skill.read_error.is_none(), |bar| {
                bar.child(
                    Button::new("skill-preview", "Preview")
                        .size(ButtonSize::Sm)
                        .variant(if self.source {
                            ButtonVariant::Ghost
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_press(move |_, cx| {
                            preview_owner.update(cx, |this, cx| {
                                this.source = false;
                                this.scroll.set_offset(gpui::point(px(0.), px(0.)));
                                cx.notify();
                            })
                        }),
                )
                .child(
                    Button::new("skill-source", "Source")
                        .size(ButtonSize::Sm)
                        .variant(if self.source {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Ghost
                        })
                        .on_press(move |_, cx| {
                            source_owner.update(cx, |this, cx| {
                                this.source = true;
                                this.scroll.set_offset(gpui::point(px(0.), px(0.)));
                                cx.notify();
                            })
                        }),
                )
            });
        let document = if self.editing {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(self.editor.clone())
                .into_any_element()
        } else {
            let contents = if let Some(error) = &skill.read_error {
                div()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::faint())
                    .child(format!(
                        "Cannot read {}: {error}. Reveal the folder to inspect its files.",
                        skill.entry.marker
                    ))
                    .into_any_element()
            } else if self.source {
                div()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(rems(12. / 16.))
                    .line_height(rems(20. / 16.))
                    .child(skill.text.clone())
                    .into_any_element()
            } else {
                let doc = parse_skill_document(&skill.text);
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::muted())
                    .children(doc.problem.map(|error| {
                        div()
                            .text_size(rems(11. / 16.))
                            .text_color(theme::danger())
                            .child(error)
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .when(!doc.frontmatter.is_empty(), |table| {
                                table.p_3().rounded(rems(6. / 16.)).bg(theme::panel())
                            })
                            .children(frontmatter_rows(&skill.text).into_iter().map(
                                |(key, value)| {
                                    div()
                                        .flex()
                                        .gap_3()
                                        .text_size(rems(11. / 16.))
                                        .line_height(rems(1.))
                                        .debug_selector(|| "SKILL_FRONTMATTER_ROW".into())
                                        .child(
                                            div()
                                                .w(rems(170. / 16.))
                                                .flex_none()
                                                .font_family(theme::UI_MONOSPACE_FONT)
                                                .text_color(theme::faint())
                                                .child(key),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_color(theme::muted())
                                                .debug_selector(|| "SKILL_FRONTMATTER_VALUE".into())
                                                .child(value),
                                        )
                                },
                            )),
                    )
                    .child(mission_markdown::render_markdown(
                        "skill-preview-document",
                        &doc.body,
                        cx.entity_id(),
                        None,
                        theme::accent(),
                        None,
                        cx,
                    ))
                    .into_any_element()
            };
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .rounded(rems(4. / 16.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::bg())
                .child(
                    div()
                        .id("skill-document-scroll")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .scrollbar_width(px(0.))
                        .track_scroll(&self.scroll)
                        .p_3()
                        .child(div().flex_none().w_full().min_w_0().child(contents)),
                )
                .child(self.scrollbar.clone())
                .into_any_element()
        };
        let body = div()
            .debug_selector(|| "SKILL_MODAL_BODY".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(overview)
            .children(self.error.clone().map(|error| {
                div()
                    .flex_none()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::danger())
                    .child(error)
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .debug_selector(|| "SKILL_DOCUMENT_PANEL".into())
                    .child(document_bar)
                    .child(document),
            );
        let mut modal = Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                backdrop_owner.update(cx, |this, cx| this.request_dismiss(window, cx))
            }),
        )
        .width(OverlayWidth::Custom(680.))
        .height(790.)
        .busy(self.busy);
        if self.editing {
            let cancel_owner = owner.clone();
            let save_owner = owner.clone();
            let hint = match &skill.entry.symlink {
                Some(target) => format!(
                    "Writes {} in place — through the symlink into {}",
                    skill.entry.marker,
                    target.display()
                ),
                None => format!("Writes {} in place.", skill.entry.marker),
            };
            modal = modal.footer(
                div()
                    .debug_selector(|| "SKILL_FOOTER".into())
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(rems(10. / 16.))
                            .text_color(theme::faint())
                            .debug_selector(|| "SKILL_FOOTER_HINT".into())
                            .child(hint),
                    )
                    .child(
                        Button::new("skill-edit-cancel", "Cancel")
                            .disabled(self.busy)
                            .on_press(move |window, cx| {
                                cancel_owner.update(cx, |this, cx| this.request_dismiss(window, cx))
                            }),
                    )
                    .child(
                        Button::new("skill-edit-save", "Save")
                            .variant(ButtonVariant::Primary)
                            .disabled(self.busy)
                            .on_press(move |window, cx| {
                                save_owner.update(cx, |this, cx| this.save(window, cx))
                            }),
                    ),
            );
        }
        Some(modal.into_any_element())
    }
}

impl Render for SkillDetail {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.skill.is_none() {
            return div().into_any_element();
        }
        let confirm_owner = cx.entity();
        let cancel_owner = confirm_owner.clone();
        div()
            .absolute()
            .inset_0()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    if this.confirming {
                        this.cancel_discard(window, cx);
                    } else {
                        this.request_dismiss(window, cx);
                    }
                }
            }))
            .children(self.render_modal(cx))
            .when(self.confirming, |overlay| {
                overlay.child(
                    ConfirmDialog::new(
                        "Discard unsaved changes?",
                        "Your edits to this skill have not been saved.",
                        "Discard changes",
                        "Discarding…",
                        false,
                        Rc::new(move |window, cx| {
                            confirm_owner.update(cx, |this, cx| this.discard_edit(window, cx))
                        }),
                        Rc::new(move |window, cx| {
                            cancel_owner.update(cx, |this, cx| this.cancel_discard(window, cx))
                        }),
                    )
                    .icon("square-pen.svg"),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> SkillEntry {
        SkillEntry {
            name: "daily-brief".into(),
            description: "Produce the trading brief. More detail.".into(),
            path: "/skills/daily-brief".into(),
            marker: "SKILL.md".into(),
            symlink: None,
            manual: false,
            hidden: false,
            problem: None,
            files: vec!["SKILL.md".into()],
            global: GlobalState::On,
        }
    }

    fn test_store(path: &std::path::Path, cx: &mut gpui::TestAppContext) -> Entity<AppStore> {
        use runner_backend::{
            db, event_bus, events, mcp, router, session, shell_path, windows, AppCore,
        };
        use std::sync::{Arc, Mutex, RwLock};
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = AppCore {
            db: Arc::new(db::open_pool(&path.join("runner.db")).unwrap()),
            app_data_dir: path.into(),
            sessions: session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
            buses: event_bus::BusRegistry::new(),
            routers: router::RouterRegistry::new(),
            mission_grid_hint: Arc::new(Mutex::new(None)),
            mcp: Arc::new(mcp::McpHandle::new()),
            windows: Arc::new(windows::WindowRegistry::new()),
            events: events::EventChannel::new(),
            session_event_observer: Default::default(),
            app_version: "0.0.0-test".into(),
        };
        cx.new(|cx| {
            AppStore::new(
                core,
                path.join("settings.json"),
                crate::app_settings::AppSettings::default(),
                Some("test settings".into()),
                cx,
            )
        })
    }

    struct SkillsTestHost(Entity<SkillsPane>);

    impl Render for SkillsTestHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(self.0.clone())
                .child(self.0.read(cx).detail.clone())
        }
    }

    #[test]
    fn missing_marker_opens_read_only_details_and_success_clears_pane_error() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|_, cx| SkillsTestHost(cx.new(|cx| SkillsPane::new(store, cx))));
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        host.update(&mut visual, |host, window, cx| {
            host.0.update(cx, |pane, cx| {
                pane.error = Some("previous error".into());
                let mut missing = entry();
                missing.files.clear();
                missing.problem = Some("no SKILL.md".into());
                let skill = OpenSkill::from_read(
                    Runtime::ClaudeCode,
                    missing,
                    Err("skill has no marker file".into()),
                );
                pane.detail.update(cx, |detail, cx| {
                    detail.finish_open(Ok((skill, Vec::new())), true, window, cx)
                });
            });
        })
        .unwrap();
        visual.run_until_parked();
        host.update(&mut visual, |host, window, cx| {
            let pane = host.0.read(cx);
            assert!(pane.error.is_none());
            let detail = pane.detail.clone();
            detail.update(cx, |detail, cx| {
                let skill = detail.skill.as_ref().unwrap();
                assert_eq!(skill.entry.path, entry().path);
                assert!(skill.read_error.is_some());
                assert!(!detail.editing);
                detail.edit(window, cx);
                assert!(!detail.editing);
            });
            host.0.update(cx, |pane, cx| {
                pane.error = Some("another error".into());
                let skill =
                    OpenSkill::from_read(Runtime::ClaudeCode, entry(), Ok("# Fresh file".into()));
                pane.detail.update(cx, |detail, cx| {
                    detail.finish_open(Ok((skill, Vec::new())), false, window, cx)
                });
            });
        })
        .unwrap();
        visual.run_until_parked();
        host.update(&mut visual, |host, _, cx| {
            let pane = host.0.read(cx);
            assert!(pane.error.is_none());
            let skill = pane.detail.read(cx).skill.as_ref().unwrap();
            assert_eq!(skill.text, "# Fresh file");
            assert!(skill.read_error.is_none());
        })
        .unwrap();
    }

    #[test]
    fn both_runtime_details_show_their_own_enabled_state() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|_, cx| SkillsTestHost(cx.new(|cx| SkillsPane::new(store, cx))));
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        for (runtime, global) in [
            (Runtime::ClaudeCode, GlobalState::Off),
            (Runtime::Codex, GlobalState::On),
        ] {
            host.update(&mut visual, |host, window, cx| {
                host.0.update(cx, |pane, cx| {
                    pane.runtime = runtime;
                    let mut selected = entry();
                    selected.global = global.clone();
                    let catalog = SkillCatalog {
                        runtime,
                        roots: vec!["/skills".into()],
                        root_exists: true,
                        entries: vec![selected.clone()],
                    };
                    let skill =
                        OpenSkill::from_read(runtime, selected, Ok("# Shared skill".into()));
                    pane.detail.update(cx, |detail, cx| {
                        detail.finish_open(Ok((skill, vec![catalog])), false, window, cx);
                    });
                });
            })
            .unwrap();
            visual.run_until_parked();
            assert!(
                visual.debug_bounds("SKILL_ENABLED_ROW").is_some(),
                "{runtime}"
            );
            host.update(&mut visual, |host, _, cx| {
                let pane = host.0.read(cx);
                assert_eq!(pane.catalogs[0].runtime, runtime);
                assert_eq!(pane.catalogs[0].entries[0].global, global);
                let detail = pane.detail.read(cx);
                let skill = detail.skill.as_ref().unwrap();
                assert_eq!(skill.runtime, runtime);
                assert_eq!(skill.entry.global, global);
            })
            .unwrap();
        }
    }

    #[test]
    fn cancel_edit_returns_to_preview_and_confirms_before_discarding() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|_, cx| SkillsTestHost(cx.new(|cx| SkillsPane::new(store, cx))));
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        for dirty in [false, true] {
            host.update(&mut visual, |host, window, cx| {
                let detail = host.0.read(cx).detail.clone();
                detail.update(cx, |detail, cx| {
                    let skill = OpenSkill::from_read(
                        Runtime::ClaudeCode,
                        entry(),
                        Ok("# Saved skill".into()),
                    );
                    detail.finish_open(Ok((skill, Vec::new())), true, window, cx);
                    if dirty {
                        detail.editor.update(cx, |editor, cx| {
                            editor.reset("# Unsaved edit", cx);
                        });
                    }
                    detail.request_dismiss(window, cx);
                    assert_eq!(detail.editing, dirty);
                    assert_eq!(detail.confirming, dirty);
                    assert_eq!(detail.skill.as_ref().unwrap().text, "# Saved skill");
                    if dirty {
                        detail.cancel_discard(window, cx);
                        assert!(detail.editing);
                        assert!(!detail.confirming);
                        assert_eq!(detail.editor.read(cx).text(), "# Unsaved edit");
                        detail.request_dismiss(window, cx);
                        assert!(detail.confirming);
                        detail.discard_edit(window, cx);
                    }
                    assert!(!detail.editing);
                    assert!(!detail.confirming);
                    assert!(!detail.source);
                    assert_eq!(detail.skill.as_ref().unwrap().text, "# Saved skill");
                    assert_eq!(detail.editor.read(cx).text(), "# Saved skill");
                    assert!(detail.focus.is_focused(window));
                });
            })
            .unwrap();
            visual.run_until_parked();
            assert!(visual.debug_bounds("SKILL_DOCUMENT_PANEL").is_some());
            assert!(visual.debug_bounds("SKILL_FOOTER").is_none());
            host.update(&mut visual, |host, window, cx| {
                let detail = host.0.read(cx).detail.clone();
                detail.update(cx, |detail, cx| {
                    detail.edit(window, cx);
                    assert_eq!(detail.editor.read(cx).text(), "# Saved skill");
                    detail.request_dismiss(window, cx);
                    assert!(detail.skill.is_some());
                    detail.request_dismiss(window, cx);
                    assert!(detail.skill.is_none());
                });
            })
            .unwrap();
        }
    }

    #[test]
    fn modal_long_paragraphs_wrap_within_their_containers() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|_, cx| SkillsTestHost(cx.new(|cx| SkillsPane::new(store, cx))));
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        for editing in [false, true] {
            host.update(&mut visual, |host, window, cx| {
                let mut long = entry();
                long.description = "Inspect local skills and their descriptions, edit the selected document, and preserve all other files. ".repeat(14);
                long.symlink = Some(std::path::PathBuf::from(format!("/Users/test/{}/skills/long-description", "project workspace/".repeat(12))));
                let text = format!("---\ndescription: {}\n---\n{}", long.description, "Document paragraph.\n\n".repeat(100));
                let skill = OpenSkill::from_read(Runtime::ClaudeCode, long, Ok(text));
                let detail = host.0.read(cx).detail.clone();
                detail.update(cx, |detail, cx| detail.finish_open(Ok((skill, Vec::new())), editing, window, cx));
            }).unwrap();
            for rem in [16., 20.8] {
                host.update(&mut visual, |_, window, _| window.set_rem_size(px(rem)))
                    .unwrap();
                for size in [
                    gpui::size(px(800.), px(600.)),
                    gpui::size(px(1440.), px(1000.)),
                ] {
                    visual.simulate_resize(size);
                    visual.run_until_parked();
                    let body = visual.debug_bounds("SKILL_MODAL_BODY").unwrap();
                    let overview = visual.debug_bounds("SKILL_OVERVIEW").unwrap();
                    let description = visual.debug_bounds("SKILL_DESCRIPTION").unwrap();
                    assert!(
                        description.size.height >= px(rem * 18. / 16. * 2.)
                            && description.size.height <= px(rem * 18. / 16. * 60.),
                        "{editing} {rem} {size:?}: {description:?}"
                    );
                    assert!(
                        (description.size.width - overview.size.width).abs() < px(3.),
                        "{editing} {rem}: {description:?} {overview:?}"
                    );
                    assert!((overview.size.width - body.size.width).abs() < px(3.));
                    let path = visual.debug_bounds("SKILL_PATH").unwrap();
                    let path_row = visual.debug_bounds("SKILL_PATH_ROW").unwrap();
                    assert!(
                        path.size.width > path_row.size.width * 0.5
                            && path.right() < path_row.right()
                    );
                    assert!(
                        path.size.height >= px(rem * 10. / 16. * 2.)
                            && path.size.height <= px(rem * 10. / 16. * 60.)
                    );
                    let document = visual.debug_bounds("SKILL_DOCUMENT_PANEL").unwrap();
                    assert!(
                        document.size.height > px(40.),
                        "{editing} {rem} {size:?}: {document:?}"
                    );
                    if editing {
                        let footer = visual.debug_bounds("SKILL_FOOTER").unwrap();
                        let hint = visual.debug_bounds("SKILL_FOOTER_HINT").unwrap();
                        assert!(
                            hint.size.height >= px(rem * 10. / 16. * 2.)
                                && hint.size.height <= px(rem * 10. / 16. * 60.)
                        );
                        assert!(
                            hint.size.width > footer.size.width * 0.5
                                && hint.right() < footer.right()
                        );
                        assert!(document.bottom() <= footer.top() && footer.bottom() < size.height);
                    } else {
                        let row = visual.debug_bounds("SKILL_FRONTMATTER_ROW").unwrap();
                        let value = visual.debug_bounds("SKILL_FRONTMATTER_VALUE").unwrap();
                        assert!(
                            value.size.height >= px(rem * 2.) && value.size.height <= px(rem * 60.)
                        );
                        let expected = row.size.width - px(rem * (170. + 12.) / 16.);
                        assert!(
                            (value.size.width - expected).abs() < px(3.),
                            "{rem}: {value:?} {row:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn row_badges_reflect_flags_targets_overrides_and_problems() {
        let mut entry = entry();
        assert!(skill_badges(&entry).is_empty());
        entry.manual = true;
        entry.hidden = true;
        entry.symlink = Some("/target".into());
        entry.global = GlobalState::Other("name-only".into());
        entry.problem = Some("legacy skill.md".into());
        let result = skill_badges(&entry);
        assert_eq!(
            result.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
            ["manual", "hidden", "symlink", "name-only", "problem"]
        );
        assert_eq!(result[2].hint.as_deref(), Some("/target"));
        assert_eq!(result[4].tone, Tone::Danger);
        assert_eq!(result[4].hint.as_deref(), Some("legacy skill.md"));
        entry.global = GlobalState::Other("user-invocable-only".into());
        assert_eq!(skill_badges(&entry)[3].label, "user-invocable-only");
    }

    #[cfg(unix)]
    #[test]
    fn shared_skill_badges_follow_each_runtimes_invocation_policy() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().unwrap();
        let target = home.path().join("shared/demo");
        std::fs::create_dir_all(target.join("agents")).unwrap();
        std::fs::write(
            target.join("skill.md"),
            "---\ndisable-model-invocation: true\nuser-invocable: false\n---\n# Shared\n",
        )
        .unwrap();
        for directory in [".claude/skills", ".agents/skills"] {
            let root = home.path().join(directory);
            std::fs::create_dir_all(&root).unwrap();
            symlink(&target, root.join("demo")).unwrap();
        }
        std::fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"skillOverrides":{"demo":"name-only"}}"#,
        )
        .unwrap();
        for (runtime, expected) in [
            (
                Runtime::ClaudeCode,
                vec!["manual", "hidden", "symlink", "name-only", "problem"],
            ),
            (Runtime::Codex, vec!["symlink", "problem"]),
        ] {
            let catalog =
                runner_backend::skills::skill_catalog(runtime, home.path(), None).unwrap();
            let badges = skill_badges(&catalog.entries[0]);
            assert_eq!(
                badges
                    .iter()
                    .map(|badge| badge.label.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
        std::fs::write(
            target.join("agents/openai.yaml"),
            "policy:\n  allow_implicit_invocation: false\n",
        )
        .unwrap();
        let catalog =
            runner_backend::skills::skill_catalog(Runtime::Codex, home.path(), None).unwrap();
        let badges = skill_badges(&catalog.entries[0]);
        assert_eq!(
            badges
                .iter()
                .map(|badge| badge.label.as_str())
                .collect::<Vec<_>>(),
            ["manual", "symlink", "problem"]
        );
    }

    #[test]
    fn description_clamp_keeps_first_sentence_and_unicode() {
        for (text, expected) in [
            (" First line.\nSecond line.", "First line."),
            ("Version 1.2 works! More.", "Version 1.2 works!"),
            ("你好。下一句", "你好。"),
            ("No punctuation\nmore", "No punctuation more"),
            ("", ""),
        ] {
            assert_eq!(first_sentence(text), expected);
        }
    }

    #[test]
    fn search_is_case_insensitive_over_name_and_full_description() {
        let entry = entry();
        for query in ["", " DAILY ", "TRADING", "more detail"] {
            assert!(matches_search(&entry, query));
        }
        assert!(!matches_search(&entry, "music"));
    }

    #[test]
    fn meta_line_counts_off_states_for_both_runtimes() {
        let mut catalog = SkillCatalog {
            runtime: Runtime::ClaudeCode,
            roots: vec!["/skills".into()],
            root_exists: true,
            entries: vec![entry(), entry()],
        };
        catalog.entries[1].global = GlobalState::Off;
        assert_eq!(catalog_meta(&catalog), "/skills · 2 skills · 1 off");
        catalog.runtime = Runtime::Codex;
        assert_eq!(catalog_meta(&catalog), "/skills · 2 skills · 1 off");
        catalog.entries.clear();
        assert_eq!(catalog_meta(&catalog), "/skills · 0 skills");
        assert_eq!(empty_catalog_text(&catalog), "No skills in /skills.");
        catalog.root_exists = false;
        assert_eq!(
            empty_catalog_text(&catalog),
            "No skills in /skills (directory does not exist)."
        );
        catalog.roots = vec!["/agents/skills".into(), "/codex/skills".into()];
        assert_eq!(
            catalog_meta(&catalog),
            "/agents/skills · /codex/skills · 0 skills"
        );
        assert_eq!(
            empty_catalog_text(&catalog),
            "No skills in /agents/skills or /codex/skills (neither directory exists)."
        );
        catalog.root_exists = true;
        assert_eq!(
            empty_catalog_text(&catalog),
            "No skills in /agents/skills or /codex/skills."
        );
        catalog.entries.push(entry());
        assert_eq!(
            catalog_meta(&catalog),
            "/agents/skills · /codex/skills · 1 skills · 0 off"
        );
        assert_eq!(empty_catalog_text(&catalog), "No matching skills.");
    }

    #[test]
    fn frontmatter_table_excludes_fences_and_body() {
        assert_eq!(
            frontmatter_rows(
                "---\nname: demo\ndescription: 'A skill'\nuser-invocable: false\n---\n# Body"
            ),
            [
                ("name".into(), "demo".into()),
                ("description".into(), "A skill".into()),
                ("user-invocable".into(), "false".into())
            ]
        );
        assert!(frontmatter_rows("# No frontmatter").is_empty());
    }

    #[test]
    fn dirty_check_compares_exact_text_even_after_undo() {
        assert!(!dirty_buffer("same\n", "same\n"));
        assert!(dirty_buffer("same\n", "same"));
        assert!(dirty_buffer("same\r\n", "same\n"));
        assert!(!dirty_buffer("", ""));
    }
}
