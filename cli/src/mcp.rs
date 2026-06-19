use std::{borrow::Cow, path::PathBuf, sync::Arc, time::Duration};

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ErrorData, Implementation, JsonObject, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
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
                "Runner MCP proxy. Open Runner.app to execute crew, runner, and slot tools.",
            )
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        match proxy_list_tools(request).await {
            Ok(result) => Ok(result),
            Err(_) => Ok(ListToolsResult::with_all_items(fallback_tools())),
        }
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

fn fallback_tools() -> Vec<Tool> {
    [
        ("crew_list", "List all crews, including runner counts and member previews.", empty_schema()),
        ("crew_get", "Get a crew by ID.", id_schema("id", "Crew ID.")),
        ("crew_create", "Create a new crew.", open_schema()),
        (
            "crew_update",
            "Update a crew by ID. Omitted fields are preserved.",
            id_input_schema("id", "Crew ID."),
        ),
        (
            "crew_delete",
            "Delete a crew by ID. Slot rows are removed; runner templates are kept.",
            id_schema("id", "Crew ID."),
        ),
        ("runner_list", "List all runner templates.", empty_schema()),
        ("runner_get", "Get a runner template by ID.", id_schema("id", "Runner ID.")),
        (
            "runner_get_by_handle",
            "Get a runner template by handle.",
            id_schema("handle", "Runner handle without the leading @."),
        ),
        ("runner_create", "Create a new runner template.", open_schema()),
        (
            "runner_update",
            "Update a runner template by ID. Omitted fields are preserved.",
            id_input_schema("id", "Runner ID."),
        ),
        (
            "runner_delete",
            "Delete a runner template by ID. Live sessions for that runner are killed first.",
            id_schema("id", "Runner ID."),
        ),
        (
            "slot_list",
            "List the slots for a crew, ordered by position.",
            id_schema("crew_id", "Crew ID."),
        ),
        ("slot_create", "Create a slot in a crew.", open_schema()),
        (
            "slot_update",
            "Update a slot by ID. Omitted fields are preserved.",
            id_input_schema("slot_id", "Slot ID."),
        ),
        ("slot_delete", "Delete a slot by ID.", id_schema("slot_id", "Slot ID.")),
        (
            "slot_set_lead",
            "Make a slot the lead slot for its crew.",
            id_schema("slot_id", "Slot ID."),
        ),
        (
            "slot_reorder",
            "Reorder all slots in a crew.",
            object_schema(
                &[
                    ("crew_id", string_schema("Crew ID.")),
                    (
                        "ordered_slot_ids",
                        serde_json::json!({
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Slot IDs in the desired final order. Must include every slot exactly once."
                        }),
                    ),
                ],
                &["crew_id", "ordered_slot_ids"],
                false,
            ),
        ),
    ]
    .into_iter()
    .map(|(name, description, input_schema)| {
        Tool::new(Cow::Borrowed(name), Cow::Borrowed(description), input_schema)
    })
    .collect()
}

fn empty_schema() -> Arc<JsonObject> {
    object_schema(&[], &[], false)
}

fn open_schema() -> Arc<JsonObject> {
    object_schema(&[], &[], true)
}

fn id_schema(name: &'static str, description: &'static str) -> Arc<JsonObject> {
    object_schema(&[(name, string_schema(description))], &[name], false)
}

fn id_input_schema(id_name: &'static str, id_description: &'static str) -> Arc<JsonObject> {
    object_schema(
        &[
            (id_name, string_schema(id_description)),
            (
                "input",
                serde_json::json!({
                    "type": "object",
                    "description": "Fields to update. Omitted fields are preserved.",
                    "additionalProperties": true
                }),
            ),
        ],
        &[id_name, "input"],
        false,
    )
}

fn string_schema(description: &'static str) -> serde_json::Value {
    serde_json::json!({ "type": "string", "description": description })
}

fn object_schema(
    properties: &[(&'static str, serde_json::Value)],
    required: &[&'static str],
    additional_properties: bool,
) -> Arc<JsonObject> {
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), serde_json::json!("object"));
    schema.insert(
        "properties".to_string(),
        serde_json::Value::Object(
            properties
                .iter()
                .map(|(name, schema)| ((*name).to_string(), schema.clone()))
                .collect(),
        ),
    );
    schema.insert(
        "additionalProperties".to_string(),
        serde_json::Value::Bool(additional_properties),
    );
    if !required.is_empty() {
        schema.insert("required".to_string(), serde_json::json!(required));
    }
    Arc::new(schema)
}
