use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, Context, Entity, FocusHandle, FontWeight, KeyDownEvent, Render,
    ScrollHandle, Subscription, Window,
};
use runner_app::ui::{
    Badge, Button, ButtonSize, ButtonVariant, ConfirmDialog, IconButton, IconButtonSize, Modal,
    OverlayWidth, PaneHeader, Scrollbar, SelectOption, SettingsCard, StyledSelect, TextField,
    Toggle, Tone,
};
use runner_backend::ops::mcp::{
    self, McpCatalog, McpClientId, McpClientStatus, McpIntegrationStatus, McpServerDefinition,
    McpServerEntry,
};
use runner_backend::ops::runtime::RuntimeCatalogEntry;

use crate::app_settings::AppSettings;
use crate::app_store::AppStore;
use crate::theme;

const CAPTION: &str = "Toggles register or unregister a server in this runtime's config. Runner writes only the named entry. Runner's own server is pinned first; add new servers with the agent's own tooling. Running sessions see changes on their next launch.";
const RUNNER_DESCRIPTION: &str = "Coordinate crews, missions, and messages from your agent.";
const EDIT_FOOTER: &str = "Only the named entry changes in each file. Invalid JSON or TOML cannot be saved. To rename a server, remove and re-add it through the agent's CLI.";

fn registered_clients(entry: &McpServerEntry) -> Vec<McpClientId> {
    McpClientId::ALL
        .into_iter()
        .filter(|client| entry.clients.get(client).is_some_and(|e| e.registered))
        .collect()
}

fn source_client(entry: &McpServerEntry) -> Option<McpClientId> {
    registered_clients(entry).first().copied()
}

fn server_toggle_on(
    entry: &McpServerEntry,
    client: McpClientId,
    runner: &McpIntegrationStatus,
) -> bool {
    if entry.name == "runner" {
        client.status(runner).matches_current
    } else {
        entry
            .clients
            .get(&client)
            .is_some_and(|slot| slot.registered)
    }
}

fn definition_for(entry: &McpServerEntry, client: McpClientId) -> Option<&McpServerDefinition> {
    let slot = entry
        .clients
        .get(&client)
        .filter(|s| s.registered)
        .or_else(|| source_client(entry).and_then(|source| entry.clients.get(&source)))?;
    slot.definition.as_ref()
}

fn transport_badge(entry: &McpServerEntry, client: McpClientId) -> &'static str {
    if entry.name == "runner" {
        return "built-in";
    }
    match definition_for(entry, client) {
        Some(McpServerDefinition::Stdio { .. }) => "stdio",
        Some(McpServerDefinition::Http { .. }) => "http",
        None => "other",
    }
}

fn definition_summary(definition: Option<&McpServerDefinition>) -> String {
    let Some(definition) = definition else {
        return "Transport managed by the agent's CLI".into();
    };
    let (mut text, secrets) = match definition {
        McpServerDefinition::Stdio { command, args, env } => (
            std::iter::once(command.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" "),
            env,
        ),
        McpServerDefinition::Http { url, headers } => (url.clone(), headers),
    };
    for key in secrets.keys() {
        text.push_str(&format!(" · {key}=•••"));
    }
    text
}

fn conflict_caption(entry: &McpServerEntry, selected: McpClientId) -> Option<String> {
    let reference = entry
        .clients
        .get(&selected)
        .filter(|s| s.registered)
        .or_else(|| source_client(entry).and_then(|c| entry.clients.get(&c)))?;
    let (&other, slot) = entry.clients.iter().find(|(client, slot)| {
        **client != selected
            && slot.registered
            && slot.conflicting
            && (slot.definition != reference.definition
                || (slot.definition.is_none() && slot.native_text != reference.native_text))
    })?;
    Some(format!(
        "{} has a different definition: {}",
        other.label(),
        definition_summary(slot.definition.as_ref())
    ))
}

fn copy_hint(entry: &McpServerEntry, client: McpClientId) -> Option<String> {
    if entry.name == "runner" || entry.clients.get(&client).is_some_and(|s| s.registered) {
        return None;
    }
    let source = source_client(entry)?;
    entry.clients[&source].definition.is_none().then(|| {
        format!(
            "This transport cannot be copied. Add it with {}'s CLI.",
            client.label()
        )
    })
}

fn available_clients(catalog: &[RuntimeCatalogEntry], settings: &AppSettings) -> Vec<McpClientId> {
    McpClientId::ALL
        .into_iter()
        .filter(|client| {
            catalog.iter().any(|r| {
                r.name == client.runtime()
                    && r.available
                    && settings.is_agent_enabled(r.name, r.default_enabled)
            })
        })
        .collect()
}

fn catalog_rows<'a>(catalog: &'a McpCatalog, query: &str) -> Vec<&'a McpServerEntry> {
    let query = query.trim().to_lowercase();
    std::iter::once(&catalog.runner_server)
        .chain(
            catalog
                .servers
                .iter()
                .filter(|entry| entry.name.to_lowercase().contains(&query)),
        )
        .collect()
}

pub(crate) struct McpPane {
    app_store: Entity<AppStore>,
    runtime: Option<McpClientId>,
    runtime_select: Entity<StyledSelect>,
    search: Entity<TextField>,
    pub(crate) detail: Entity<McpDetail>,
    _subscriptions: Vec<Subscription>,
}

impl McpPane {
    pub(crate) fn new(app_store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let weak = cx.weak_entity();
        let runtime_select = cx.new(|cx| {
            StyledSelect::new(
                "mcp-runtime",
                cx.focus_handle(),
                "claude_code",
                Vec::new(),
                Rc::new(move |value, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.runtime = McpClientId::parse(&value).ok();
                        cx.notify();
                    });
                }),
                cx,
            )
            .width(px(160.))
        });
        let search = cx.new(|cx| {
            TextField::new(cx.focus_handle(), "", "Search servers…", false)
                .text_size(theme::text_ui())
        });
        let detail = cx.new(|cx| McpDetail::new(app_store.clone(), cx));
        let subscriptions = vec![
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.observe(&detail, |this, _, cx| {
                this.sync_runtime(cx);
                cx.notify();
            }),
            cx.observe(&app_store, |this, _, cx| {
                this.sync_runtime(cx);
                cx.notify();
            }),
        ];
        Self {
            app_store,
            runtime: Some(McpClientId::ClaudeCode),
            runtime_select,
            search,
            detail,
            _subscriptions: subscriptions,
        }
    }

    fn sync_runtime(&mut self, cx: &mut Context<Self>) {
        let clients = available_clients(
            &self.detail.read(cx).runtimes,
            &self.app_store.read(cx).settings,
        );
        if !self
            .runtime
            .is_some_and(|runtime| clients.contains(&runtime))
        {
            self.runtime = clients.first().copied();
        }
        self.runtime_select.update(cx, |select, cx| {
            select.set_options(
                clients
                    .iter()
                    .map(|c| SelectOption::new(c.key(), c.label()))
                    .collect(),
                cx,
            );
            select.set_value(self.runtime.map(McpClientId::key).unwrap_or_default(), cx);
        });
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.detail.update(cx, |detail, cx| detail.refresh(cx));
    }

    fn row(
        &self,
        entry: &McpServerEntry,
        client: McpClientId,
        index: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        let state = self.detail.read(cx);
        let built_in = entry.name == "runner";
        let registered = state
            .catalog
            .as_ref()
            .is_some_and(|catalog| server_toggle_on(entry, client, &catalog.runner));
        let presentation = state.catalog.as_ref().filter(|_| built_in).map(|catalog| {
            mcp_row_presentation(
                client,
                Some(client.status(&catalog.runner)),
                state.busy,
                true,
            )
        });
        let conflict = conflict_caption(entry, client);
        let hint = copy_hint(entry, client);
        let error = entry
            .clients
            .get(&client)
            .and_then(|s| s.error.clone())
            .or_else(|| presentation.as_ref().and_then(|p| p.error.clone()));
        let open = self.detail.clone();
        let toggle = open.clone();
        let name = entry.name.clone();
        let toggle_name = name.clone();
        div()
            .id(("mcp-row", index))
            .debug_selector(move || format!("MCP_ROW_{name}"))
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .px_4()
            .py(rems(10. / 16.))
            .min_h(rems(58. / 16.))
            .cursor_pointer()
            .hover(|row| row.bg(theme::raised()))
            .on_click({
                let name = entry.name.clone();
                move |_, window, cx| {
                    open.update(cx, |detail, cx| detail.open(client, &name, window, cx))
                }
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
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(if built_in {
                                        "Runner".into()
                                    } else {
                                        entry.name.clone()
                                    }),
                            )
                            .child(Badge::new(
                                transport_badge(entry, client),
                                if built_in { Tone::Accent } else { Tone::Muted },
                            )),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .when(!built_in, |d| d.font_family(theme::UI_MONOSPACE_FONT))
                            .child(if built_in {
                                RUNNER_DESCRIPTION.into()
                            } else {
                                definition_summary(definition_for(entry, client))
                            }),
                    )
                    .children(presentation.as_ref().map(|p| {
                        div()
                            .text_size(theme::text_caption())
                            .text_color(row_color(p.tone))
                            .child(p.status.clone())
                    }))
                    .children(conflict.map(|text| {
                        div()
                            .text_size(theme::text_caption())
                            .text_color(theme::warning())
                            .child(format!("⚠ {text}"))
                    }))
                    .children(hint.clone().map(|text| {
                        div()
                            .text_size(theme::text_caption())
                            .text_color(theme::faint())
                            .child(text)
                    }))
                    .children(error.map(|text| {
                        div()
                            .text_size(theme::text_caption())
                            .text_color(theme::danger())
                            .child(text)
                    })),
            )
            .child(
                Toggle::new(("mcp-toggle", index), registered)
                    .disabled(
                        state.busy || hint.is_some() || presentation.is_some_and(|p| p.disabled),
                    )
                    .on_change(move |enabled, _, cx| {
                        cx.stop_propagation();
                        toggle.update(cx, |detail, cx| {
                            detail.set_enabled(client, toggle_name.clone(), enabled, cx)
                        });
                    }),
            )
            .into_any_element()
    }
}

impl Render for McpPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity();
        let state = self.detail.read(cx);
        let mut rows = Vec::new();
        let mut meta = None;
        if let (Some(catalog), Some(client)) = (&state.catalog, self.runtime) {
            rows = catalog_rows(catalog, self.search.read(cx).text())
                .into_iter()
                .enumerate()
                .map(|(i, entry)| self.row(entry, client, i, cx))
                .collect();
            let off = std::iter::once(&catalog.runner_server)
                .chain(&catalog.servers)
                .filter(|entry| !server_toggle_on(entry, client, &catalog.runner))
                .count();
            meta = Some(format!(
                "{} · {} servers · {off} off",
                client.config_file(),
                catalog.servers.len() + 1
            ));
            if self.search.read(cx).text().trim().is_empty()
                && !catalog
                    .servers
                    .iter()
                    .any(|s| s.clients[&client].registered)
            {
                rows.push(div().px_4().py_4().flex().flex_col().gap_2().text_size(theme::text_ui()).text_color(theme::faint())
                    .child(format!("No other servers in {} yet.", client.config_file()))
                    .child(format!("Add servers with {}'s CLI, or switch runtime to copy a server another agent already has.", client.label())).into_any_element());
            } else if rows.len() == 1 && !self.search.read(cx).text().trim().is_empty() {
                rows.push(
                    div()
                        .p_4()
                        .text_size(theme::text_ui())
                        .text_color(theme::faint())
                        .child("No matching servers.")
                        .into_any_element(),
                );
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                PaneHeader::new("MCP", "").action(
                    Button::new("mcp-refresh", "Refresh")
                        .icon("refresh-cw.svg")
                        .size(ButtonSize::Sm)
                        .disabled(state.loading || state.busy)
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
            .children(meta.map(|meta| {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(meta),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .line_height(rems(1.))
                            .text_color(theme::faint())
                            .child(CAPTION),
                    )
            }))
            .children(state.error.clone().map(|error| {
                div()
                    .text_size(theme::text_ui())
                    .text_color(theme::danger())
                    .child(error)
            }))
            .child(SettingsCard::new(if rows.is_empty() {
                vec![div()
                    .p_4()
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child(if state.loading {
                        "Loading MCP servers…"
                    } else {
                        "No supported, enabled agents are installed."
                    })
                    .into_any_element()]
            } else {
                rows
            }))
    }
}

pub(crate) struct McpDetail {
    app_store: Entity<AppStore>,
    catalog: Option<McpCatalog>,
    runtimes: Vec<RuntimeCatalogEntry>,
    name: Option<String>,
    runtime: McpClientId,
    viewing: McpClientId,
    editor: Entity<TextField>,
    original: String,
    also: BTreeSet<McpClientId>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    overview_scroll: ScrollHandle,
    editing: bool,
    confirming: bool,
    busy: bool,
    loading: bool,
    refresh_pending: bool,
    generation: u64,
    initialized: BTreeSet<String>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl McpDetail {
    fn new(app_store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            TextField::textarea(cx.focus_handle(), "", "", 16, true)
                .text_size(theme::text_ui())
                .fill_height()
                .with_scrollbar(cx)
        });
        let scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), owner));
        let initialized = app_store.read(cx).settings.initialized_mcp_clients.clone();
        let subscriptions = vec![
            cx.observe(&editor, |_, _, cx| cx.notify()),
            cx.observe(&app_store, |this, store, cx| {
                let initialized = &store.read(cx).settings.initialized_mcp_clients;
                if this.initialized != *initialized {
                    this.initialized = initialized.clone();
                    this.refresh(cx);
                }
            }),
        ];
        Self {
            app_store,
            catalog: None,
            runtimes: Vec::new(),
            name: None,
            runtime: McpClientId::ClaudeCode,
            viewing: McpClientId::ClaudeCode,
            editor,
            original: String::new(),
            also: BTreeSet::new(),
            focus: cx.focus_handle(),
            previous_focus: None,
            scroll,
            scrollbar,
            overview_scroll: ScrollHandle::new(),
            editing: false,
            confirming: false,
            busy: false,
            loading: false,
            refresh_pending: false,
            generation: 0,
            initialized,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.busy {
            self.refresh_pending = true;
            return;
        }
        self.loading = true;
        self.generation += 1;
        let generation = self.generation;
        let core = self.app_store.read(cx).core.clone();
        let task = cx.background_spawn(async move {
            Ok::<_, String>((
                mcp::mcp_catalog(&core).map_err(|e| e.to_string())?,
                runner_backend::ops::runtime::runtime_catalog(&core).map_err(|e| e.to_string())?,
            ))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.loading = false;
                if this.generation == generation {
                    match result {
                        Ok((catalog, runtimes)) => {
                            this.apply_catalog(catalog);
                            this.runtimes = runtimes;
                            this.error = None;
                        }
                        Err(e) => this.error = Some(e),
                    }
                }
                this.run_pending_refresh(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn run_pending_refresh(&mut self, cx: &mut Context<Self>) {
        if self.refresh_pending && !self.loading && !self.busy {
            self.refresh_pending = false;
            self.refresh(cx);
        }
    }

    fn apply_catalog(&mut self, catalog: McpCatalog) {
        self.catalog = Some(catalog);
        if self.name.is_some() && self.entry().is_none() {
            self.name = None;
            self.editing = false;
            self.confirming = false;
        }
    }

    fn entry(&self) -> Option<&McpServerEntry> {
        let name = self.name.as_deref()?;
        self.find_entry(name)
    }

    fn find_entry(&self, name: &str) -> Option<&McpServerEntry> {
        let catalog = self.catalog.as_ref()?;
        if name == "runner" {
            Some(&catalog.runner_server)
        } else {
            catalog.servers.iter().find(|e| e.name == name)
        }
    }

    fn open(
        &mut self,
        runtime: McpClientId,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.editing {
            return;
        }
        let Some(entry) = self.find_entry(name) else {
            return;
        };
        self.viewing = if entry.clients[&runtime].registered {
            runtime
        } else {
            source_client(entry).unwrap_or(runtime)
        };
        self.runtime = runtime;
        self.name = Some(name.into());
        self.previous_focus = window.focused(cx);
        self.editing = false;
        self.confirming = false;
        self.error = None;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.overview_scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.focus.focus(window);
        cx.notify();
    }

    fn set_enabled(
        &mut self,
        client: McpClientId,
        name: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.editing {
            return;
        }
        let source = self.find_entry(&name).and_then(source_client);
        if name == "runner" {
            self.record_manual_choice(client, cx);
        } else if enabled
            && self
                .find_entry(&name)
                .is_none_or(|entry| copy_hint(entry, client).is_some())
        {
            return;
        }
        self.write(
            move |core| {
                if name == "runner" {
                    mcp::mcp_set_integration(core, client.key(), enabled)
                } else if enabled {
                    mcp::mcp_copy_server(
                        core,
                        source.ok_or_else(|| {
                            runner_backend::error::Error::msg("Server is no longer registered")
                        })?,
                        client,
                        &name,
                    )
                } else {
                    mcp::mcp_remove_server(core, client, &name)
                }
            },
            cx,
        );
    }

    fn record_manual_choice(&mut self, client: McpClientId, cx: &mut Context<Self>) {
        self.initialized.insert(client.key().into());
        self.app_store.update(cx, |store, cx| {
            store.update_settings(
                |settings| settings.initialized_mcp_clients.insert(client.key().into()),
                true,
                cx,
            );
        });
    }

    fn write(
        &mut self,
        operation: impl FnOnce(&runner_backend::AppCore) -> runner_backend::error::Result<()>
            + Send
            + 'static,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.error = None;
        self.generation += 1;
        let generation = self.generation;
        self.editor.update(cx, |e, cx| e.set_disabled(true, cx));
        let core = self.app_store.read(cx).core.clone();
        let task = cx.background_spawn(async move {
            let result = operation(&core).map_err(|e| e.to_string());
            (result, mcp::mcp_catalog(&core).map_err(|e| e.to_string()))
        });
        cx.spawn(async move |weak, cx| {
            let (result, catalog) = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.busy = false;
                this.editor.update(cx, |e, cx| e.set_disabled(false, cx));
                if generation == this.generation {
                    if let Ok(catalog) = &catalog {
                        this.apply_catalog(catalog.clone());
                    }
                    this.error = result.as_ref().err().cloned().or_else(|| catalog.err());
                    if result.is_ok() {
                        this.editing = false;
                        this.confirming = false;
                        if let Some(entry) = this.entry() {
                            if !entry.clients[&this.viewing].registered {
                                this.viewing = source_client(entry).unwrap_or(this.runtime);
                            }
                        } else {
                            this.name = None;
                        }
                    }
                }
                this.run_pending_refresh(cx);
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
        let Some(entry) = self.entry() else {
            return;
        };
        if entry.name == "runner" {
            return;
        }
        let Some(slot) = entry.clients.get(&self.viewing).filter(|e| e.registered) else {
            return;
        };
        let text = slot.native_text.clone();
        self.also = registered_clients(entry)
            .into_iter()
            .filter(|&c| c != self.viewing)
            .collect();
        self.original = text.clone();
        self.editor.update(cx, |editor, cx| editor.reset(text, cx));
        self.editing = true;
        self.error = None;
        self.editor.read(cx).focus_handle().focus(window);
        cx.notify();
    }

    fn validation(&self, cx: &Context<Self>) -> Result<(), String> {
        let name = self
            .name
            .as_deref()
            .ok_or_else(|| "No server selected".to_owned())?;
        mcp::validate_mcp_edit(
            self.viewing,
            name,
            self.editor.read(cx).text(),
            !self.also.is_empty(),
        )
        .map_err(|e| e.to_string())
    }

    fn can_save(&self, cx: &Context<Self>) -> bool {
        self.editing && !self.busy && !self.confirming && self.validation(cx).is_ok()
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.can_save(cx) {
            return;
        }
        let name = self.name.clone().unwrap();
        let client = self.viewing;
        let text = self.editor.read(cx).text().to_owned();
        let also: Vec<_> = self.also.iter().copied().collect();
        self.write(
            move |core| mcp::mcp_edit_server(core, client, &name, &text, &also),
            cx,
        );
    }

    fn request_dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.editing && self.editor.read(cx).text() != self.original {
            self.confirming = true;
            self.focus.focus(window);
        } else if self.editing {
            self.discard_edit(window, cx);
        } else {
            self.name = None;
            if let Some(focus) = self.previous_focus.take() {
                focus.focus(window);
            }
        }
        cx.notify();
    }

    fn discard_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.reset(self.original.clone(), cx));
        self.editing = false;
        self.confirming = false;
        self.error = None;
        self.focus.focus(window);
        cx.notify();
    }

    fn cancel_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirming = false;
        self.editor.read(cx).focus_handle().focus(window);
        cx.notify();
    }

    fn render_modal(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let entry = self.entry()?;
        let client = self.viewing;
        let runtime = self.runtime;
        let owner = cx.entity();
        let edit_owner = owner.clone();
        let close_owner = owner.clone();
        let backdrop_owner = owner.clone();
        let toggle_owner = owner.clone();
        let name = entry.name.clone();
        let built_in = name == "runner";
        let title = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().min_w_0().truncate().child(if built_in {
                "Runner".into()
            } else {
                name.clone()
            }))
            .child(Badge::new(
                transport_badge(entry, client),
                if built_in { Tone::Accent } else { Tone::Muted },
            ))
            .when(self.editing, |d| {
                d.child(Badge::new("editing", Tone::Accent))
            })
            .child(div().flex_1())
            .when(!self.editing && !built_in, |d| {
                d.child(
                    Button::new("mcp-modal-edit", "Edit")
                        .icon("square-pen.svg")
                        .size(ButtonSize::Sm)
                        .disabled(self.busy || !entry.clients[&client].registered)
                        .on_press(move |window, cx| {
                            edit_owner.update(cx, |this, cx| this.edit(window, cx))
                        }),
                )
            })
            .child(
                IconButton::new("mcp-modal-close", "close.svg")
                    .size(IconButtonSize::Sm)
                    .disabled(self.busy)
                    .on_press(move |window, cx| {
                        close_owner.update(cx, |this, cx| this.request_dismiss(window, cx))
                    }),
            );
        let path = PathBuf::from(&client.status(&self.catalog.as_ref()?.runner).config_path);
        let description = if built_in {
            RUNNER_DESCRIPTION
        } else {
            match definition_for(entry, client) {
                Some(McpServerDefinition::Stdio { .. }) => "The agent launches this command and communicates with the server over standard input and output.",
                Some(McpServerDefinition::Http { .. }) => "The agent connects to this server over HTTP using the configured URL and headers.",
                None => "This transport is managed by the agent's own tooling and cannot be copied to another agent.",
            }
        };
        let clients = registered_clients(entry);
        let registered = server_toggle_on(entry, runtime, &self.catalog.as_ref()?.runner);
        let copy_hint = copy_hint(entry, runtime);
        let overview =
            div()
                .id("mcp-overview")
                .debug_selector(|| "MCP_OVERVIEW".into())
                .flex()
                .flex_col()
                .gap_3()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scroll()
                .scrollbar_width(px(0.))
                .track_scroll(&self.overview_scroll)
                .child(
                    div()
                        .text_size(theme::text_ui())
                        .line_height(rems(18. / 16.))
                        .text_color(theme::muted())
                        .child(description),
                )
                .child(
                    div()
                        .debug_selector(|| "MCP_PATH_ROW".into())
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .child(
                            div()
                                .debug_selector(|| "MCP_PATH".into())
                                .flex_1()
                                .min_w_0()
                                .text_size(theme::text_caption())
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_color(theme::faint())
                                .child(format!(
                                    "{} → {}",
                                    client.config_file(),
                                    client.entry_key(&name)
                                )),
                        )
                        .child(
                            Button::new("mcp-reveal", "Reveal config")
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_press(move |_, cx| cx.reveal_path(&path)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .text_size(theme::text_caption())
                        .text_color(theme::faint())
                        .child("Registered in")
                        .children(clients.iter().map(|c| Badge::new(c.label(), Tone::Muted)))
                        .when(clients.is_empty(), |d| d.child("No agents")),
                )
                .when(!self.editing, |d| {
                    d.child(
                        div()
                            .debug_selector(|| "MCP_REGISTERED_ROW".into())
                            .flex()
                            .items_center()
                            .gap_4()
                            .rounded(rems(6. / 16.))
                            .bg(theme::raised())
                            .px_3()
                            .py_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(theme::text_ui())
                                            .child(format!("Registered in {}", runtime.label())),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme::text_caption())
                                            .line_height(rems(15. / 16.))
                                            .text_color(theme::faint())
                                            .child(format!(
                                                "Writes only {} in {}. Applies to new sessions.",
                                                runtime.entry_key(&name),
                                                runtime.config_file()
                                            )),
                                    )
                                    .children(copy_hint.clone().map(|hint| {
                                        div()
                                            .text_size(theme::text_caption())
                                            .text_color(theme::warning())
                                            .child(hint)
                                    })),
                            )
                            .child(
                                Toggle::new("mcp-detail-toggle", registered)
                                    .disabled(self.busy || copy_hint.is_some())
                                    .on_change(move |enabled, _, cx| {
                                        toggle_owner.update(cx, |this, cx| {
                                            this.set_enabled(runtime, name.clone(), enabled, cx)
                                        })
                                    }),
                            ),
                    )
                })
                .when(self.editing, |d| {
                    d.children(
                        clients
                            .iter()
                            .copied()
                            .filter(|&other| other != client)
                            .enumerate()
                            .map(|(i, other)| {
                                let owner = owner.clone();
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_4()
                                    .rounded(rems(6. / 16.))
                                    .bg(theme::raised())
                                    .px_3()
                                    .py_2()
                                    .child(
                                        div().flex_1().min_w_0().text_size(theme::text_ui()).child(
                                            format!("Also update {}'s entry", other.label()),
                                        ),
                                    )
                                    .child(
                                        Toggle::new(("mcp-also", i), self.also.contains(&other))
                                            .disabled(self.busy)
                                            .on_change(move |enabled, _, cx| {
                                                owner.update(cx, |this, cx| {
                                                    if enabled {
                                                        this.also.insert(other);
                                                    } else {
                                                        this.also.remove(&other);
                                                    }
                                                    cx.notify();
                                                })
                                            }),
                                    )
                            }),
                    )
                });
        let bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .flex_none()
            .pb_2()
            .when(self.editing, |d| {
                d.child(
                    div()
                        .text_size(theme::text_caption())
                        .text_color(theme::faint())
                        .child(format!(
                            "{} · {}",
                            client.label(),
                            if client == McpClientId::ClaudeCode {
                                "JSON"
                            } else {
                                "TOML"
                            }
                        )),
                )
            })
            .when(!self.editing, |d| {
                d.children(clients.iter().copied().enumerate().map(|(i, other)| {
                    let owner = owner.clone();
                    Button::new(("mcp-source", i), other.label())
                        .size(ButtonSize::Sm)
                        .variant(if other == client {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Ghost
                        })
                        .disabled(self.busy)
                        .on_press(move |_, cx| {
                            owner.update(cx, |this, cx| {
                                this.viewing = other;
                                this.scroll.set_offset(gpui::point(px(0.), px(0.)));
                                cx.notify();
                            })
                        })
                }))
            });
        let document = if self.editing {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(self.editor.clone())
                .into_any_element()
        } else {
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .rounded(rems(4. / 16.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::bg())
                .child(
                    div()
                        .id("mcp-document-scroll")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_y_scroll()
                        .scrollbar_width(px(0.))
                        .track_scroll(&self.scroll)
                        .p_3()
                        .child(
                            div()
                                .debug_selector(|| "MCP_NATIVE_TEXT".into())
                                .flex_none()
                                .w_full()
                                .min_w_0()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_ui())
                                .line_height(rems(20. / 16.))
                                .child(if entry.clients[&client].registered {
                                    entry.clients[&client].native_text.clone()
                                } else {
                                    "Not registered in this runtime.".into()
                                }),
                        ),
                )
                .child(self.scrollbar.clone())
                .into_any_element()
        };
        let error = self
            .error
            .clone()
            .or_else(|| self.editing.then(|| self.validation(cx).err()).flatten())
            .or_else(|| entry.clients[&client].error.clone());
        let body = div()
            .debug_selector(|| "MCP_MODAL_BODY".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_3()
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .min_w_0()
                    .max_h(gpui::relative(0.45))
                    .child(overview),
            )
            .children(error.map(|error| {
                div()
                    .flex_none()
                    .text_size(theme::text_caption())
                    .text_color(theme::danger())
                    .child(error)
            }))
            .child(
                div()
                    .debug_selector(|| "MCP_DOCUMENT_PANEL".into())
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(bar)
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
            let cancel = owner.clone();
            let save = owner.clone();
            modal = modal.footer(
                div()
                    .debug_selector(|| "MCP_FOOTER".into())
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .debug_selector(|| "MCP_FOOTER_HINT".into())
                            .flex_1()
                            .min_w_0()
                            .text_size(theme::text_caption())
                            .text_color(theme::faint())
                            .child(EDIT_FOOTER),
                    )
                    .child(
                        Button::new("mcp-edit-cancel", "Cancel")
                            .disabled(self.busy)
                            .on_press(move |window, cx| {
                                cancel.update(cx, |this, cx| this.request_dismiss(window, cx))
                            }),
                    )
                    .child(
                        Button::new("mcp-edit-save", "Save")
                            .variant(ButtonVariant::Primary)
                            .disabled(!self.can_save(cx))
                            .on_press(move |_, cx| save.update(cx, |this, cx| this.save(cx))),
                    ),
            );
        }
        Some(modal.into_any_element())
    }
}

impl Render for McpDetail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.entry().is_none() {
            return div().into_any_element();
        }
        let confirm = cx.entity();
        let cancel = confirm.clone();
        div()
            .absolute()
            .inset_0()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = &event.keystroke;
                if key.key == "escape" {
                    cx.stop_propagation();
                    if this.confirming {
                        this.cancel_discard(window, cx);
                    } else {
                        this.request_dismiss(window, cx);
                    }
                } else if key.key == "s"
                    && if cfg!(target_os = "macos") {
                        key.modifiers.platform
                    } else {
                        key.modifiers.control
                    }
                {
                    cx.stop_propagation();
                    this.save(cx);
                }
            }))
            .children(self.render_modal(cx))
            .when(self.confirming, |d| {
                d.child(
                    ConfirmDialog::new(
                        "Discard unsaved changes?",
                        "Your edits to this server have not been saved.",
                        "Discard changes",
                        "Discarding…",
                        false,
                        Rc::new(move |window, cx| {
                            confirm.update(cx, |this, cx| this.discard_edit(window, cx))
                        }),
                        Rc::new(move |window, cx| {
                            cancel.update(cx, |this, cx| this.cancel_discard(window, cx))
                        }),
                    )
                    .icon("square-pen.svg"),
                )
            })
            .into_any_element()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpRowTone {
    Muted,
    Accent,
    Warning,
    Danger,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct McpRowPresentation {
    tone: McpRowTone,
    status: String,
    disabled: bool,
    error: Option<String>,
}

fn mcp_row_presentation(
    client: McpClientId,
    status: Option<&McpClientStatus>,
    busy: bool,
    installed: bool,
) -> McpRowPresentation {
    let mut presentation = mcp_status_presentation(client, status, busy);
    // Nothing to register for an agent that is not on this machine.
    presentation.disabled |= !installed;
    presentation
}

fn mcp_status_presentation(
    client: McpClientId,
    status: Option<&McpClientStatus>,
    busy: bool,
) -> McpRowPresentation {
    let Some(status) = status else {
        return McpRowPresentation {
            tone: McpRowTone::Muted,
            status: "Checking".into(),

            disabled: true,
            error: None,
        };
    };
    if busy {
        return McpRowPresentation {
            tone: if status.error.is_some() {
                McpRowTone::Danger
            } else if status.matches_current {
                McpRowTone::Accent
            } else if status.registered {
                McpRowTone::Warning
            } else {
                McpRowTone::Muted
            },
            status: "Updating".into(),

            disabled: true,
            error: status.error.clone(),
        };
    }
    if let Some(error) = status.error.clone() {
        return McpRowPresentation {
            tone: McpRowTone::Danger,
            status: "Config error".into(),

            disabled: false,
            error: Some(error),
        };
    }
    if !status.registered {
        return McpRowPresentation {
            tone: McpRowTone::Muted,
            status: "Not registered".into(),

            disabled: false,
            error: None,
        };
    }
    if status.matches_current {
        return McpRowPresentation {
            tone: McpRowTone::Accent,
            status: format!("Registered in {}", client.config_file()),

            disabled: false,
            error: None,
        };
    }
    McpRowPresentation {
        tone: McpRowTone::Warning,
        status: format!(
            "Registered to another Runner · {}",
            configured_command(status)
        ),

        disabled: false,
        error: None,
    }
}

fn configured_command(status: &McpClientStatus) -> String {
    let mut command = status
        .command
        .clone()
        .unwrap_or_else(|| "(missing command)".into());
    for arg in &status.args {
        command.push(' ');
        command.push_str(arg);
    }
    command
}

fn row_color(tone: McpRowTone) -> gpui::Hsla {
    match tone {
        McpRowTone::Muted => theme::faint(),
        McpRowTone::Accent => theme::accent(),
        McpRowTone::Warning => theme::warning(),
        McpRowTone::Danger => theme::danger(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_backend::ops::mcp::McpServerClientEntry;
    use std::collections::BTreeMap;
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
                AppSettings::default(),
                None,
                cx,
            )
        })
    }

    fn mcp_status(registered: bool, matches_current: bool, error: Option<&str>) -> McpClientStatus {
        McpClientStatus {
            registered,
            matches_current,
            command: Some("/other/runner-mcp".into()),
            args: vec!["--stdio".into()],
            config_path: "/tmp/config".into(),
            error: error.map(str::to_owned),
        }
    }

    #[test]
    fn derives_each_mcp_row_state() {
        let checking = mcp_row_presentation(McpClientId::Codex, None, false, true);
        assert_eq!(checking.status, "Checking");
        assert_eq!(checking.tone, McpRowTone::Muted);
        assert!(checking.disabled);

        let missing = mcp_row_presentation(
            McpClientId::Codex,
            Some(&mcp_status(false, false, None)),
            false,
            true,
        );
        assert_eq!(missing.status, "Not registered");
        assert_eq!(missing.tone, McpRowTone::Muted);

        assert!(!missing.disabled);
        assert_eq!(missing.error, None);

        let current = mcp_row_presentation(
            McpClientId::ClaudeCode,
            Some(&mcp_status(true, true, None)),
            false,
            true,
        );
        assert_eq!(current.status, "Registered in ~/.claude.json");
        assert_eq!(current.tone, McpRowTone::Accent);

        assert!(!current.disabled);
        assert_eq!(
            mcp_row_presentation(
                McpClientId::Trae,
                Some(&mcp_status(true, true, None)),
                false,
                true
            )
            .status,
            "Registered in ~/.trae/traecli.toml"
        );

        let other = mcp_row_presentation(
            McpClientId::Codex,
            Some(&mcp_status(true, false, None)),
            false,
            true,
        );
        assert_eq!(
            other.status,
            "Registered to another Runner · /other/runner-mcp --stdio"
        );
        assert_eq!(other.tone, McpRowTone::Warning);

        let broken = mcp_row_presentation(
            McpClientId::Codex,
            Some(&mcp_status(false, false, Some("bad config"))),
            false,
            true,
        );
        assert_eq!(broken.status, "Config error");
        assert_eq!(broken.tone, McpRowTone::Danger);

        assert!(!broken.disabled);
        assert_eq!(broken.error.as_deref(), Some("bad config"));

        let updating = mcp_row_presentation(
            McpClientId::Codex,
            Some(&mcp_status(true, true, None)),
            true,
            true,
        );
        assert_eq!(updating.status, "Updating");
        assert_eq!(updating.tone, McpRowTone::Accent);

        assert!(updating.disabled);
        let updating_error = mcp_row_presentation(
            McpClientId::Codex,
            Some(&mcp_status(false, false, Some("bad config"))),
            true,
            true,
        );
        assert_eq!(updating_error.tone, McpRowTone::Danger);
        assert_eq!(updating_error.error.as_deref(), Some("bad config"));

        let not_installed = mcp_row_presentation(
            McpClientId::Codex,
            Some(&mcp_status(false, false, None)),
            false,
            false,
        );
        assert_eq!(not_installed.status, "Not registered");

        assert!(not_installed.disabled);
    }

    fn entry(name: &str, conflict: bool) -> McpServerEntry {
        let clients = McpClientId::ALL
            .into_iter()
            .map(|client| {
                let command = if client == McpClientId::Codex && conflict {
                    "different"
                } else {
                    "gh-mcp"
                };
                let definition = McpServerDefinition::Stdio {
                    command: command.into(),
                    args: vec![],
                    env: BTreeMap::from([("TOKEN".into(), "secret-value".into())]),
                };
                let registered = client != McpClientId::Trae;
                let text = if client == McpClientId::ClaudeCode {
                    serde_json::to_string_pretty(&definition.to_claude()).unwrap()
                } else {
                    format!(
                        "[mcp_servers.{name}]\ncommand = '{command}'\nstartup_timeout_sec = 60\n"
                    )
                };
                (
                    client,
                    McpServerClientEntry {
                        registered,
                        native_text: text,
                        definition: Some(definition),
                        conflicting: conflict && registered,
                        error: None,
                    },
                )
            })
            .collect();
        McpServerEntry {
            name: name.into(),
            clients,
        }
    }

    fn catalog() -> McpCatalog {
        McpCatalog {
            runner: McpIntegrationStatus {
                environment: "test".into(),
                binary_path: "runner".into(),
                endpoint: String::new(),
                claude_code: mcp_status(true, true, None),
                codex: mcp_status(true, true, None),
                trae: mcp_status(false, false, None),
            },
            runner_server: entry("runner", false),
            servers: vec![entry("aaa", false), entry("github", true)],
        }
    }

    fn runtimes() -> Vec<RuntimeCatalogEntry> {
        runner_backend::ops::runtime::runtime_list()
            .into_iter()
            .map(|r| RuntimeCatalogEntry {
                name: r.name,
                display_name: r.display_name,
                command: r.command,
                native_fork: r.native_fork,
                description: String::new(),
                default_enabled: true,
                available: r.name != runner_backend::model::Runtime::Trae,
                default_model: None,
                default_effort: None,
                models: vec![],
                efforts: vec![],
            })
            .collect()
    }

    struct Host(Entity<McpPane>);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(self.0.clone())
                .child(self.0.read(cx).detail.clone())
        }
    }

    #[test]
    fn catalog_rows_pin_runner_and_reflect_each_clients_registration_and_conflict() {
        let catalog = catalog();
        assert_eq!(
            catalog_rows(&catalog, "")
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["runner", "aaa", "github"]
        );
        assert_eq!(
            catalog_rows(&catalog, "GITHUB")
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["runner", "github"]
        );
        let github = &catalog.servers[1];
        assert_eq!(
            registered_clients(github),
            [McpClientId::ClaudeCode, McpClientId::Codex]
        );
        assert!(conflict_caption(github, McpClientId::ClaudeCode)
            .unwrap()
            .starts_with("Codex has a different definition:"));
        assert!(conflict_caption(github, McpClientId::Codex)
            .unwrap()
            .starts_with("Claude Code has a different definition:"));
        assert!(conflict_caption(&catalog.servers[0], McpClientId::ClaudeCode).is_none());
        assert_eq!(source_client(github), Some(McpClientId::ClaudeCode));
        let summary = definition_summary(definition_for(github, McpClientId::ClaudeCode));
        assert!(summary.contains("TOKEN=•••"));
        assert!(!summary.contains("secret-value"));
        let mut unsupported = entry("sse", false);
        unsupported
            .clients
            .get_mut(&McpClientId::ClaudeCode)
            .unwrap()
            .definition = None;
        assert!(copy_hint(&unsupported, McpClientId::Trae)
            .unwrap()
            .contains("TRAE CLI's CLI"));
        assert!(copy_hint(&unsupported, McpClientId::ClaudeCode).is_none());
    }

    #[test]
    fn runner_toggle_offers_repoint_when_another_installation_is_registered() {
        let mut catalog = catalog();
        let client = McpClientId::Codex;
        catalog.runner.codex = mcp_status(true, false, None);
        assert!(catalog.runner_server.clients[&client].registered);
        assert!(!server_toggle_on(
            &catalog.runner_server,
            client,
            &catalog.runner
        ));
        assert!(
            mcp_row_presentation(client, Some(&catalog.runner.codex), false, true)
                .status
                .starts_with("Registered to another Runner")
        );
        assert!(server_toggle_on(
            &catalog.servers[0],
            client,
            &catalog.runner
        ));
        catalog.runner.codex = mcp_status(true, true, None);
        assert!(server_toggle_on(
            &catalog.runner_server,
            client,
            &catalog.runner
        ));
        catalog.runner.codex = mcp_status(false, false, None);
        assert!(!server_toggle_on(
            &catalog.runner_server,
            client,
            &catalog.runner
        ));
        catalog
            .runner_server
            .clients
            .get_mut(&client)
            .unwrap()
            .registered = false;
        catalog
            .runner_server
            .clients
            .get_mut(&McpClientId::ClaudeCode)
            .unwrap()
            .definition = None;
        assert!(copy_hint(&catalog.runner_server, client).is_none());
    }

    #[test]
    fn runtime_dropdown_filters_unavailable_disabled_and_unknown_agents() {
        let mut settings = AppSettings::default();
        assert_eq!(
            available_clients(&runtimes(), &settings),
            [McpClientId::ClaudeCode, McpClientId::Codex]
        );
        settings.disabled_agents.insert("claude-code".into());
        assert_eq!(
            available_clients(&runtimes(), &settings),
            [McpClientId::Codex]
        );
    }

    #[test]
    fn conflict_detail_shows_each_native_body_and_blocks_invalid_save() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|_, cx| Host(cx.new(|cx| McpPane::new(store, cx))));
        host.update(&mut cx, |host, window, cx| {
            let detail = host.0.read(cx).detail.clone();
            detail.update(cx, |detail, cx| {
                detail.catalog = Some(catalog());
                detail.open(McpClientId::ClaudeCode, "github", window, cx);
                assert_eq!(detail.viewing, McpClientId::ClaudeCode);
                assert!(detail.entry().unwrap().clients[&detail.viewing]
                    .native_text
                    .contains("gh-mcp"));
                detail.viewing = McpClientId::Codex;
                assert!(detail.entry().unwrap().clients[&detail.viewing]
                    .native_text
                    .contains("different"));
                detail.edit(window, cx);
                assert!(detail
                    .editor
                    .read(cx)
                    .text()
                    .starts_with("[mcp_servers.github]"));
                assert!(detail.also.contains(&McpClientId::ClaudeCode));
                assert!(detail.can_save(cx));
                detail.editor.update(cx, |e, cx| e.reset("[broken", cx));
                assert!(!detail.can_save(cx));
                detail.save(cx);
                assert!(!detail.busy);
                detail.discard_edit(window, cx);
                detail.viewing = McpClientId::ClaudeCode;
                detail.edit(window, cx);
                assert!(detail.editor.read(cx).text().starts_with('{'));
                detail.editor.update(cx, |e, cx| e.reset("[]", cx));
                assert!(!detail.can_save(cx));
                detail.editor.update(cx, |e, cx| {
                    e.reset(r#"{"type":"sse","url":"https://example.test"}"#, cx)
                });
                assert!(!detail.can_save(cx));
                detail.also.clear();
                assert!(detail.can_save(cx));
            });
        })
        .unwrap();
    }

    #[test]
    fn cancel_edit_confirms_before_discard_and_runner_cannot_be_edited() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|_, cx| Host(cx.new(|cx| McpPane::new(store, cx))));
        host.update(&mut cx, |host, window, cx| {
            let detail = host.0.read(cx).detail.clone();
            detail.update(cx, |detail, cx| {
                detail.catalog = Some(catalog());
                detail.open(McpClientId::ClaudeCode, "github", window, cx);
                detail.edit(window, cx);
                let original = detail.original.clone();
                detail.request_dismiss(window, cx);
                assert!(!detail.editing);
                detail.edit(window, cx);
                detail.editor.update(cx, |e, cx| e.reset("unsaved", cx));
                detail.request_dismiss(window, cx);
                assert!(detail.confirming && detail.editing);
                detail.cancel_discard(window, cx);
                assert!(!detail.confirming && detail.editing);
                assert_eq!(detail.editor.read(cx).text(), "unsaved");
                detail.request_dismiss(window, cx);
                detail.discard_edit(window, cx);
                assert!(!detail.editing && !detail.confirming);
                assert_eq!(detail.editor.read(cx).text(), original);
                detail.request_dismiss(window, cx);
                assert!(detail.name.is_none());
                detail.open(McpClientId::ClaudeCode, "runner", window, cx);
                detail.edit(window, cx);
                assert!(!detail.editing);
            });
        })
        .unwrap();
    }

    #[test]
    fn catalog_removal_clears_edit_state_and_allows_opening_another_server() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|_, cx| Host(cx.new(|cx| McpPane::new(store, cx))));
        host.update(&mut cx, |host, window, cx| {
            let detail = host.0.read(cx).detail.clone();
            detail.update(cx, |detail, cx| {
                detail.apply_catalog(catalog());
                detail.open(McpClientId::ClaudeCode, "github", window, cx);
                detail.edit(window, cx);
                detail
                    .editor
                    .update(cx, |editor, cx| editor.reset("unsaved", cx));
                detail.request_dismiss(window, cx);
                assert!(detail.editing && detail.confirming);
                let mut next = catalog();
                next.servers.retain(|entry| entry.name != "github");
                detail.apply_catalog(next);
                assert!(detail.name.is_none());
                assert!(!detail.editing && !detail.confirming);
                detail.open(McpClientId::ClaudeCode, "aaa", window, cx);
                assert_eq!(detail.name.as_deref(), Some("aaa"));
                detail.edit(window, cx);
                assert!(detail.can_save(cx));
                assert_ne!(detail.editor.read(cx).text(), "unsaved");
            });
        })
        .unwrap();
    }

    #[test]
    fn manual_runner_choice_persists_default_registration_marker() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let detail = cx.new(|cx| McpDetail::new(store.clone(), cx));
        detail.update(&mut cx, |detail, cx| {
            detail.record_manual_choice(McpClientId::Codex, cx)
        });
        cx.run_until_parked();
        assert!(store.read_with(&cx, |store, _| store
            .settings
            .initialized_mcp_clients
            .contains("codex")));
        assert!(AppSettings::load(&temp.path().join("settings.json"))
            .unwrap()
            .initialized_mcp_clients
            .contains("codex"));
        detail.read_with(&cx, |detail, _| assert!(!detail.loading));
    }

    #[test]
    fn busy_refresh_queues_once_and_stale_catalogs_do_not_overwrite_newer_state() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let detail = cx.new(|cx| McpDetail::new(store, cx));
        detail.update(&mut cx, |detail, cx| {
            detail.refresh(cx);
            detail.refresh(cx);
            assert!(detail.loading && detail.refresh_pending);
            assert_eq!(detail.generation, 1);
        });
        cx.run_until_parked();
        detail.read_with(&cx, |detail, _| {
            assert!(!detail.loading && !detail.refresh_pending);
            assert_eq!(detail.generation, 2);
            assert!(detail.catalog.is_some());
        });
        detail.update(&mut cx, |detail, cx| {
            detail.catalog = None;
            detail.refresh(cx);
            detail.generation += 1;
        });
        cx.run_until_parked();
        detail.read_with(&cx, |detail, _| {
            assert!(!detail.loading);
            assert!(detail.catalog.is_none());
        });
    }

    #[test]
    fn native_text_and_edit_footer_wrap_inside_modal_at_two_sizes_and_zoom_levels() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|_, cx| Host(cx.new(|cx| McpPane::new(store, cx))));
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        for editing in [false, true] {
            host.update(&mut visual, |host, window, cx| {
                let detail = host.0.read(cx).detail.clone();
                detail.update(cx, |detail, cx| {
                    let mut catalog = catalog();
                    catalog.servers[1]
                        .clients
                        .get_mut(&McpClientId::ClaudeCode)
                        .unwrap()
                        .native_text = format!(
                        "{{\n  \"command\": \"{}\"\n}}",
                        "very-long-path-component/".repeat(60)
                    );
                    detail.catalog = Some(catalog);
                    detail.open(McpClientId::ClaudeCode, "github", window, cx);
                    if editing {
                        detail.edit(window, cx);
                    }
                });
            })
            .unwrap();
            for rem in [16., 20.8] {
                host.update(&mut visual, |_, window, _| window.set_rem_size(px(rem)))
                    .unwrap();
                for size in [
                    gpui::size(px(800.), px(600.)),
                    gpui::size(px(1440.), px(1000.)),
                ] {
                    visual.simulate_resize(size);
                    visual.run_until_parked();
                    let body = visual.debug_bounds("MCP_MODAL_BODY").unwrap();
                    let overview = visual.debug_bounds("MCP_OVERVIEW").unwrap();
                    let panel = visual.debug_bounds("MCP_DOCUMENT_PANEL").unwrap();
                    assert!((body.size.width - overview.size.width).abs() < px(3.));
                    assert!(panel.size.height > px(40.), "{editing} {rem}: {panel:?}");
                    assert!(panel.right() <= body.right());
                    if editing {
                        let footer = visual.debug_bounds("MCP_FOOTER").unwrap();
                        let hint = visual.debug_bounds("MCP_FOOTER_HINT").unwrap();
                        assert!(hint.size.height >= px(rem * 10. / 16. * 2.));
                        assert!(hint.right() < footer.right());
                        assert!(panel.bottom() <= footer.top() && footer.bottom() < size.height);
                    } else {
                        let text = visual.debug_bounds("MCP_NATIVE_TEXT").unwrap();
                        assert!(text.right() <= panel.right());
                        assert!(
                            text.size.height > px(rem * 20. / 16. * 5.),
                            "Long mono line did not wrap: {text:?}"
                        );
                    }
                }
            }
        }
    }
}
