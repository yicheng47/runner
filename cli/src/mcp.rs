//! MCP stdio↔socket bridge for external MCP clients.
//!
//! Unix-only: it connects to the app's Unix-domain `mcp.sock`. On the
//! Windows host build it compiles to a stub that errors out — the
//! WSL-hosted agents reach the app's MCP server from inside Linux, so a
//! Windows-host bridge isn't part of the M1 surface (see plan M4+).

#[cfg(unix)]
pub use unix_impl::run;

#[cfg(not(unix))]
pub fn run() -> i32 {
    eprintln!(
        "runner-mcp: the stdio↔socket MCP bridge needs a Unix domain socket \
         and isn't supported on the Windows host build. Run it inside WSL."
    );
    1
}

#[cfg(unix)]
mod unix_impl {
use std::{path::PathBuf, time::Duration};

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ErrorData, Implementation, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServiceExt;
use tokio::net::UnixStream;
use tokio::time::timeout;

const APP_IDENTIFIER: &str = "com.wycstudios.runner";
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

fn app_data_segment() -> String {
    if cfg!(debug_assertions) {
        format!("{APP_IDENTIFIER}-dev")
    } else {
        APP_IDENTIFIER.to_string()
    }
}

fn socket_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    #[cfg(target_os = "macos")]
    let base = home.join("Library/Application Support");
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    Some(base.join(app_data_segment()).join("mcp.sock"))
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
        let handler = RunnerMcpProxy;
        let stdout = tokio::io::BufWriter::new(tokio::io::stdout());
        match handler.serve((tokio::io::stdin(), stdout)).await {
            Ok(server) => match server.waiting().await {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("runner-mcp: stdio session ended with error: {e}");
                    1
                }
            },
            Err(e) => {
                eprintln!("runner-mcp: failed to initialize stdio MCP server: {e}");
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

async fn connect_app() -> Result<UnixStream, ErrorData> {
    let path = socket_path().ok_or_else(|| {
        ErrorData::internal_error(
            "Runner app data directory could not be resolved from HOME.",
            None,
        )
    })?;

    match timeout(CONNECT_TIMEOUT, UnixStream::connect(&path)).await {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(e)) => Err(ErrorData::internal_error(
            format!(
                "Runner.app is not running. Open Runner and retry. Could not connect to {}: {e}",
                path.display()
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
} // mod unix_impl
