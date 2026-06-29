use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{Implementation, ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::tool_handler;
use rmcp::ServiceExt;

use super::state::McpState;

#[derive(Clone)]
pub(crate) struct RunnerMcpHandler {
    #[allow(dead_code)] // read by Phase 2 tool methods
    pub(crate) state: McpState,
}

impl RunnerMcpHandler {
    pub(crate) fn new(state: McpState) -> Self {
        Self { state }
    }

    pub(crate) fn tool_router() -> ToolRouter<Self> {
        let mut r = ToolRouter::new();
        r.merge(Self::crew_router());
        r.merge(Self::runner_router());
        r.merge(Self::slot_router());
        r.merge(Self::mission_router());
        r
    }
}

#[tool_handler]
impl ServerHandler for RunnerMcpHandler {
    fn get_info(&self) -> ServerInfo {
        let implementation = Implementation::new("runner", env!("CARGO_PKG_VERSION"));
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        ServerInfo::new(capabilities)
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_server_info(implementation)
            .with_instructions(
                "Runner MCP server. Access crews, runners, slots, and mission \
                 lifecycle/status tools for operating a Runner workspace.",
            )
    }
}

pub(crate) async fn serve_connection(stream: tokio::net::UnixStream, state: McpState) {
    let (read, write) = stream.into_split();
    let handler = RunnerMcpHandler::new(state);
    // OwnedWriteHalf needs to be wrapped in BufWriter for the
    // async-rw transport codec (it expects AsyncBufRead + AsyncWrite).
    let write = tokio::io::BufWriter::new(write);
    match handler.serve((read, write)).await {
        Ok(server) => {
            let _ = server.waiting().await;
        }
        Err(e) => {
            log::warn!("mcp: session handshake failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_router_registers_workspace_and_mission_tools() {
        let router = RunnerMcpHandler::tool_router();
        let names: std::collections::BTreeSet<_> = router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        let expected: std::collections::BTreeSet<_> = [
            "crew_list",
            "crew_get",
            "crew_create",
            "crew_update",
            "crew_delete",
            "runner_list",
            "runner_get",
            "runner_get_by_handle",
            "runner_create",
            "runner_update",
            "runner_delete",
            "slot_list",
            "slot_create",
            "slot_update",
            "slot_delete",
            "slot_set_lead",
            "slot_reorder",
            "mission_list",
            "mission_get",
            "mission_list_summary",
            "mission_feed",
            "mission_status",
            "mission_start",
            "mission_stop",
            "mission_archive",
            "mission_pin",
            "mission_rename",
            "mission_reset",
            "mission_post_human_signal",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(names, expected, "MCP tool registry diverged from Phase 2");
    }

    // Regression for #240. A bare `serde_json::Value` field derives a
    // *typeless* schema (`{}`, plus any doc-comment `description`). Strict MCP
    // clients reject a property schema that declares neither `type` nor
    // `$ref`, and because tool discovery validates the whole `tools` array at
    // once, that single bad schema drops every Runner tool. Assert every tool
    // input property declares a `type` or `$ref` so a future free-form field
    // can't silently take the registry down again.
    #[test]
    fn every_tool_input_property_declares_a_type() {
        let router = RunnerMcpHandler::tool_router();
        for tool in router.list_all() {
            let Some(props) = tool.input_schema.get("properties") else {
                continue;
            };
            let props = props.as_object().unwrap_or_else(|| {
                panic!("{}: inputSchema.properties is not an object", tool.name)
            });
            for (field, schema) in props {
                let obj = schema.as_object().unwrap_or_else(|| {
                    panic!(
                        "{}: inputSchema.properties.{field} is not an object",
                        tool.name
                    )
                });
                assert!(
                    obj.contains_key("type") || obj.contains_key("$ref"),
                    "{}: inputSchema.properties.{field} must declare `type` or `$ref` \
                     (strict MCP clients reject typeless schemas, dropping the whole \
                     tool list — #240); got keys {:?}",
                    tool.name,
                    obj.keys().collect::<Vec<_>>(),
                );
            }
        }
    }
}
