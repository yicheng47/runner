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
        ToolRouter::new()
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
                "Runner MCP server. CRUD access to crews, runners, and slots — \
                 the building blocks of a Runner workspace.",
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
