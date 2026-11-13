use super::*;
use std::path::PathBuf;

#[derive(Debug)]
pub(super) struct CliError {
    pub(super) code: i32,
    pub(super) message: String,
}

impl CliError {
    pub(super) fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }
}

impl From<ClientError> for CliError {
    fn from(error: ClientError) -> Self {
        let code = match error {
            ClientError::NotRunning => 3,
            ClientError::Blocked => 5,
            ClientError::Refused(_) | ClientError::Protocol(_) => 1,
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

pub(super) trait Caller {
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        Box::new(ClosedSubscription)
    }
    async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, CliError>;
}

pub(super) struct ClosedSubscription;
impl EventSubscription for ClosedSubscription {
    fn recv(
        &mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ClientEvent, EventError>> + Send + '_>,
    > {
        Box::pin(async { Err(EventError::Closed) })
    }
}

impl Caller for SocketClient {
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        SocketClient::subscribe(self)
    }
    async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, CliError> {
        SocketClient::call(self, name, arguments)
            .await
            .map_err(Into::into)
    }
}

pub(super) fn output_view(command: &Command) -> output::View {
    use output::View;

    match command {
        Command::Remote { .. } | Command::Daemon { .. } | Command::Hook { .. } => View::Generic,
        Command::Status => View::Status,
        Command::Project { command } => match command {
            ProjectCommand::List => View::ProjectList,
            ProjectCommand::Show { .. }
            | ProjectCommand::Create { .. }
            | ProjectCommand::Rename { .. } => View::Project,
            ProjectCommand::Delete { .. } => View::Confirmation("Deleted project"),
        },
        Command::Role { command } => match command {
            RoleCommand::List => View::RoleList,
            RoleCommand::Show { .. } | RoleCommand::Create { .. } | RoleCommand::Update { .. } => {
                View::Role
            }
            RoleCommand::Delete { .. } => View::Confirmation("Deleted role"),
        },
        Command::Crew { command } => match command {
            CrewCommand::List => View::CrewList,
            CrewCommand::Show { .. } => View::CrewShow,
            CrewCommand::Create { .. } | CrewCommand::Update { .. } => View::Crew,
            CrewCommand::Delete { .. } => View::Confirmation("Deleted crew"),
            CrewCommand::Add { .. }
            | CrewCommand::Set { .. }
            | CrewCommand::Remove { .. }
            | CrewCommand::Order { .. } => View::Confirmation("Updated crew slots"),
        },
        Command::Mission { command } => match command {
            MissionCommand::List { .. } => View::MissionList,
            MissionCommand::Show { .. } => View::MissionShow,
            MissionCommand::Feed { .. } => View::MissionFeed,
            MissionCommand::Resume { .. } => View::Mission,
            MissionCommand::AddRole { .. } => View::RoleChange("Added role"),
            MissionCommand::RemoveRole { .. } => View::RoleChange("Removed role"),
            MissionCommand::Answer { .. } => View::Confirmation("Answered question"),
            MissionCommand::Link { .. } | MissionCommand::Unlink { .. } => View::MissionLink,
            MissionCommand::Start { .. }
            | MissionCommand::Stop { .. }
            | MissionCommand::Archive { .. }
            | MissionCommand::Unarchive { .. }
            | MissionCommand::Rename { .. }
            | MissionCommand::Pin { .. }
            | MissionCommand::Unpin { .. }
            | MissionCommand::Move { .. } => View::Mission,
        },
        Command::Chat { .. } => View::Confirmation("Started session"),
        Command::Session { command } => match command {
            SessionCommand::List => View::SessionList,
            SessionCommand::Show { .. } => View::SessionShow,
            SessionCommand::Stop { .. } => View::Confirmation("Stopped session"),
            SessionCommand::Archive { .. } => View::SessionArchive,
            SessionCommand::Resume { .. } => View::Confirmation("Resumed session"),
            SessionCommand::Restart { .. } => View::Confirmation("Restarted session"),
        },
        Command::Msg { .. } => View::Confirmation("Posted message"),
        Command::Signal { .. } => View::Confirmation("Posted signal"),
        Command::Ask(_) => View::Confirmation("Posted question"),
        Command::Call { .. } => View::Generic,
        Command::Help { .. } => unreachable!(),
    }
}

pub(super) fn postprocess_response(cli: &Cli, response: ToolResponse) -> ToolResponse {
    let Command::Mission {
        command:
            MissionCommand::Feed {
                follow: false,
                types,
                from,
                all,
                ..
            },
    } = &cli.command
    else {
        return response;
    };
    let filter = FeedFilter::new(types.as_deref(), from.as_deref(), *all)
        .expect("feed filters were validated before connecting");
    if cli.json && !filter.is_explicit() {
        response
    } else {
        filter_feed_response(&response, &filter)
    }
}

pub(super) fn mission_signal_args(
    mission_id: &str,
    signal_type: &str,
    payload: Value,
    from: Option<&str>,
) -> Value {
    let mut args = json!({
        "mission_id": mission_id,
        "signal_type": signal_type,
        "payload": payload,
    });
    insert_string(&mut args, "from", from);
    args
}

pub(super) async fn resolve_scoped_mission(
    client: &impl Caller,
    target: Option<&str>,
    context: &BusContext,
) -> Result<String, CliError> {
    match target {
        Some(target) => resolve_mission(client, target).await,
        None => match context {
            BusContext::Mission(mission) => Ok(mission.mission_id.clone()),
            BusContext::OffBus => Err(CliError::usage(
                "runner: --mission is required outside a mission",
            )),
            BusContext::Partial { .. } => unreachable!(),
        },
    }
}

pub(super) async fn resolve_mission_arg(
    client: &impl Caller,
    target: Option<&str>,
    context: &BusContext,
) -> Result<String, CliError> {
    match target {
        Some(target) => resolve_mission(client, target).await,
        None => match context {
            BusContext::Mission(mission) => Ok(mission.mission_id.clone()),
            BusContext::OffBus => Err(CliError::usage("mission reference is required")),
            BusContext::Partial { .. } => unreachable!(),
        },
    }
}

pub(super) async fn resolve_mission(
    client: &impl Caller,
    target: &str,
) -> Result<String, CliError> {
    match resolve_prefix(client, "mission", "mission_list", target).await {
        Ok(id) => Ok(id),
        Err(error) if target.len() == 26 && error.code == 2 => {
            match call(client, "mission_get", json!({"id": target})).await {
                Ok(_) => Ok(target.to_owned()),
                Err(fallback) if fallback.code == 1 => Err(error),
                Err(fallback) => Err(fallback),
            }
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone)]
pub(super) struct Resolved {
    pub(super) id: String,
    pub(super) name: String,
}

pub(super) async fn resolve_named(
    client: &impl Caller,
    kind: &str,
    tool: &str,
    target: &str,
) -> Result<Resolved, CliError> {
    let rows = list(client, tool, json!({})).await?;
    resolve_named_rows(&rows, kind, target)
}

pub(super) fn resolve_named_rows(
    rows: &[Value],
    kind: &str,
    target: &str,
) -> Result<Resolved, CliError> {
    if let Some(row) = rows.iter().find(|row| field(row, "id") == Some(target)) {
        return Ok(Resolved {
            id: target.to_owned(),
            name: field(row, "name").unwrap_or(target).to_owned(),
        });
    }
    let matches = rows
        .iter()
        .filter(|row| field(row, "name") == Some(target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [row] => Ok(Resolved {
            id: field(row, "id").unwrap().to_owned(),
            name: target.to_owned(),
        }),
        [] => Err(CliError::usage(format!("{kind} not found: {target}"))),
        rows => Err(CliError::usage(format!(
            "ambiguous {kind} name {target:?}: {}",
            rows.iter()
                .filter_map(|row| field(row, "id"))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

pub(super) async fn resolve_role(client: &impl Caller, handle: &str) -> Result<Resolved, CliError> {
    let rows = list(client, "role_list", json!({})).await?;
    resolve_role_rows(&rows, handle)
}

pub(super) async fn resolve_session(
    client: &impl Caller,
    target: &str,
) -> Result<String, CliError> {
    match resolve_prefix(client, "session", "session_list", target).await {
        Ok(id) => Ok(id),
        Err(error) if target.len() == 26 && error.code == 2 => Ok(target.to_owned()),
        Err(error) => Err(error),
    }
}

pub(super) fn resolve_role_rows(rows: &[Value], handle: &str) -> Result<Resolved, CliError> {
    let handle = handle.strip_prefix('@').unwrap_or(handle);
    let row = rows
        .iter()
        .find(|row| field(row, "handle") == Some(handle))
        .ok_or_else(|| CliError::usage(format!("role handle not found: @{handle}")))?;
    Ok(Resolved {
        id: field(row, "id").unwrap().to_owned(),
        name: handle.to_owned(),
    })
}

pub(super) async fn resolve_slot(
    client: &impl Caller,
    crew_id: &str,
    handle: &str,
) -> Result<Resolved, CliError> {
    let rows = list(client, "slot_list", json!({"crew_id": crew_id})).await?;
    let row = rows
        .iter()
        .find(|row| field(row, "slot_handle") == Some(handle))
        .ok_or_else(|| CliError::usage(format!("crew slot handle not found: {handle}")))?;
    Ok(Resolved {
        id: field(row, "id").unwrap().to_owned(),
        name: handle.to_owned(),
    })
}

pub(super) async fn resolve_prefix(
    client: &impl Caller,
    kind: &str,
    tool: &str,
    target: &str,
) -> Result<String, CliError> {
    let rows = list(client, tool, json!({})).await?;
    resolve_prefix_rows(&rows, kind, target)
}

pub(super) fn resolve_prefix_rows(
    rows: &[Value],
    kind: &str,
    target: &str,
) -> Result<String, CliError> {
    if rows.iter().any(|row| reference_id(row) == Some(target)) {
        return Ok(target.to_owned());
    }
    let matches = rows
        .iter()
        .filter_map(reference_id)
        .filter(|id| id.starts_with(target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [id] => Ok((*id).to_owned()),
        [] => Err(CliError::usage(format!("{kind} not found: {target}"))),
        ids => Err(CliError::usage(format!(
            "ambiguous {kind} id prefix {target:?}: {}",
            ids.join(", ")
        ))),
    }
}

pub(super) fn reference_id(value: &Value) -> Option<&str> {
    field(value, "id").or_else(|| field(value, "session_id"))
}

pub(super) async fn list(
    client: &impl Caller,
    tool: &str,
    args: Value,
) -> Result<Vec<Value>, CliError> {
    let response = call(client, tool, args).await?;
    response.value.as_array().cloned().ok_or_else(|| CliError {
        code: 1,
        message: format!("{tool} returned a non-list result"),
    })
}

pub(super) async fn call(
    client: &impl Caller,
    tool: &str,
    args: Value,
) -> Result<ToolResponse, CliError> {
    client.call(tool, args).await
}

pub(super) fn runtime_command(runtime: &str) -> Result<&'static str, CliError> {
    runner_core::Runtime::parse(runtime).and_then(runner_core::Runtime::command).ok_or_else(|| CliError::usage(format!(
        "unknown role runtime {runtime:?}; expected codex, claude-code, antigravity, pi, copilot, trae, cursor, or grok"
    )))
}

pub(super) fn validate_permission(permission: Option<&str>) -> Result<(), CliError> {
    match permission {
        None | Some("default" | "accept_edits" | "auto" | "bypass") => Ok(()),
        Some(other) => Err(CliError::usage(format!(
            "unknown permission mode {other:?}; expected default, accept_edits, auto, or bypass"
        ))),
    }
}

pub(super) fn parse_env(values: &[String]) -> Result<BTreeMap<String, String>, CliError> {
    values
        .iter()
        .map(|value| {
            value
                .split_once('=')
                .filter(|(key, _)| !key.is_empty())
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .ok_or_else(|| {
                    CliError::usage(format!("invalid --env {value:?}; expected KEY=VALUE"))
                })
        })
        .collect()
}

pub(super) fn read_text(
    inline: Option<&str>,
    file: Option<&Path>,
    label: &str,
) -> Result<Option<String>, CliError> {
    if let Some(value) = inline {
        Ok(Some(value.to_owned()))
    } else if let Some(path) = file {
        read_file(path, label).map(Some)
    } else {
        Ok(None)
    }
}

pub(super) fn read_file(path: &Path, label: &str) -> Result<String, CliError> {
    if path == Path::new("-") {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map_err(|error| {
            CliError::usage(format!("failed to read {label} from stdin: {error}"))
        })?;
        Ok(text)
    } else {
        std::fs::read_to_string(path).map_err(|error| {
            CliError::usage(format!(
                "failed to read {label} file {}: {error}",
                path.display()
            ))
        })
    }
}

pub(super) fn absolute_path(path: &Path) -> Result<String, CliError> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|error| CliError::usage(format!("failed to read current directory: {error}")))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized.to_string_lossy().into_owned())
}

pub(super) fn parse_json_object(value: &str) -> Result<Value, CliError> {
    let value: Value = serde_json::from_str(value)
        .map_err(|error| CliError::usage(format!("invalid JSON arguments: {error}")))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(CliError::usage("tool arguments must be a JSON object"))
    }
}

pub(super) fn response(value: Value) -> Result<ToolResponse, CliError> {
    let raw_json = serde_json::to_string(&value).map_err(|error| CliError {
        code: 1,
        message: error.to_string(),
    })?;
    Ok(ToolResponse { value, raw_json })
}

pub(super) fn field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

pub(super) fn id_from(value: &Value) -> Result<String, CliError> {
    ["id", "slot_id", "session_id"]
        .iter()
        .find_map(|key| field(value, key))
        .map(str::to_owned)
        .ok_or_else(|| CliError {
            code: 1,
            message: "tool result did not contain an id".to_owned(),
        })
}

pub(super) fn insert_value(target: &mut Value, key: &str, value: Value) {
    target
        .as_object_mut()
        .expect("command arguments are objects")
        .insert(key.to_owned(), value);
}

pub(super) fn insert_string(target: &mut Value, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        insert_value(target, key, json!(value));
    }
}

pub(super) fn insert_speed(target: &mut Value, key: &str, speed: Option<SpeedArg>) {
    if let Some(speed) = speed {
        insert_value(target, key, speed.value());
    }
}

pub(super) fn insert_clearable(target: &mut Value, key: &str, value: Option<&str>, clear: bool) {
    if let Some(value) = value {
        insert_value(
            target,
            key,
            if clear { nullable(value) } else { json!(value) },
        );
    }
}

pub(super) fn nullable(value: &str) -> Value {
    if value.is_empty() {
        Value::Null
    } else {
        json!(value)
    }
}
