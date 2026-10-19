use runner_core::app_paths::IpcEndpoint;
use runner_core::protocol::socket::{ConnectError, SocketTransport};
use runner_core::protocol::wire;
use runner_core::protocol::{DaemonClient, EventSubscription};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::time::Duration;

#[cfg(not(target_os = "linux"))]
pub const NOT_RUNNING_MESSAGE: &str = "Runner is not running. Open Runner and retry.";
#[cfg(target_os = "linux")]
pub const NOT_RUNNING_MESSAGE: &str =
    "Runner is not running. Open Runner or run systemctl --user start runnerd and retry.";
pub const BLOCKED_MESSAGE: &str = "Runner cannot be reached from this process: connecting to its socket was denied, which usually means a command sandbox. Run the same command again outside the sandbox.";
pub const VERSION_SKEW_MESSAGE: &str = wire::PROTOCOL_MISMATCH;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    NotRunning,
    Blocked,
    Refused(String),
    Protocol(String),
}
impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning => f.write_str(NOT_RUNNING_MESSAGE),
            Self::Blocked => f.write_str(BLOCKED_MESSAGE),
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
    let data = runner_core::app_paths::app_data_dir(debug)?;
    Some(runner_core::app_paths::daemon_endpoint(&data, debug))
}
fn connect_failure(kind: std::io::ErrorKind, endpoint_exists: bool) -> ClientError {
    if kind == std::io::ErrorKind::PermissionDenied && endpoint_exists {
        ClientError::Blocked
    } else {
        ClientError::NotRunning
    }
}
#[cfg(unix)]
fn endpoint_exists(endpoint: &IpcEndpoint) -> bool {
    endpoint.0.exists()
}
#[cfg(windows)]
fn endpoint_exists(_: &IpcEndpoint) -> bool {
    true
}

pub fn mission_read_cursor(
    mission_id: &str,
    handle: &str,
) -> Option<runner_core::protocol::MissionReadCursor> {
    mission_read_cursor_at(&endpoint()?, mission_id, handle)
}

fn mission_read_cursor_at(
    endpoint: &IpcEndpoint,
    mission_id: &str,
    handle: &str,
) -> Option<runner_core::protocol::MissionReadCursor> {
    let transport = SocketTransport::connect_with_timeout(
        endpoint,
        wire::Hello {
            exe_sha256: String::new(),
            client: "cli".into(),
        },
        Duration::from_millis(200),
    )
    .ok()?;
    let response = call_daemon(
        &transport.client(),
        "mission_read_cursor",
        json!({"mission_id": mission_id, "handle": handle}),
    )
    .ok()?;
    serde_json::from_value(response.value).ok().flatten()
}

pub struct SocketClient {
    daemon: DaemonClient,
    transport: std::sync::Arc<SocketTransport>,
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
        match Self::connect_endpoint(endpoint.clone()).await {
            Err(ClientError::NotRunning) if std::env::var_os("SSH_CONNECTION").is_none() => {
                let paths = runner_core::daemon_process::NativePaths::resolve()
                    .map_err(|error| ClientError::Protocol(error.to_string()))?;
                let source = std::env::current_exe()
                    .and_then(|path| path.canonicalize())
                    .map_err(|error| ClientError::Protocol(error.to_string()))?;
                let launch = runner_core::daemon_process::Launch::new(paths, source, false);
                Self::connect_or_start(&launch, false).await
            }
            result => result,
        }
    }

    pub async fn connect_or_start(
        launch: &runner_core::daemon_process::Launch,
        ssh: bool,
    ) -> Result<Self, ClientError> {
        match Self::connect_endpoint(launch.daemon_endpoint.clone()).await {
            Err(ClientError::NotRunning) if !ssh => (),
            result => return result,
        }
        let data = launch.paths.app_data_dir.clone();
        let _starter =
            tokio::task::spawn_blocking(move || runner_core::daemon_process::startup_lock(&data))
                .await
                .map_err(|error| ClientError::Protocol(error.to_string()))?
                .map_err(|error| ClientError::Protocol(error.to_string()))?;
        match Self::connect_endpoint(launch.daemon_endpoint.clone()).await {
            Err(ClientError::NotRunning) => (),
            result => return result,
        }
        let mut child = launch.spawn().map_err(|_| ClientError::NotRunning)?;
        let result = async {
            let deadline =
                tokio::time::Instant::now() + runner_core::daemon_process::DAEMON_START_TIMEOUT;
            loop {
                launch
                    .check_startup(&mut child)
                    .map_err(|error| ClientError::Protocol(error.to_string()))?;
                match Self::connect_endpoint(launch.daemon_endpoint.clone()).await {
                    Err(ClientError::NotRunning) if tokio::time::Instant::now() < deadline => {
                        tokio::time::sleep(Duration::from_millis(20)).await
                    }
                    Err(ClientError::NotRunning) => {
                        return Err(ClientError::Protocol(launch.startup_timeout().to_string()));
                    }
                    result => return result,
                }
            }
        }
        .await;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        result
    }

    pub async fn connect_endpoint(endpoint: IpcEndpoint) -> Result<Self, ClientError> {
        let destination = endpoint.clone();
        let exists = endpoint_exists(&endpoint);
        let transport = tokio::task::spawn_blocking(move || {
            SocketTransport::connect_with_timeout(
                &destination,
                wire::Hello {
                    exe_sha256: String::new(),
                    client: "cli".into(),
                },
                Duration::from_secs(30),
            )
        })
        .await
        .map_err(|error| ClientError::Protocol(error.to_string()))?
        .map_err(|error| match error {
            ConnectError::NotRunning => ClientError::NotRunning,
            ConnectError::Blocked => connect_failure(std::io::ErrorKind::PermissionDenied, exists),
            ConnectError::Mismatch(_) | ConnectError::Protocol(_) => {
                ClientError::Protocol(VERSION_SKEW_MESSAGE.into())
            }
        })?;
        let daemon = transport.client();
        let probe = daemon.clone();
        let app_version = tokio::task::spawn_blocking(move || probe.app_version())
            .await
            .map_err(|error| ClientError::Protocol(error.to_string()))?
            .map_err(|_| ClientError::Protocol(VERSION_SKEW_MESSAGE.into()))?;
        Ok(Self {
            daemon,
            transport,
            endpoint,
            app_version,
        })
    }
    pub fn daemon(&self) -> DaemonClient {
        self.daemon.clone()
    }
    pub fn endpoint(&self) -> &IpcEndpoint {
        &self.endpoint
    }
    pub fn app_version(&self) -> &str {
        &self.app_version
    }
    pub fn subscribe(&self) -> Box<dyn EventSubscription> {
        self.daemon.subscribe()
    }
    pub async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, ClientError> {
        let daemon = self.daemon.clone();
        let name = name.to_owned();
        let result = tokio::task::spawn_blocking(move || call_daemon(&daemon, &name, arguments))
            .await
            .map_err(|error| ClientError::Protocol(error.to_string()))?;
        match result {
            Err(ClientError::Refused(message)) if message == wire::PROTOCOL_MISMATCH => {
                Err(ClientError::Protocol(VERSION_SKEW_MESSAGE.into()))
            }
            Err(_) if self.transport.is_closed() => Err(ClientError::NotRunning),
            result => result,
        }
    }
}
fn argument<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, ClientError> {
    match args.get(name) {
        Some(value) => parse(value.clone()),
        None => parse(Value::Null).map_err(|_| {
            ClientError::Refused(format!(
                "failed to deserialize parameters: missing field `{name}`"
            ))
        }),
    }
}
fn parse<T: DeserializeOwned>(value: Value) -> Result<T, ClientError> {
    serde_json::from_value(value)
        .map_err(|error| ClientError::Refused(format!("failed to deserialize parameters: {error}")))
}
fn response<T: serde::Serialize>(
    result: Result<T, runner_core::protocol::ClientError>,
) -> Result<ToolResponse, ClientError> {
    let value = result.map_err(|error| ClientError::Refused(error.message))?;
    let raw_json =
        serde_json::to_string(&value).map_err(|error| ClientError::Protocol(error.to_string()))?;
    let value = serde_json::from_str(&raw_json)
        .map_err(|error| ClientError::Protocol(error.to_string()))?;
    Ok(ToolResponse { value, raw_json })
}
pub fn call_daemon(
    client: &DaemonClient,
    name: &str,
    args: Value,
) -> Result<ToolResponse, ClientError> {
    if !args.is_object() {
        return Err(ClientError::Protocol(
            "tool arguments must be a JSON object".into(),
        ));
    }
    match name {
        "crew_list" => response(client.crew_list_all()),
        "crew_get" => response(client.crew_get(&argument::<String>(&args, "id")?)),
        "crew_create" => response(client.crew_create(parse(args)?)),
        "crew_update" => response(
            client.crew_update(&argument::<String>(&args, "id")?, argument(&args, "input")?),
        ),
        "crew_delete" => {
            let id: String = argument(&args, "id")?;
            response(
                client
                    .crew_delete(&id)
                    .map(|()| json!({"deleted": true, "id": id})),
            )
        }
        "role_list" => response(client.role_list()),
        "role_get" => response(client.role_get(&argument::<String>(&args, "id")?)),
        "role_create" => response(client.role_create(parse(args)?)),
        "role_update" => response(
            client.role_update(&argument::<String>(&args, "id")?, argument(&args, "input")?),
        ),
        "role_delete" => {
            let id: String = argument(&args, "id")?;
            response(
                client
                    .role_delete(&id)
                    .map(|()| json!({"deleted": true, "id": id})),
            )
        }
        "slot_list" => response(client.slot_list(argument::<String>(&args, "crew_id")?.as_str())),
        "slot_create" => response(client.slot_create(parse(args)?)),
        "slot_update" => response(client.slot_update(
            &argument::<String>(&args, "slot_id")?,
            argument(&args, "input")?,
        )),
        "slot_delete" => {
            let id: String = argument(&args, "slot_id")?;
            response(
                client
                    .slot_delete(&id)
                    .map(|()| json!({"deleted": true, "slot_id": id})),
            )
        }
        "role_get_by_handle" => {
            response(client.role_get_by_handle(&argument::<String>(&args, "handle")?))
        }
        "slot_set_lead" => response(client.slot_set_lead(&argument::<String>(&args, "slot_id")?)),
        "slot_reorder" => response(client.slot_reorder(
            &argument::<String>(&args, "crew_id")?,
            argument(&args, "ordered_slot_ids")?,
        )),
        "project_list" => response(client.project_list()),
        "project_get" => response(client.project_get(&argument::<String>(&args, "id")?)),
        "project_create" => response(
            client.project_create_checked(argument(&args, "name")?, argument(&args, "cwd")?),
        ),
        "project_rename" => {
            response(client.project_rename(argument(&args, "id")?, argument(&args, "name")?))
        }
        "project_delete" => response(
            client.project_delete_checked(
                argument(&args, "id")?,
                args.get("force")
                    .map(|v| parse(v.clone()))
                    .transpose()?
                    .unwrap_or(false),
            ),
        ),
        "mission_list" => response(client.mission_list(argument(&args, "crew_id")?)),
        "mission_list_summary" => {
            response(client.mission_list_summary_impl(argument(&args, "crew_id")?))
        }
        "mission_feed" => response(client.mission_feed(parse(args)?)),
        "mission_status" => response(client.mission_status(&argument::<String>(&args, "id")?)),
        "mission_get" => response(client.mission_get(&argument::<String>(&args, "id")?)),
        "mission_start" => response(client.mission_start_impl_with_size(
            parse::<runner_core::protocol::StartMissionInput>(args)?.into(),
            None,
        )),
        "mission_add_role" => response(client.mission_add_role(parse(args)?)),
        "mission_remove_role" => response(client.mission_remove_role(
            &argument::<String>(&args, "mission_id")?,
            &argument::<String>(&args, "handle")?,
        )),
        "mission_resume" => response(client.mission_resume(&argument::<String>(&args, "id")?)),
        "mission_set_project" => response(client.mission_set_project(
            argument(&args, "mission_id")?,
            argument(&args, "project_id")?,
        )),
        "mission_pin" => {
            response(client.mission_pin_impl(argument(&args, "id")?, argument(&args, "pinned")?))
        }
        "mission_rename" => {
            response(client.mission_rename_impl(argument(&args, "id")?, argument(&args, "title")?))
        }
        "session_list" => response(client.session_list_with_activity()),
        "session_get" => {
            response(client.session_get_with_status(&argument::<String>(&args, "session_id")?))
        }
        "session_start_direct" => response(client.session_start_chat(parse(args)?)),
        "mission_stop" => response(client.mission_stop_impl(argument(&args, "id")?)),
        "mission_archive" => response(client.mission_archive_impl(argument(&args, "id")?)),
        "mission_unarchive" => response(client.mission_unarchive_impl(argument(&args, "id")?)),
        "mission_read_cursor" => response(client.mission_read_cursor(
            &argument::<String>(&args, "mission_id")?,
            &argument::<String>(&args, "handle")?,
        )),
        "mission_post" => response(client.mission_post_impl(parse(args)?)),
        "mission_signal" => response(client.mission_signal_impl(parse(args)?)),
        "session_resume" => {
            response(client.session_resume(&argument::<String>(&args, "session_id")?, None, None))
        }
        "session_restart" => {
            response(client.session_restart(&argument::<String>(&args, "session_id")?, None, None))
        }
        "session_stop" => {
            let id: String = argument(&args, "session_id")?;
            response(client.session_stop(&id).map(|()| json!({"session_id": id})))
        }
        "session_archive" => {
            let id: String = argument(&args, "session_id")?;
            response(
                client
                    .session_archive_direct(&id)
                    .map(|()| json!({"session_id": id})),
            )
        }
        _ => Err(ClientError::Refused("tool not found".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn denied_existing_endpoint_is_blocked() {
        assert_eq!(
            connect_failure(std::io::ErrorKind::PermissionDenied, true),
            ClientError::Blocked
        );
        assert_eq!(
            connect_failure(std::io::ErrorKind::PermissionDenied, false),
            ClientError::NotRunning
        );
        assert_eq!(
            connect_failure(std::io::ErrorKind::NotFound, false),
            ClientError::NotRunning
        );
    }
}

#[cfg(all(test, unix))]
mod skew_tests {
    use super::*;
    use wire::Frame;

    #[tokio::test]
    async fn old_daemon_without_version_request_reports_restart_instead_of_not_running() {
        let root = tempfile::tempdir().unwrap();
        let endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&endpoint.0).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let hello: wire::Hello = Frame::read(&mut stream).unwrap().decode().unwrap();
            assert!(hello.exe_sha256.is_empty());
            Frame::json(
                wire::WELCOME,
                &wire::Welcome {
                    exe_sha256: "old".into(),
                    pid: 1,
                    started_at: "now".into(),
                },
            )
            .unwrap()
            .write(&mut stream)
            .unwrap();
            let call: wire::Call = Frame::read(&mut stream).unwrap().decode().unwrap();
            assert!(matches!(
                call.request,
                runner_core::protocol::Request::app_version { .. }
            ));
        });
        let error = SocketClient::connect_endpoint(endpoint)
            .await
            .err()
            .unwrap();
        assert_eq!(error, ClientError::Protocol(VERSION_SKEW_MESSAGE.into()));
        server.join().unwrap();
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use runner_core::protocol::{ClientError as ProtocolError, Request, Response, Transport};

    struct Cursor;
    impl Transport for Cursor {
        fn call(&self, request: Request) -> Result<Response, ProtocolError> {
            let Request::mission_read_cursor { mission_id, handle } = request else {
                panic!("wrong request")
            };
            assert_eq!((mission_id.as_str(), handle.as_str()), ("m", "coder"));
            Ok(Response::mission_read_cursor(Ok(Some(
                runner_core::protocol::MissionReadCursor {
                    start_offset: 123,
                    ack_id: Some("ack".into()),
                    ack_offset: 456,
                },
            ))))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            panic!("read-only query does not subscribe")
        }
    }
    #[cfg(unix)]
    #[test]
    fn cursor_query_uses_existing_socket_and_missing_or_unavailable_cursor_falls_back() {
        use wire::Frame;
        let dir = tempfile::tempdir().unwrap();
        let endpoint = IpcEndpoint(dir.path().join("cursor.sock"));
        assert!(mission_read_cursor_at(&endpoint, "m", "coder").is_none());
        assert!(!endpoint.0.exists());
        let listener = std::os::unix::net::UnixListener::bind(&endpoint.0).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let hello: wire::Hello = Frame::read(&mut stream).unwrap().decode().unwrap();
            assert!(hello.exe_sha256.is_empty());
            Frame::json(
                wire::WELCOME,
                &wire::Welcome {
                    exe_sha256: "existing".into(),
                    pid: 1,
                    started_at: "now".into(),
                },
            )
            .unwrap()
            .write(&mut stream)
            .unwrap();
            let call: wire::Call = Frame::read(&mut stream).unwrap().decode().unwrap();
            assert!(
                matches!(call.request, Request::mission_read_cursor { mission_id, handle } if mission_id == "m" && handle == "coder")
            );
            Frame::json(
                wire::RESPONSE,
                &wire::Reply {
                    id: call.id,
                    response: Response::mission_read_cursor(Ok(None)),
                },
            )
            .unwrap()
            .write(&mut stream)
            .unwrap();
        });
        assert!(mission_read_cursor_at(&endpoint, "m", "coder").is_none());
        server.join().unwrap();
    }

    #[test]
    fn cursor_query_uses_typed_cli_adapter() {
        let client = DaemonClient::new(std::sync::Arc::new(Cursor));
        let result = call_daemon(
            &client,
            "mission_read_cursor",
            json!({"mission_id":"m","handle":"coder"}),
        )
        .unwrap();
        assert_eq!(
            result.value,
            json!({"start_offset":123,"ack_id":"ack","ack_offset":456})
        );
    }
}

#[cfg(test)]
mod remote_tests {
    use super::*;
    struct NoCalls;
    impl runner_core::protocol::Transport for NoCalls {
        fn call(
            &self,
            _: runner_core::protocol::Request,
        ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
            panic!("remote tools must never reach the daemon through runner call");
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            panic!("no subscription expected");
        }
    }
    #[test]
    fn generic_call_has_no_remote_requests() {
        let client = DaemonClient::new(std::sync::Arc::new(NoCalls));
        for name in [
            "remote_request",
            "remote_overview",
            "remote_follow",
            "remote_take_control",
            "remote_status",
            "remote_enroll_start",
            "remote_enroll_confirm",
            "remote_enroll_cancel",
            "remote_set_paused",
            "remote_retry",
            "remote_logout",
            "remote_trust",
            "remote_set_pin",
            "remote_unlock_pin",
            "remote_revoke_device",
            "remote_reset_identity",
        ]
        .into_iter()
        .chain(runner_core::companion::ActionRequest::NAMES.iter().copied())
        {
            assert!(
                matches!(call_daemon(&client, name, json!({})), Err(ClientError::Refused(message)) if message == "tool not found")
            );
        }
    }
}
