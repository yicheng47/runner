use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, JsonObject, ListToolsResult, PaginatedRequestParams,
};
use rmcp::service::{RoleClient, RunningService, ServiceError};
use rmcp::ServiceExt;
use runner_core::app_paths::IpcEndpoint;
use serde_json::Value;
use tokio::time::timeout;

use crate::ipc::IpcStream;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
pub const NOT_RUNNING_MESSAGE: &str = "Runner is not running. Open Runner and retry.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    NotRunning,
    Refused(String),
    Protocol(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning => f.write_str(NOT_RUNNING_MESSAGE),
            Self::Refused(message) | Self::Protocol(message) => f.write_str(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResponse {
    pub value: Value,
    pub raw_json: String,
}

pub fn endpoint() -> Option<IpcEndpoint> {
    let debug = cfg!(debug_assertions);
    let app_data_dir = runner_core::app_paths::app_data_dir(debug)?;
    Some(runner_core::app_paths::mcp_endpoint(&app_data_dir, debug))
}

pub struct SocketClient {
    service: RunningService<RoleClient, ()>,
    endpoint: IpcEndpoint,
    app_version: String,
}

impl SocketClient {
    pub async fn connect() -> Result<Self, ClientError> {
        let endpoint = endpoint().ok_or_else(|| {
            ClientError::Protocol(
                "Runner app data directory could not be resolved from the home directory."
                    .to_owned(),
            )
        })?;
        let stream = match timeout(CONNECT_TIMEOUT, IpcStream::connect(&endpoint)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(_)) | Err(_) => return Err(ClientError::NotRunning),
        };
        let (read, write) = stream.into_split();
        let write = tokio::io::BufWriter::new(write);
        let service = match timeout(HANDSHAKE_TIMEOUT, ().serve((read, write))).await {
            Ok(Ok(service)) => service,
            Ok(Err(_)) | Err(_) => return Err(ClientError::NotRunning),
        };
        let app_version = service
            .peer_info()
            .map(|info| info.server_info.version.clone())
            .unwrap_or_else(|| "unknown".to_owned());
        Ok(Self {
            service,
            endpoint,
            app_version,
        })
    }

    pub fn endpoint(&self) -> &IpcEndpoint {
        &self.endpoint
    }

    pub fn app_version(&self) -> &str {
        &self.app_version
    }

    pub async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
    ) -> Result<ListToolsResult, ClientError> {
        self.service
            .peer()
            .list_tools(request)
            .await
            .map_err(service_error)
    }

    pub async fn call_result(
        &self,
        request: CallToolRequestParams,
    ) -> Result<CallToolResult, ClientError> {
        self.service
            .peer()
            .call_tool(request)
            .await
            .map_err(service_error)
    }

    pub async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, ClientError> {
        let arguments = arguments.as_object().cloned().ok_or_else(|| {
            ClientError::Protocol("tool arguments must be a JSON object".to_owned())
        })?;
        let request = CallToolRequestParams::new(name.to_owned()).with_arguments(arguments);
        decode_result(self.call_result(request).await?)
    }
}

fn service_error(error: ServiceError) -> ClientError {
    match error {
        ServiceError::McpError(error) => ClientError::Refused(error.message.into_owned()),
        ServiceError::TransportClosed
        | ServiceError::TransportSend(_)
        | ServiceError::Timeout { .. } => ClientError::NotRunning,
        other => ClientError::Protocol(other.to_string()),
    }
}

fn decode_result(result: CallToolResult) -> Result<ToolResponse, ClientError> {
    let text = result
        .content
        .iter()
        .find_map(|content| content.as_text())
        .map(|content| content.text.clone());
    if result.is_error.unwrap_or(false) {
        return Err(ClientError::Refused(
            text.unwrap_or_else(|| "Runner refused the tool call.".to_owned()),
        ));
    }
    if let Some(raw_json) = text {
        let value = serde_json::from_str(&raw_json)
            .map_err(|error| ClientError::Protocol(format!("invalid tool JSON result: {error}")))?;
        return Ok(ToolResponse { value, raw_json });
    }
    if let Some(value) = result.structured_content {
        let raw_json = serde_json::to_string(&value)
            .map_err(|error| ClientError::Protocol(error.to_string()))?;
        return Ok(ToolResponse { value, raw_json });
    }
    Ok(ToolResponse {
        value: Value::Null,
        raw_json: "null".to_owned(),
    })
}

pub fn arguments(value: Value) -> Result<JsonObject, ClientError> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| ClientError::Protocol("tool arguments must be a JSON object".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::ServerHandler;
    use rmcp::model::{CallToolResult, Content, ServerInfo};
    use rmcp::{tool, tool_handler, tool_router, ErrorData};

    #[derive(Clone)]
    struct Stub;

    #[tool_router]
    impl Stub {
        #[tool(description = "Echo arguments.")]
        async fn echo(
            &self,
            rmcp::handler::server::wrapper::Parameters(input): rmcp::handler::server::wrapper::Parameters<
                std::collections::HashMap<String, String>,
            >,
        ) -> Result<CallToolResult, ErrorData> {
            Ok(CallToolResult::success(vec![Content::json(input)?]))
        }
    }

    #[tool_handler]
    impl ServerHandler for Stub {
        fn get_info(&self) -> ServerInfo {
            ServerInfo::default()
        }
    }

    #[tokio::test]
    async fn shared_call_path_works_over_a_duplex_rmcp_server() {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(async move { Stub.serve(server_stream).await.unwrap() });
        let client = ().serve(client_stream).await.unwrap();
        let request = CallToolRequestParams::new("echo")
            .with_arguments(arguments(serde_json::json!({"value": "wire"})).unwrap());
        let response = decode_result(client.peer().call_tool(request).await.unwrap()).unwrap();
        assert_eq!(response.value, serde_json::json!({"value": "wire"}));
        drop(client);
        drop(server.await.unwrap());
    }

    #[test]
    fn json_is_preserved_and_tool_errors_are_refusals() {
        let response = decode_result(CallToolResult::success(vec![Content::text(
            r#"{"id":"01","name":"Runner"}"#,
        )]))
        .unwrap();
        assert_eq!(response.raw_json, r#"{"id":"01","name":"Runner"}"#);
        assert_eq!(response.value["id"], "01");

        let error =
            decode_result(CallToolResult::error(vec![Content::text("refused")])).unwrap_err();
        assert_eq!(error, ClientError::Refused("refused".into()));
        assert_eq!(
            service_error(ServiceError::McpError(ErrorData::invalid_request(
                "bad request",
                None,
            ))),
            ClientError::Refused("bad request".into())
        );
    }
}
