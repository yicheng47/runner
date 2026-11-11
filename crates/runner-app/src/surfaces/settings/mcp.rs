use std::collections::BTreeSet;
use std::rc::Rc;

use super::SaveNotice;
use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, Context, Entity, EventEmitter, FocusHandle, FontWeight,
    KeyDownEvent, Render, ScrollHandle, Subscription, Window,
};
use runner_app::ui::{
    Badge, Button, ButtonSize, ButtonVariant, ConfirmDialog, IconButton, IconButtonSize, Modal,
    OverlayWidth, PaneHeader, Scrollbar, SelectOption, SettingsCard, StyledSelect, TextField,
    Toggle, Tone,
};
use runner_core::protocol::mcp::{
    McpCatalog, McpClientId, McpServerClientEntry, McpServerDefinition, McpServerEntry,
};
use runner_core::protocol::runtime::RuntimeCatalogEntry;

use crate::app_settings::AppSettings;
use crate::app_store::AppStore;
use crate::theme;

const CAPTION: &str =
    "ui.toggles_register_or_unregister_a_server_in_this_runtime_s_config_runner_writes_only_the_na";
const EDIT_FOOTER: &str =
    "ui.only_the_named_entry_changes_in_each_file_invalid_json_or_toml_cannot_be_saved_to_rename_a";

fn registered_clients(entry: &McpServerEntry) -> Vec<McpClientId> {
    McpClientId::all()
        .into_iter()
        .filter(|client| entry.clients.get(client).is_some_and(|e| e.registered))
        .collect()
}

fn source_client(entry: &McpServerEntry) -> Option<McpClientId> {
    // Copy fallback priority is independent of the runtime menu order.
    let mut clients = McpClientId::all();
    clients.sort();
    clients.into_iter().find(|client| {
        entry
            .clients
            .get(client)
            .is_some_and(|slot| slot.registered)
    })
}

fn server_toggle_on(entry: &McpServerEntry, client: McpClientId) -> bool {
    entry
        .clients
        .get(&client)
        .is_some_and(|slot| slot.registered)
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
    match definition_for(entry, client) {
        Some(McpServerDefinition::Stdio { .. }) => "stdio",
        Some(McpServerDefinition::Http { .. }) => "http",
        None => crate::i18n::text("word.other"),
    }
}

fn definition_summary(definition: Option<&McpServerDefinition>) -> String {
    let Some(definition) = definition else {
        return crate::i18n::text("ui.transport_managed_by_the_agent_s_cli").into();
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
    let shown = secrets.keys().take(2);
    for key in shown {
        text.push_str(&format!(" · {key}=•••"));
    }
    let hidden = secrets.len().saturating_sub(2);
    if hidden > 0 {
        let noun = match definition {
            McpServerDefinition::Stdio { .. } => "mcp.hidden_env",
            McpServerDefinition::Http { .. } => "mcp.hidden_headers",
        };
        text.push_str(&crate::i18n::tf(noun, &[("count", hidden.to_string())]));
    }
    text
}

/// A parser error spans several lines (a caret diagram for TOML); the modal
/// shows its first and last lines, which carry the position and the reason.
fn one_line_error(error: &str) -> String {
    let mut lines = error.lines().map(str::trim).filter(|l| !l.is_empty());
    let Some(first) = lines.next() else {
        return error.to_owned();
    };
    match lines.next_back() {
        Some(last) if last != first => format!("{first} — {last}"),
        _ => first.to_owned(),
    }
}

fn same_definition(a: &McpServerClientEntry, b: &McpServerClientEntry) -> bool {
    a.definition == b.definition && (a.definition.is_some() || a.native_text == b.native_text)
}

fn configured_differently(entry: &McpServerEntry) -> bool {
    entry
        .clients
        .values()
        .any(|slot| slot.registered && slot.conflicting)
}

fn copy_hint(entry: &McpServerEntry, client: McpClientId) -> Option<String> {
    if entry.clients.get(&client).is_some_and(|s| s.registered) {
        return None;
    }
    let source = source_client(entry)?;
    let copyable = match &entry.clients[&source].definition {
        None => false,
        // Antigravity CLI's HTTP entry shape is unprobed (#644); the backend refuses it.
        Some(McpServerDefinition::Http { .. }) => client.supports_http(),
        Some(McpServerDefinition::Stdio { .. }) => true,
    };
    (!copyable).then(|| {
        crate::i18n::tf(
            "format.this_transport_cannot_be_copied_add_it_with_s_cli",
            &[("runtime", (client.label()).to_string())],
        )
    })
}

fn available_clients(catalog: &[RuntimeCatalogEntry], settings: &AppSettings) -> Vec<McpClientId> {
    McpClientId::all()
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
    catalog
        .servers
        .iter()
        .filter(|entry| entry.name.to_lowercase().contains(&query))
        .collect()
}

pub(crate) struct McpPane {
    app_store: Entity<AppStore>,
    runtime: Option<McpClientId>,
    runtime_select: Entity<StyledSelect>,
    scoped: bool,
    account_select: Entity<StyledSelect>,
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
            .width(px(200.))
        });
        let account_owner = cx.weak_entity();
        let account_select = cx.new(|cx| {
            StyledSelect::new(
                "integration-account",
                cx.focus_handle(),
                "",
                Vec::new(),
                Rc::new(move |id, _, cx| {
                    let _ = account_owner.update(cx, |this, cx| this.select_account(id, cx));
                }),
                cx,
            )
            .width(px(200.))
        });
        let search = cx.new(|cx| {
            TextField::new(
                cx.focus_handle(),
                "",
                crate::i18n::message("ui.search_servers"),
                false,
            )
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
            scoped: false,
            account_select,
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
        if !self.scoped
            && !self
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

    pub(crate) fn is_composing(&self, cx: &gpui::App) -> bool {
        self.search.read(cx).is_composing() || self.detail.read(cx).editor.read(cx).is_composing()
    }
    pub(crate) fn count(&self, cx: &gpui::App) -> usize {
        self.detail
            .read(cx)
            .catalog
            .as_ref()
            .map_or(0, |c| c.servers.len())
    }
    pub(crate) fn scope(&mut self, runtime: runner_core::Runtime, cx: &mut Context<Self>) {
        self.scoped = true;
        self.runtime = McpClientId::for_runtime(runtime);
        self.search.update(cx, |search, cx| search.reset("", cx));
        let id = self.app_store.read(cx).claude_accounts.main_id.clone();
        self.select_account(id, cx);
    }
    fn select_account(&mut self, id: String, cx: &mut Context<Self>) {
        self.detail.update(cx, |detail, _| {
            detail.account_id = (!id.is_empty()).then_some(id.clone());
            detail.generation += 1;
            detail.catalog = None;
        });
        let select = self.account_select.clone();
        cx.defer(move |cx| select.update(cx, |select, cx| select.set_value(id, cx)));
        self.refresh(cx);
        cx.notify();
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
        let registered = server_toggle_on(entry, client);
        let hint = copy_hint(entry, client);
        let error = entry.clients.get(&client).and_then(|s| s.error.clone());
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
                                    .child(entry.name.clone()),
                            )
                            .child(Badge::new(transport_badge(entry, client), Tone::Muted)),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .child(definition_summary(definition_for(entry, client))),
                    )
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
                    .disabled(state.busy || hint.is_some())
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
        let accounts = self.app_store.read(cx).claude_accounts.clone();
        self.account_select.update(cx, |select, cx| {
            select.set_options(super::integration_account::options(&accounts), cx)
        });
        let shared = (self.runtime == Some(McpClientId::ClaudeCode))
            .then(|| {
                super::integration_account::shared_note(
                    &accounts,
                    self.detail.read(cx).account_id.as_deref(),
                    true,
                )
            })
            .flatten();
        let owner = cx.entity();
        let refresh_owner = owner.clone();
        let state = self.detail.read(cx);
        let mut rows = Vec::new();
        let mut meta = None;
        if let (Some(catalog), Some(client)) = (&state.catalog, self.runtime) {
            rows = catalog_rows(catalog, self.search.read(cx).text())
                .into_iter()
                .enumerate()
                .map(|(i, entry)| self.row(entry, client, i, cx))
                .collect();
            let off = catalog
                .servers
                .iter()
                .filter(|entry| !server_toggle_on(entry, client))
                .count();
            let config_file = if client == McpClientId::ClaudeCode {
                let accounts = &self.app_store.read(cx).claude_accounts;
                let id = state.account_id.as_deref().unwrap_or(&accounts.main_id);
                let id = if accounts.sharing.get(id).is_some_and(|s| s.using_main) {
                    accounts.main_id.as_str()
                } else {
                    id
                };
                accounts
                    .accounts
                    .iter()
                    .find(|a| a.id == id)
                    .map(|a| {
                        if a.native {
                            self.app_store
                                .read(cx)
                                .home_dir
                                .as_ref()
                                .map(|home| home.join(".claude.json").display().to_string())
                                .unwrap_or_default()
                        } else {
                            std::path::Path::new(&a.folder)
                                .join(".claude.json")
                                .display()
                                .to_string()
                        }
                    })
                    .unwrap_or_else(|| client.config_file().to_owned())
            } else {
                client.config_file().to_owned()
            };
            meta = Some(crate::i18n::tf(
                "format.servers_off",
                &[
                    ("config_file", config_file.clone()),
                    ("server_count", (catalog.servers.len()).to_string()),
                    ("off", (off).to_string()),
                ],
            ));
            if self.search.read(cx).text().trim().is_empty()
                && !catalog
                    .servers
                    .iter()
                    .any(|s| s.clients[&client].registered)
            {
                rows.push(div().px_4().py_4().flex().flex_col().gap_2().text_size(theme::text_ui()).text_color(theme::faint())
                    .child(crate::i18n::tf("format.no_servers_in_yet", &[("config_file", config_file.clone())]))
                    .child(crate::i18n::tf(if self.scoped { "integration.add_servers" } else { "format.add_servers_with_s_cli_or_switch_runtime_to_copy_a_server_another_agent_alread" }, &[("runtime", (client.label()).to_string())])).into_any_element());
            } else if rows.is_empty() && !self.search.read(cx).text().trim().is_empty() {
                rows.push(
                    div()
                        .p_4()
                        .text_size(theme::text_ui())
                        .text_color(theme::faint())
                        .child(crate::i18n::text("ui.no_matching_servers"))
                        .into_any_element(),
                );
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_4()
            .when(!self.scoped, |pane| {
                pane.child(
                    PaneHeader::new("MCP", "").action(
                        Button::new("mcp-refresh", crate::i18n::text("ui.refresh"))
                            .icon("refresh-cw.svg")
                            .size(ButtonSize::Sm)
                            .disabled(state.loading || state.busy)
                            .on_press(move |_, cx| owner.update(cx, |this, cx| this.refresh(cx))),
                    ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(!self.scoped, |row| row.child(self.runtime_select.clone()))
                    .when(
                        self.scoped
                            && self.runtime == Some(McpClientId::ClaudeCode)
                            && self.app_store.read(cx).claude_accounts.multiple(),
                        |row| row.child(self.account_select.clone()),
                    )
                    .child(div().flex_1().min_w_0().child(self.search.clone()))
                    .when(self.scoped, |row| {
                        row.child(
                            Button::new("integration-refresh", crate::i18n::text("ui.refresh"))
                                .icon("refresh-cw.svg")
                                .size(ButtonSize::Sm)
                                .on_press(move |_, cx| {
                                    refresh_owner.update(cx, |this, cx| this.refresh(cx))
                                }),
                        )
                    }),
            )
            .children(shared.map(|(note, label, id)| {
                let owner = cx.entity();
                div()
                    .debug_selector(|| "INTEGRATION_SHARING_NOTE".into())
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .debug_selector(|| "INTEGRATION_SHARING_TEXT".into())
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .line_height(rems(1.))
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(note),
                    )
                    .child(
                        div()
                            .debug_selector(|| "INTEGRATION_SHOW_MAIN".into())
                            .flex_none()
                            .child(
                                Button::new("integration-show-main", label)
                                    .size(ButtonSize::Sm)
                                    .on_press(move |_, cx| {
                                        owner.update(cx, |this, cx| {
                                            this.select_account(id.clone(), cx)
                                        })
                                    }),
                            ),
                    )
            }))
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
                            .child(crate::i18n::text(CAPTION)),
                    )
            }))
            .children(
                state
                    .error
                    .clone()
                    .filter(|_| !self.app_store.read(cx).daemon_disconnected)
                    .map(|error| {
                        runner_app::ui::notice_banner(error, runner_app::ui::Tone::Danger)
                    }),
            )
            .child(SettingsCard::new(if rows.is_empty() {
                vec![div()
                    .p_4()
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child(if state.loading {
                        crate::i18n::text("ui.loading_mcp_servers")
                    } else {
                        crate::i18n::text("ui.no_supported_enabled_agents_are_installed")
                    })
                    .into_any_element()]
            } else {
                rows
            }))
    }
}

impl EventEmitter<SaveNotice> for McpDetail {}

pub(crate) struct McpDetail {
    account_id: Option<String>,
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
        let subscriptions = vec![cx.observe(&editor, |_, _, cx| cx.notify())];
        Self {
            account_id: None,
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
        let home = self.app_store.read(cx).home_dir.clone();
        let core = self.app_store.read(cx).client.clone();
        let account_id = self.account_id.clone();
        let task = cx.background_spawn(async move {
            let home = home.ok_or_else(|| {
                crate::i18n::text("ui.home_directory_is_not_available").to_owned()
            })?;
            Ok::<_, String>((
                core.mcp_catalog(&home, account_id.as_deref())
                    .map_err(|e| e.to_string())?,
                core.runtime_catalog().map_err(|e| e.to_string())?,
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
        self.catalog
            .as_ref()?
            .servers
            .iter()
            .find(|entry| entry.name == name)
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
        self.focus.focus(window, cx);
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
        if enabled
            && self
                .find_entry(&name)
                .is_none_or(|entry| copy_hint(entry, client).is_some())
        {
            return;
        }
        let account_id = self.account_id.clone();
        self.write(
            move |daemon, home| {
                if enabled {
                    daemon.mcp_copy_server(
                        home,
                        source.ok_or_else(|| {
                            runner_core::protocol::ClientError::msg(
                                "Server is no longer registered",
                            )
                        })?,
                        client,
                        &name,
                        account_id.as_deref(),
                    )
                } else {
                    daemon.mcp_remove_server(home, client, &name, account_id.as_deref())
                }
            },
            None,
            cx,
        );
    }

    fn write(
        &mut self,
        operation: impl FnOnce(
                &runner_core::protocol::DaemonClient,
                &std::path::Path,
            ) -> std::result::Result<(), runner_core::protocol::ClientError>
            + Send
            + 'static,
        saved: Option<crate::i18n::Text>,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.error = None;
        self.generation += 1;
        let generation = self.generation;
        self.editor.update(cx, |e, cx| e.set_disabled(true, cx));
        let home = self.app_store.read(cx).home_dir.clone();
        let core = self.app_store.read(cx).client.clone();
        let account_id = self.account_id.clone();
        let task = cx.background_spawn(async move {
            let Some(home) = home else {
                let error = crate::i18n::text("ui.home_directory_is_not_available").to_owned();
                return (Err(error.clone()), Err(error));
            };
            let result = operation(&core, &home).map_err(|e| e.to_string());
            (
                result,
                core.mcp_catalog(&home, account_id.as_deref())
                    .map_err(|e| e.to_string()),
            )
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
                    match (&result, &saved) {
                        (Err(message), Some(_)) => cx.emit(SaveNotice {
                            message: message.clone().into(),
                            tone: crate::toast::ToastTone::Error,
                        }),
                        (Ok(()), Some(saved)) => cx.emit(SaveNotice {
                            message: saved.clone(),
                            tone: crate::toast::ToastTone::Success,
                        }),
                        _ => {}
                    }
                    this.error = match &saved {
                        Some(_) => None,
                        None => result.as_ref().err().cloned(),
                    }
                    .or_else(|| catalog.err());
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
        let Some(slot) = entry.clients.get(&self.viewing).filter(|e| e.registered) else {
            return;
        };
        let text = slot.native_text.clone();
        self.also = registered_clients(entry)
            .into_iter()
            .filter(|&c| c != self.viewing && same_definition(&entry.clients[&c], slot))
            .collect();
        self.original = text.clone();
        self.editor.update(cx, |editor, cx| editor.reset(text, cx));
        self.editing = true;
        self.error = None;
        self.editor.read(cx).focus_handle().focus(window, cx);
        cx.notify();
    }

    fn validation(&self, cx: &Context<Self>) -> Result<(), String> {
        let name = self
            .name
            .as_deref()
            .ok_or_else(|| crate::i18n::text("ui.no_server_selected").to_owned())?;
        self.app_store
            .read(cx)
            .client
            .mcp_validate_edit(
                self.viewing,
                name,
                self.editor.read(cx).text(),
                &self.also.iter().copied().collect::<Vec<_>>(),
            )
            .map_err(|e| e.to_string())
    }

    fn can_save(&self, _cx: &Context<Self>) -> bool {
        self.editing && !self.busy && !self.confirming
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.can_save(cx) {
            return;
        }
        if let Err(error) = self.validation(cx) {
            cx.emit(SaveNotice {
                message: one_line_error(&error).into(),
                tone: crate::toast::ToastTone::Error,
            });
            return;
        }
        let name = self
            .name
            .clone()
            .expect("validation above rejects an empty selection");
        let client = self.viewing;
        let text = self.editor.read(cx).text().to_owned();
        let also: Vec<_> = self.also.iter().copied().collect();
        let files = std::iter::once(client)
            .chain(also.iter().copied())
            .map(|c| c.config_file())
            .collect::<Vec<_>>()
            .join(crate::i18n::text("ui.and"));
        let saved = crate::i18n::messagef(
            "format.saved_to",
            &[("name", (name).to_string()), ("files", (files).to_string())],
        );
        let account_id = self.account_id.clone();
        self.write(
            move |daemon, home| {
                daemon.mcp_edit_server(home, client, &name, &text, &also, account_id.as_deref())
            },
            Some(saved),
            cx,
        );
    }

    fn request_dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.editing && self.editor.read(cx).text() != self.original {
            self.confirming = true;
            self.focus.focus(window, cx);
        } else if self.editing {
            self.discard_edit(window, cx);
        } else {
            self.name = None;
            if let Some(focus) = self.previous_focus.take() {
                focus.focus(window, cx);
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
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn cancel_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirming = false;
        self.editor.read(cx).focus_handle().focus(window, cx);
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
        let title = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().min_w_0().truncate().child(name.clone()))
            .child(Badge::new(transport_badge(entry, client), Tone::Muted))
            .when(self.editing, |d| {
                d.child(Badge::new(crate::i18n::text("word.editing"), Tone::Accent))
            })
            .child(div().flex_1())
            .when(!self.editing, |d| {
                d.child(
                    Button::new("mcp-modal-edit", crate::i18n::text("ui.edit"))
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
        let path = client.config_path(self.app_store.read(cx).home_dir.as_deref()?);
        let description = match definition_for(entry, client) {
            Some(McpServerDefinition::Stdio { .. }) => crate::i18n::text("ui.the_agent_launches_this_command_and_communicates_with_the_server_over_standard_input_and_o"),
            Some(McpServerDefinition::Http { .. }) => crate::i18n::text("ui.the_agent_connects_to_this_server_over_http_using_the_configured_url_and_headers"),
            None => crate::i18n::text("ui.this_transport_is_managed_by_the_agent_s_own_tooling_and_cannot_be_copied_to_another_agent"),
        };
        let clients = registered_clients(entry);
        let registered = server_toggle_on(entry, runtime);
        let copy_hint = copy_hint(entry, runtime);
        let overview = div()
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
                        Button::new("mcp-reveal", crate::i18n::text("ui.reveal_config"))
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
                    .child(crate::i18n::text("ui.registered_in"))
                    .children(clients.iter().map(|c| Badge::new(c.label(), Tone::Muted)))
                    .when(clients.is_empty(), |d| {
                        d.child(crate::i18n::text("ui.no_agents"))
                    })
                    .when(configured_differently(entry), |d| {
                        d.child(
                            div()
                                .debug_selector(|| "MCP_PER_AGENT_NOTE".into())
                                .child(crate::i18n::text("ui.configured_differently_per_agent")),
                        )
                    }),
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
                                .child(div().text_size(theme::text_ui()).child(crate::i18n::tf(
                                    "format.registered_in",
                                    &[("runtime", (runtime.label()).to_string())],
                                )))
                                .child(
                                    div()
                                        .text_size(theme::text_caption())
                                        .line_height(rems(15. / 16.))
                                        .text_color(theme::faint())
                                        .child(crate::i18n::tf(
                                            "format.writes_only_in_applies_to_new_sessions",
                                            &[
                                                ("entry", (runtime.entry_key(&name)).to_string()),
                                                (
                                                    "config_file",
                                                    (runtime.config_file()).to_string(),
                                                ),
                                            ],
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
                                .child(div().flex_1().min_w_0().text_size(theme::text_ui()).child(
                                    crate::i18n::tf(
                                        "format.also_update_s_entry",
                                        &[("runtime", (other.label()).to_string())],
                                    ),
                                ))
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
                            if client.is_json() { "JSON" } else { "TOML" }
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
                                    crate::i18n::text("ui.not_registered_in_this_runtime").into()
                                }),
                        ),
                )
                .child(self.scrollbar.clone())
                .into_any_element()
        };
        let error = (!self.editing)
            .then(|| self.error.clone())
            .flatten()
            .or_else(|| entry.clients[&client].error.clone())
            .filter(|_| !self.app_store.read(cx).daemon_disconnected);
        let body =
            div()
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
                    runner_app::ui::notice_banner(error, runner_app::ui::Tone::Danger)
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
                            .child(crate::i18n::text(EDIT_FOOTER)),
                    )
                    .child(
                        Button::new("mcp-edit-cancel", crate::i18n::text("ui.cancel"))
                            .disabled(self.busy)
                            .on_press(move |window, cx| {
                                cancel.update(cx, |this, cx| this.request_dismiss(window, cx))
                            }),
                    )
                    .child(
                        Button::new("mcp-edit-save", crate::i18n::text("ui.save"))
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
                        crate::i18n::text("ui.discard_unsaved_changes"),
                        crate::i18n::text("ui.your_edits_to_this_server_have_not_been_saved"),
                        crate::i18n::text("ui.discard_changes"),
                        crate::i18n::text("ui.discarding"),
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
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn test_store(path: &std::path::Path, cx: &mut gpui::TestAppContext) -> Entity<AppStore> {
        use runner_daemon::{db, session, shell_path};
        use std::sync::{Arc, RwLock};
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = crate::test_support::core(
            Arc::new(db::open_fixture_pool(&path.join("runner.db")).unwrap()),
            path.into(),
            session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
        );
        cx.new(|cx| {
            AppStore::new(
                core,
                Some(path.join("home")),
                None,
                path.join("settings.json"),
                AppSettings::default(),
                None,
                cx,
            )
        })
    }

    fn entry(name: &str, differs: bool) -> McpServerEntry {
        let clients = McpClientId::all()
            .into_iter()
            .map(|client| {
                let command = if client == McpClientId::Codex && differs {
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
                let text = if client.is_json() {
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
                        conflicting: differs && registered,
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
            servers: vec![
                entry("aaa", false),
                entry("github", true),
                entry("runner", false),
            ],
        }
    }

    fn runtimes() -> Vec<RuntimeCatalogEntry> {
        runner_core::protocol::runtime_metadata::runtime_list()
            .into_iter()
            .map(|r| RuntimeCatalogEntry {
                installed_version: None,
                available_version: None,
                min_version: None,
                below_minimum: false,
                name: r.name,
                capabilities: runner_core::protocol::runtime::RuntimeCatalogEntry::for_runtime(
                    r.name,
                )
                .map(|entry| entry.capabilities)
                .unwrap_or_default(),
                display_name: r.display_name,
                command: r.command,
                native_fork: r.native_fork,
                description: String::new(),
                install_url: String::new(),
                default_enabled: true,
                available: r.name != runner_core::protocol::model::Runtime::Trae,
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
    fn sharing_guidance_wraps_and_keeps_show_main_visible() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        store.update(&mut cx, |store, _| {
            store.claude_accounts = runner_core::protocol::ClaudeAccounts {
                accounts: vec![
                    runner_core::protocol::ClaudeAccount {
                        id: "main".into(),
                        name: "Personal".into(),
                        folder: temp.path().join("main").display().to_string(),
                        native: false,
                    },
                    runner_core::protocol::ClaudeAccount {
                        id: "shared".into(),
                        name: "Work account with a longer descriptive name".into(),
                        folder: temp.path().join("shared").display().to_string(),
                        native: false,
                    },
                ],
                main_id: "main".into(),
                active_id: "main".into(),
                sharing: std::collections::BTreeMap::from([(
                    "shared".into(),
                    runner_core::protocol::ClaudeAccountSharing {
                        using_main: true,
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            };
        });
        let host = cx.add_window(|_, cx| Host(cx.new(|cx| McpPane::new(store, cx))));
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        host.update(&mut visual, |host, _, cx| {
            host.0.update(cx, |pane, cx| {
                pane.scope(runner_core::Runtime::ClaudeCode, cx);
                pane.select_account("shared".into(), cx);
            })
        })
        .unwrap();
        for rem in [16., 19.2] {
            host.update(&mut visual, |_, window, _| window.set_rem_size(px(rem)))
                .unwrap();
            for width in [640., 900.] {
                visual.simulate_resize(gpui::size(px(width), px(800.)));
                visual.run_until_parked();
                let row = visual.debug_bounds("INTEGRATION_SHARING_NOTE").unwrap();
                let text = visual.debug_bounds("INTEGRATION_SHARING_TEXT").unwrap();
                let action = visual.debug_bounds("INTEGRATION_SHOW_MAIN").unwrap();
                assert!(row.right() <= px(width) + px(1.));
                assert!(text.size.height > px(rem * 16. / 16.));
                assert!(text.right() <= action.left());
                assert!(action.right() <= row.right() + px(1.));
                assert!(action.top() >= row.top() && action.bottom() <= row.bottom());
            }
        }
    }

    #[test]
    fn account_picker_keyboard_selection_defers_its_own_value_update() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        store.update(&mut cx, |store, _| {
            store.claude_accounts = runner_core::protocol::ClaudeAccounts {
                accounts: ["main", "fixture"]
                    .into_iter()
                    .map(|id| runner_core::protocol::ClaudeAccount {
                        id: id.into(),
                        name: id.into(),
                        folder: temp.path().join(id).display().to_string(),
                        native: false,
                    })
                    .collect(),
                main_id: "main".into(),
                active_id: "main".into(),
                ..Default::default()
            };
        });
        let host = cx.add_window(|_, cx| Host(cx.new(|cx| McpPane::new(store, cx))));
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        host.update(&mut visual, |host, window, cx| {
            host.0.update(cx, |pane, cx| {
                pane.scope(runner_core::Runtime::ClaudeCode, cx)
            });
            host.0
                .read(cx)
                .account_select
                .read(cx)
                .focus_handle()
                .focus(window, cx);
        })
        .unwrap();
        visual.run_until_parked();
        for key in ["space", "down", "enter", "space", "space"] {
            visual.simulate_keystrokes(key);
            visual.run_until_parked();
        }
        host.read_with(&visual, |host, cx| {
            let pane = host.0.read(cx);
            assert_eq!(pane.account_select.read(cx).value(), "fixture");
            assert_eq!(pane.detail.read(cx).account_id.as_deref(), Some("fixture"));
        })
        .unwrap();
        host.update(&mut visual, |host, _, cx| {
            host.0
                .update(cx, |pane, cx| pane.select_account("main".into(), cx))
        })
        .unwrap();
        visual.run_until_parked();
        host.read_with(&visual, |host, cx| {
            assert_eq!(host.0.read(cx).account_select.read(cx).value(), "main")
        })
        .unwrap();
    }

    #[test]
    fn pane_has_no_pinned_runner_row_and_renders_configured_runner_normally() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|_, cx| Host(cx.new(|cx| McpPane::new(store, cx))));
        host.update(&mut cx, |host, _, cx| {
            let detail = host.0.read(cx).detail.clone();
            detail.update(cx, |detail, cx| {
                let mut without_runner = catalog();
                without_runner
                    .servers
                    .retain(|entry| entry.name != "runner");
                detail.catalog = Some(without_runner);
                detail.runtimes = runtimes();
                cx.notify();
            });
        })
        .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(gpui::size(px(900.), px(700.)));
        visual.run_until_parked();
        assert!(visual.debug_bounds("MCP_ROW_runner").is_none());
        assert!(visual.debug_bounds("MCP_ROW_github").is_some());

        host.update(&mut visual, |host, _, cx| {
            let detail = host.0.read(cx).detail.clone();
            detail.update(cx, |detail, cx| {
                detail.catalog = Some(catalog());
                cx.notify();
            });
        })
        .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("MCP_ROW_runner").is_some());
        assert!(!crate::i18n::text(CAPTION).contains("Runner's own server"));
    }

    #[test]
    fn catalog_rows_filter_every_server_and_reflect_registration_and_per_agent_differences() {
        let catalog = catalog();
        assert_eq!(
            catalog_rows(&catalog, "")
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["aaa", "github", "runner"]
        );
        assert_eq!(
            catalog_rows(&catalog, "GITHUB")
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["github"]
        );
        let github = &catalog.servers[1];
        assert_eq!(
            registered_clients(github),
            [
                McpClientId::Codex,
                McpClientId::ClaudeCode,
                McpClientId::Antigravity,
                McpClientId::Copilot,
                McpClientId::Grok,
                McpClientId::OpenCode,
            ]
        );
        assert!(configured_differently(github));
        assert!(!configured_differently(&catalog.servers[0]));
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

        let mut http = entry("web", false);
        for client in McpClientId::all() {
            let slot = http.clients.get_mut(&client).unwrap();
            slot.registered = client == McpClientId::ClaudeCode;
            slot.definition = Some(McpServerDefinition::Http {
                url: "https://example.com/mcp".into(),
                headers: BTreeMap::new(),
            });
        }
        assert!(copy_hint(&http, McpClientId::Antigravity)
            .unwrap()
            .contains("Antigravity CLI's CLI"));
        assert!(copy_hint(&http, McpClientId::Copilot).is_none());
    }

    #[test]
    fn runner_is_an_ordinary_editable_server() {
        let mut catalog = catalog();
        let client = McpClientId::Codex;
        let runner = catalog
            .servers
            .iter_mut()
            .find(|entry| entry.name == "runner")
            .unwrap();
        assert!(server_toggle_on(runner, client));
        runner.clients.get_mut(&client).unwrap().registered = false;
        runner
            .clients
            .get_mut(&McpClientId::ClaudeCode)
            .unwrap()
            .definition = None;
        assert!(!server_toggle_on(runner, client));
        assert!(copy_hint(runner, client).is_some());
        assert_eq!(transport_badge(runner, McpClientId::ClaudeCode), "other");
    }

    #[test]
    fn runtime_dropdown_filters_unavailable_disabled_and_unknown_agents() {
        let mut settings = AppSettings::default();
        let mut all_runtimes = runtimes();
        all_runtimes
            .iter_mut()
            .find(|runtime| runtime.name == runner_core::protocol::model::Runtime::Trae)
            .unwrap()
            .available = true;
        assert_eq!(
            available_clients(&all_runtimes, &settings),
            [
                McpClientId::Codex,
                McpClientId::ClaudeCode,
                McpClientId::Antigravity,
                McpClientId::Copilot,
                McpClientId::Trae,
                McpClientId::Grok,
                McpClientId::OpenCode,
            ]
        );
        assert_eq!(
            available_clients(&runtimes(), &settings),
            [
                McpClientId::Codex,
                McpClientId::ClaudeCode,
                McpClientId::Antigravity,
                McpClientId::Copilot,
                McpClientId::Grok,
                McpClientId::OpenCode,
            ]
        );
        settings.disabled_agents.insert("claude-code".into());
        settings.disabled_agents.insert("antigravity".into());
        assert_eq!(
            available_clients(&runtimes(), &settings),
            [
                McpClientId::Codex,
                McpClientId::Copilot,
                McpClientId::Grok,
                McpClientId::OpenCode
            ]
        );
    }

    #[test]
    fn differing_detail_shows_each_native_body_and_blocks_invalid_save() {
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
                assert!(
                    detail.also.is_empty(),
                    "an agent configured differently is not updated by default"
                );
                assert!(detail.can_save(cx));
                detail.editor.update(cx, |e, cx| e.reset("[broken", cx));
                assert!(detail.can_save(cx), "typing never disables Save");
                detail.save(cx);
                assert!(
                    !detail.busy && detail.editing,
                    "an invalid Save writes nothing"
                );
                assert!(detail.error.is_none(), "the parse error goes to a toast");
                assert_eq!(detail.editor.read(cx).text(), "[broken");
                let error = one_line_error(&detail.validation(cx).unwrap_err());
                assert!(error.starts_with("Invalid TOML"), "{error}");
                assert_eq!(error.lines().count(), 1, "{error}");
                detail.discard_edit(window, cx);
                detail.viewing = McpClientId::ClaudeCode;
                detail.edit(window, cx);
                assert!(detail.editor.read(cx).text().starts_with('{'));
                assert_eq!(
                    detail.also,
                    BTreeSet::from([
                        McpClientId::Antigravity,
                        McpClientId::Copilot,
                        McpClientId::Grok,
                        McpClientId::OpenCode,
                    ])
                );
                detail.editor.update(cx, |e, cx| e.reset("[]", cx));
                detail.save(cx);
                assert!(detail.error.is_none() && !detail.busy && detail.editing);
                detail.editor.update(cx, |e, cx| {
                    e.reset(r#"{"type":"sse","url":"https://example.test"}"#, cx)
                });
                assert!(detail
                    .validation(cx)
                    .is_err_and(|e| e.contains("cannot be translated")));
                detail.also.clear();
                assert!(detail.validation(cx).is_ok());
                detail.editor.update(cx, |e, cx| {
                    e.reset(r#"{"type":"http","url":"https://example.test"}"#, cx)
                });
                detail.also.insert(McpClientId::Codex);
                assert!(detail.validation(cx).is_ok());
                detail.also.insert(McpClientId::Antigravity);
                assert!(detail
                    .validation(cx)
                    .is_err_and(|e| e.contains("Turn off Also update for Antigravity CLI")));
            });
        })
        .unwrap();
    }

    #[test]
    fn per_agent_difference_shows_a_neutral_note_in_the_detail() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|_, cx| Host(cx.new(|cx| McpPane::new(store, cx))));
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(gpui::size(px(900.), px(700.)));
        for (name, differs) in [("github", true), ("aaa", false)] {
            host.update(&mut visual, |host, window, cx| {
                let detail = host.0.read(cx).detail.clone();
                detail.update(cx, |detail, cx| {
                    detail.catalog = Some(catalog());
                    detail.runtimes = runtimes();
                    detail.name = None;
                    detail.open(McpClientId::Codex, name, window, cx);
                });
            })
            .unwrap();
            visual.run_until_parked();
            assert_eq!(
                visual.debug_bounds("MCP_PER_AGENT_NOTE").is_some(),
                differs,
                "{name}"
            );
        }
    }

    #[test]
    fn cancel_edit_confirms_before_discard_and_runner_can_be_edited() {
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
                assert!(detail.editing);
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
