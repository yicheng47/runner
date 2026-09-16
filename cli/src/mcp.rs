use std::time::Duration;

use crate::ipc::IpcStream;
use runner_core::app_paths::IpcEndpoint;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ErrorData, Implementation, InitializeRequestParams,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities,
    ServerInfo,
};
use rmcp::service::{serve_directly, RequestContext, RoleServer};
use rmcp::ServiceExt;
use tokio::time::timeout;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

fn endpoint() -> Option<IpcEndpoint> {
    let debug = cfg!(debug_assertions);
    let app_data_dir = runner_core::app_paths::app_data_dir(debug)?;
    Some(runner_core::app_paths::mcp_endpoint(&app_data_dir, debug))
}

pub fn run() -> i32 {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runner-mcp: failed to start runtime: {e}");
            return 1;
        }
    };

    rt.block_on(async {
        let stdout = tokio::io::BufWriter::new(tokio::io::stdout());
        // GitHub Copilot CLI sends a custom `server/discover` request before
        // `initialize`, and rmcp's handshake exits on any first message that is
        // not `initialize`. Serving directly routes every message through the
        // handler instead: unknown requests get method-not-found and
        // `initialize` is answered by `RunnerMcpProxy::initialize` (#621).
        let server = serve_directly::<RoleServer, _, _, _, _>(
            RunnerMcpProxy,
            (tokio::io::stdin(), stdout),
            None,
        );
        match server.waiting().await {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("runner-mcp: stdio session ended with error: {e}");
                1
            }
        }
    })
}

#[derive(Clone)]
struct RunnerMcpProxy;

impl ServerHandler for RunnerMcpProxy {
    fn get_info(&self) -> ServerInfo {
        let implementation = Implementation::new("runner", env!("CARGO_PKG_VERSION"));
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        ServerInfo::new(capabilities)
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_server_info(implementation)
            .with_instructions(
                "Runner MCP proxy. Open Runner.app to execute workspace and mission tools.",
            )
    }

    /// The negotiation rmcp's own handshake performs: a client on an older
    /// version gets that version back, anything else gets ours.
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        let mut info = self.get_info();
        info.protocol_version = match request.protocol_version.partial_cmp(&info.protocol_version) {
            Some(std::cmp::Ordering::Less) => request.protocol_version.clone(),
            _ => info.protocol_version,
        };
        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }
        Ok(info)
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        proxy_list_tools(request).await
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        proxy_call_tool(request).await
    }
}

async fn proxy_list_tools(
    request: Option<PaginatedRequestParams>,
) -> Result<ListToolsResult, ErrorData> {
    let stream = connect_app().await?;
    let (read, write) = stream.into_split();
    let write = tokio::io::BufWriter::new(write);
    let client = ().serve((read, write)).await.map_err(proxy_init_error)?;
    client
        .peer()
        .list_tools(request)
        .await
        .map_err(proxy_service_error)
}

async fn proxy_call_tool(request: CallToolRequestParams) -> Result<CallToolResult, ErrorData> {
    let stream = connect_app().await?;
    let (read, write) = stream.into_split();
    let write = tokio::io::BufWriter::new(write);
    let client = ().serve((read, write)).await.map_err(proxy_init_error)?;
    client
        .peer()
        .call_tool(request)
        .await
        .map_err(proxy_service_error)
}

async fn connect_app() -> Result<IpcStream, ErrorData> {
    let path = endpoint().ok_or_else(|| {
        ErrorData::internal_error(
            "Runner app data directory could not be resolved from the home directory.",
            None,
        )
    })?;

    match timeout(CONNECT_TIMEOUT, IpcStream::connect(&path)).await {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(e)) => Err(ErrorData::internal_error(
            format!(
                "Runner.app is not running. Open Runner and retry. Could not connect to {}: {e}",
                path
            ),
            None,
        )),
        Err(_) => Err(ErrorData::internal_error(
            format!(
                "Runner.app did not accept the MCP connection within {}ms. Open Runner and retry.",
                CONNECT_TIMEOUT.as_millis()
            ),
            None,
        )),
    }
}

fn proxy_init_error(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(
        format!("Runner.app MCP server did not initialize: {e}"),
        None,
    )
}

fn proxy_service_error(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(format!("Runner.app MCP call failed: {e}"), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn drive(lines: &[&str]) -> Vec<serde_json::Value> {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server);
        let running = serve_directly::<RoleServer, _, _, _, _>(
            RunnerMcpProxy,
            (server_read, server_write),
            None,
        );
        let (client_read, mut client_write) = tokio::io::split(client);
        let mut client_read = BufReader::new(client_read).lines();
        let mut replies = Vec::new();
        for line in lines {
            client_write.write_all(line.as_bytes()).await.unwrap();
            client_write.write_all(b"\n").await.unwrap();
            client_write.flush().await.unwrap();
            if line.contains("\"id\"") {
                let reply = client_read.next_line().await.unwrap().unwrap();
                replies.push(serde_json::from_str(&reply).unwrap());
            }
        }
        drop(client_write);
        drop(client_read);
        let _ = running.waiting().await;
        replies
    }

    #[tokio::test]
    async fn copilot_discovery_before_initialize_is_refused_without_ending_the_session() {
        let replies = drive(&[
            r#"{"jsonrpc":"2.0","id":0,"method":"server/discover","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"copilot","version":"1.0.83"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        ])
        .await;
        assert_eq!(replies.len(), 2, "{replies:?}");
        assert_eq!(replies[0]["id"], 0);
        assert_eq!(replies[0]["error"]["code"], -32601, "{:?}", replies[0]);
        assert_eq!(replies[1]["id"], 1);
        let result = &replies[1]["result"];
        assert_eq!(result["protocolVersion"], "2025-06-18");
        assert_eq!(result["serverInfo"]["name"], "runner");
        assert!(result["capabilities"]["tools"].is_object(), "{result:?}");
    }

    #[tokio::test]
    async fn a_client_on_a_newer_version_gets_the_bridge_version() {
        for version in ["2025-11-25", "2026-07-28"] {
            let replies = drive(&[&format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{version}","capabilities":{{}},"clientInfo":{{"name":"probe","version":"0"}}}}}}"#
            )])
            .await;
            assert_eq!(
                replies[0]["result"]["protocolVersion"],
                ProtocolVersion::LATEST.as_str(),
                "{version}: {:?}",
                replies[0]
            );
        }
    }
}
