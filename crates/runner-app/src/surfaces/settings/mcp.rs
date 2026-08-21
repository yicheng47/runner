use gpui::prelude::*;
use gpui::{div, rems, svg, AnyElement, Context, Entity, FontWeight, Render, SharedString, Window};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, CopyValueButton, PaneHeader, Toggle, Tooltip,
};
use runner_backend::ops::mcp::{McpClientStatus, McpIntegrationStatus};

use crate::app_store::AppStore;
use crate::theme;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpClient {
    ClaudeCode,
    Codex,
    Qoder,
    Trae,
}

impl McpClient {
    const ALL: [Self; 4] = [Self::ClaudeCode, Self::Codex, Self::Qoder, Self::Trae];

    fn key(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::Qoder => "qoder",
            Self::Trae => "trae",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex CLI",
            Self::Qoder => "Qoder CLI",
            Self::Trae => "TRAE CLI",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Writes the runner entry under ~/.claude.json.",
            Self::Codex => "Writes the runner table under ~/.codex/config.toml.",
            Self::Qoder => "Writes the runner entry under ~/.qoder/settings.json.",
            Self::Trae => "Writes the runner table under ~/.trae/traecli.toml.",
        }
    }

    fn copy_label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude",
            Self::Codex => "Codex",
            Self::Qoder => "Qoder",
            Self::Trae => "TRAE",
        }
    }

    fn status(self, status: &McpIntegrationStatus) -> &McpClientStatus {
        match self {
            Self::ClaudeCode => &status.claude_code,
            Self::Codex => &status.codex,
            Self::Qoder => &status.qoder,
            Self::Trae => &status.trae,
        }
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
    label: &'static str,
    tone: McpRowTone,
    active: bool,
    disabled: bool,
    detail_label: Option<&'static str>,
    detail: Option<String>,
}

struct McpSnippets {
    claude_code: String,
    codex: String,
    qoder: String,
    trae: String,
}

impl McpSnippets {
    fn value(&self, client: McpClient) -> &str {
        match client {
            McpClient::ClaudeCode => &self.claude_code,
            McpClient::Codex => &self.codex,
            McpClient::Qoder => &self.qoder,
            McpClient::Trae => &self.trae,
        }
    }
}

pub(crate) struct McpPane {
    app_store: Entity<AppStore>,
    status: Option<McpIntegrationStatus>,
    error: Option<String>,
    loading: bool,
    busy: Option<McpClient>,
    binding_copy: Entity<CopyValueButton>,
    snippet_copies: Vec<(McpClient, Entity<CopyValueButton>)>,
}

impl McpPane {
    pub(crate) fn new(app_store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let binding_copy = cx.new(|copy_cx| {
            CopyValueButton::new(copy_cx.focus_handle(), None, "Copy Binding dir").show_when_empty()
        });
        let snippet_copies = McpClient::ALL
            .into_iter()
            .map(|client| {
                let copy = cx.new(|copy_cx| {
                    CopyValueButton::new(copy_cx.focus_handle(), None, client.copy_label())
                        .labeled()
                        .show_when_empty()
                });
                (client, copy)
            })
            .collect();
        Self {
            app_store,
            status: None,
            error: None,
            loading: false,
            busy: None,
            binding_copy,
            snippet_copies,
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.busy.is_some() {
            return;
        }
        self.loading = true;
        let core = self.app_store.read(cx).core.clone();
        let task = cx.background_spawn(async move {
            let status = runner_backend::ops::mcp::mcp_integration_status(&core)
                .map_err(|error| error.to_string())?;
            let snippets = runner_backend::ops::mcp::mcp_config_snippet(&core)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((
                status,
                McpSnippets {
                    claude_code: snippets.claude_code,
                    codex: snippets.codex,
                    qoder: snippets.qoder,
                    trae: snippets.trae,
                },
            ))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((status, snippets)) => {
                        this.apply_status(status, snippets, cx);
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_status(
        &mut self,
        status: McpIntegrationStatus,
        snippets: McpSnippets,
        cx: &mut Context<Self>,
    ) {
        let binding_dir = parent_path(&status.socket_path);
        self.binding_copy.update(cx, |copy, copy_cx| {
            copy.set_value((!binding_dir.is_empty()).then_some(binding_dir), copy_cx)
        });
        for (client, copy) in &self.snippet_copies {
            let value = snippets.value(*client).to_owned();
            copy.update(cx, |copy, copy_cx| {
                copy.set_value((!value.is_empty()).then_some(value), copy_cx)
            });
        }
        self.status = Some(status);
    }

    fn set_integration(&mut self, client: McpClient, enabled: bool, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(client);
        self.error = None;
        let core = self.app_store.read(cx).core.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mcp::mcp_set_integration(&core, client.key(), enabled)
                .map_err(|error| error.to_string())?;
            let status = runner_backend::ops::mcp::mcp_integration_status(&core)
                .map_err(|error| error.to_string())?;
            let snippets = runner_backend::ops::mcp::mcp_config_snippet(&core)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((
                status,
                McpSnippets {
                    claude_code: snippets.claude_code,
                    codex: snippets.codex,
                    qoder: snippets.qoder,
                    trae: snippets.trae,
                },
            ))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok((status, snippets)) => {
                        this.apply_status(status, snippets, cx);
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_binding(&self, _cx: &mut Context<Self>) -> AnyElement {
        let environment = self
            .status
            .as_ref()
            .map(|status| status.environment.as_str())
            .unwrap_or("Loading");
        let development = environment.to_lowercase().contains("dev");
        let loading = self.status.is_none();
        let badge_color = if loading {
            theme::faint()
        } else if development {
            theme::warning()
        } else {
            theme::accent()
        };
        let binding_dir = self
            .status
            .as_ref()
            .map(|status| parent_path(&status.socket_path))
            .unwrap_or_default();
        let binding_field = div()
            .h_8()
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
            .gap_2()
            .rounded(rems(4. / 16.))
            .bg(theme::raised())
            .px_2()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .font_family("JetBrains Mono")
                    .text_size(rems(11. / 16.))
                    .text_color(theme::muted())
                    .child(if binding_dir.is_empty() {
                        "Loading...".into()
                    } else {
                        binding_dir.clone()
                    }),
            )
            .child(self.binding_copy.clone());
        let binding_field = if binding_dir.is_empty() {
            binding_field.into_any_element()
        } else {
            Tooltip::new("mcp-binding-dir-tooltip", binding_dir, binding_field)
                .expand()
                .into_any_element()
        };
        div()
            .rounded(rems(12. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .p_4()
            .child(
                div()
                    .mb_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                svg()
                                    .path("plug.svg")
                                    .size(rems(14. / 16.))
                                    .text_color(theme::muted()),
                            )
                            .child(
                                div()
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Current binding"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(6. / 16.))
                            .rounded(rems(4. / 16.))
                            .bg(theme::with_alpha(badge_color, 0.1))
                            .px_2()
                            .py(rems(2. / 16.))
                            .text_size(rems(10. / 16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(badge_color)
                            .child(div().size(rems(6. / 16.)).rounded_full().bg(badge_color))
                            .child(environment.to_owned()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .child(
                        div()
                            .w(rems(72. / 16.))
                            .text_size(rems(11. / 16.))
                            .text_color(theme::faint())
                            .child("Binding dir"),
                    )
                    .child(binding_field),
            )
            .into_any_element()
    }

    fn render_client(&self, client: McpClient, cx: &mut Context<Self>) -> AnyElement {
        let status = self.status.as_ref().map(|status| client.status(status));
        let presentation = mcp_row_presentation(status, self.busy == Some(client));
        let color = match presentation.tone {
            McpRowTone::Muted => theme::faint(),
            McpRowTone::Accent => theme::accent(),
            McpRowTone::Warning => theme::warning(),
            McpRowTone::Danger => theme::danger(),
        };
        let pane = cx.entity();
        let detail = presentation.detail.clone();
        div()
            .rounded(rems(12. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .p_4()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(rems(13. / 16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(client.title()),
                                    )
                                    .children((!presentation.label.is_empty()).then(|| {
                                        div()
                                            .text_size(rems(11. / 16.))
                                            .text_color(color)
                                            .child(presentation.label)
                                    })),
                            )
                            .child(
                                div()
                                    .mt(rems(2. / 16.))
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::muted())
                                    .child(client.subtitle()),
                            ),
                    )
                    .child(
                        Toggle::new(
                            SharedString::from(format!("mcp-toggle-{}", client.key())),
                            presentation.active,
                        )
                        .disabled(presentation.disabled)
                        .on_change(move |enabled, _, cx| {
                            pane.update(cx, |this, pane_cx| {
                                this.set_integration(client, enabled, pane_cx)
                            });
                        }),
                    ),
            )
            .children(detail.map(|detail| {
                div()
                    .mt_2()
                    .min_w_0()
                    .rounded(rems(4. / 16.))
                    .bg(theme::raised())
                    .px_2()
                    .py(rems(6. / 16.))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .w(rems(112. / 16.))
                                    .text_size(rems(10. / 16.))
                                    .text_color(theme::faint())
                                    .child(presentation.detail_label.unwrap_or("")),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .font_family("JetBrains Mono")
                                    .text_size(rems(10. / 16.))
                                    .line_height(rems(14.5 / 16.))
                                    .text_color(theme::faint())
                                    .child(detail),
                            ),
                    )
            }))
            .into_any_element()
    }
}

impl Render for McpPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let retry = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new("MCP", "Register Runner with external MCP clients."))
            .child(self.render_binding(cx))
            .children(McpClient::ALL.into_iter().map(|client| self.render_client(client, cx)))
            .child(
                div()
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .p_4()
                    .child(
                        div()
                            .mb_2()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(div().text_size(rems(13. / 16.)).font_weight(FontWeight::SEMIBOLD).child("Manual config"))
                            .child(div().flex().items_center().gap_2().children(self.snippet_copies.iter().map(|(_, copy)| copy.clone()))),
                    )
                    .child(div().text_size(rems(11. / 16.)).line_height(rems(16.5 / 16.)).text_color(theme::muted()).child("Use these snippets for clients Runner does not update directly.")),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(rems(10. / 16.))
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .px_4()
                    .py_3()
                    .child(
                        svg()
                            .path("shield-alert.svg")
                            .size(rems(14. / 16.))
                            .flex_none()
                            .text_color(theme::accent()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .whitespace_normal()
                            .text_size(rems(11. / 16.))
                            .line_height(rems(16.5 / 16.))
                            .text_color(theme::muted())
                            .child("Registering replaces only the `runner` MCP entry. If the row says it points to another binary, replacing it will move that client to the binding shown above."),
                    ),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_3()
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.3))
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .px_4()
                    .py_3()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::danger())
                    .child(div().min_w_0().child(error))
                    .child(Button::new("mcp-retry", "Retry").size(ButtonSize::Sm).variant(ButtonVariant::Ghost).on_press(move |_, cx| {
                        retry.update(cx, |this, pane_cx| this.refresh(pane_cx));
                    }))
            }))
    }
}

fn parent_path(path: &str) -> String {
    path.rfind('/')
        .filter(|index| *index > 0)
        .map_or_else(|| path.to_owned(), |index| path[..index].to_owned())
}

fn mcp_row_presentation(status: Option<&McpClientStatus>, busy: bool) -> McpRowPresentation {
    let Some(status) = status else {
        return McpRowPresentation {
            label: "Checking",
            tone: McpRowTone::Muted,
            active: false,
            disabled: true,
            detail_label: None,
            detail: None,
        };
    };
    let configured = status.registered.then(|| {
        format!(
            "{} {}",
            status.command.as_deref().unwrap_or("(missing command)"),
            serde_json::to_string(&status.args).unwrap_or_else(|_| "[]".into())
        )
    });
    if busy {
        let (detail_label, detail) = if let Some(error) = status.error.clone() {
            (Some("Error"), Some(error))
        } else if status.registered && !status.matches_current {
            (Some("Configured command"), configured)
        } else {
            (None, None)
        };
        return McpRowPresentation {
            label: "Updating",
            tone: if status.error.is_some() {
                McpRowTone::Danger
            } else if status.matches_current {
                McpRowTone::Accent
            } else if status.registered {
                McpRowTone::Warning
            } else {
                McpRowTone::Muted
            },
            active: status.matches_current,
            disabled: true,
            detail_label,
            detail,
        };
    }
    if let Some(error) = status.error.clone() {
        return McpRowPresentation {
            label: "Config error",
            tone: McpRowTone::Danger,
            active: status.matches_current,
            disabled: true,
            detail_label: Some("Error"),
            detail: Some(error),
        };
    }
    if !status.registered {
        return McpRowPresentation {
            label: "Not registered",
            tone: McpRowTone::Muted,
            active: false,
            disabled: false,
            detail_label: None,
            detail: None,
        };
    }
    if status.matches_current {
        return McpRowPresentation {
            label: "",
            tone: McpRowTone::Accent,
            active: true,
            disabled: false,
            detail_label: None,
            detail: None,
        };
    }
    McpRowPresentation {
        label: "Registered to another Runner",
        tone: McpRowTone::Warning,
        active: false,
        disabled: false,
        detail_label: Some("Configured command"),
        detail: configured,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(registered: bool, matches_current: bool, error: Option<&str>) -> McpClientStatus {
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
        assert_eq!(mcp_row_presentation(None, false).label, "Checking");
        assert_eq!(
            mcp_row_presentation(Some(&status(false, false, None)), false).label,
            "Not registered"
        );
        assert_eq!(
            mcp_row_presentation(Some(&status(true, true, None)), false).label,
            ""
        );
        let other = mcp_row_presentation(Some(&status(true, false, None)), false);
        assert_eq!(other.label, "Registered to another Runner");
        assert_eq!(other.detail_label, Some("Configured command"));
        assert_eq!(
            mcp_row_presentation(Some(&status(false, false, Some("bad config"))), false).label,
            "Config error"
        );
        assert_eq!(
            mcp_row_presentation(Some(&status(true, true, None)), true).label,
            "Updating"
        );
        assert_eq!(
            mcp_row_presentation(Some(&status(true, true, None)), true).tone,
            McpRowTone::Accent
        );
        let updating_error =
            mcp_row_presentation(Some(&status(false, false, Some("bad config"))), true);
        assert_eq!(updating_error.tone, McpRowTone::Danger);
        assert_eq!(updating_error.detail_label, Some("Error"));
    }

    #[test]
    fn binding_dir_is_socket_parent() {
        assert_eq!(parent_path("/test/mcp.sock"), "/test");
    }
}
