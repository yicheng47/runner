use super::terminal::*;
use super::*;
use super::{agent_skill::*, command::*, skills::*};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

trait Sample {
    fn sample() -> Self;
}
macro_rules! scalar { ($($ty:ty => $value:expr),* $(,)?) => { $(impl Sample for $ty { fn sample() -> Self { $value } })* }; }
scalar!(() => (), bool => true, u8 => 7, u16 => 80, u32 => 3, u64 => 7, usize => 2, i32 => -2, i64 => -3, f64 => 42.5, String => "样例\nvalue".into(), PathBuf => PathBuf::from("sample/路径"), serde_json::Value => serde_json::json!({"nested": [null, true, 42, "样例"]}), chrono::DateTime<chrono::Utc> => "2026-10-05T00:00:00Z".parse().unwrap());
impl<T: Sample> Sample for Option<T> {
    fn sample() -> Self {
        Some(T::sample())
    }
}
impl<T: Sample> Sample for Vec<T> {
    fn sample() -> Self {
        vec![T::sample()]
    }
}
impl<K: Sample + Ord, V: Sample> Sample for BTreeMap<K, V> {
    fn sample() -> Self {
        [(K::sample(), V::sample())].into()
    }
}
impl<K: Sample + Eq + std::hash::Hash, V: Sample> Sample for HashMap<K, V> {
    fn sample() -> Self {
        [(K::sample(), V::sample())].into()
    }
}
impl<A: Sample, B: Sample> Sample for (A, B) {
    fn sample() -> Self {
        (A::sample(), B::sample())
    }
}
impl<A: Sample, B: Sample> Sample for Result<A, B> {
    fn sample() -> Self {
        Ok(A::sample())
    }
}
macro_rules! record { ($ty:ident { $($field:ident),* $(,)? }) => { impl Sample for $ty { fn sample() -> Self { Self { $($field: Sample::sample()),* } } } }; }
macro_rules! variant {
    ($ty:ident => $value:expr) => {
        impl Sample for $ty {
            fn sample() -> Self {
                $value
            }
        }
    };
}
variant!(Activity => Self::Working);
record!(AgentObservation {
    activity,
    source,
    outcome,
    interactions,
    detail
});
record!(AgentStatus {
    lifecycle,
    observation,
    exit_code,
    error_since,
    failed_since,
    unread_since
});
record!(AgentUsage {
    windows,
    updated_at
});
record!(AutoResumeReport { resumed, errors });
variant!(CodexSpeed => Self::Standard);
variant!(CommandActionOutcome => Self::Installed(Sample::sample()));
record!(CommandInstallInputs {
    home,
    login_path,
    system_path,
    sidecar,
    local_bin,
    system_bin,
    system_bin_writable,
    debug,
    platform
});
variant!(CommandPlatform => Self::Unix);
record!(CreateCrewInput {
    name,
    system_prompt_addendum
});
record!(CreateRoleInput {
    handle,
    display_name,
    runtime,
    command,
    args,
    working_dir,
    system_prompt,
    env,
    model,
    effort,
    codex_speed,
    permission_mode
});
record!(CreateSlotInput {
    crew_id,
    role_id,
    slot_handle,
    runtime_override,
    model_override
});
record!(Crew {
    id,
    name,
    system_prompt_addendum,
    created_at,
    updated_at
});
record!(CrewListItem {
    crew,
    role_count,
    members
});
record!(CrewMemberPreview {
    slot_handle,
    role_handle,
    runtime,
    lead
});
record!(CrewMembership {
    crew_id,
    crew_name,
    slot_id,
    slot_handle,
    lead,
    position,
    added_at
});
record!(DirectSessionEntry {
    session_id,
    project_id,
    role_id,
    handle,
    agent_runtime,
    agent_command,
    agent_model,
    agent_effort,
    display_name,
    status,
    title,
    live_title,
    cwd,
    started_at,
    stopped_at,
    resumable,
    native_fork,
    forkable,
    agent_session_key,
    pinned,
    archived_at
});
variant!(DiscoveryOutcome => Self::Ok);
record!(DiscoveryResult {
    shell,
    outcome,
    duration_ms,
    env
});
record!(DiscoverySnapshot {
    checking,
    result,
    shell_env
});
record!(Event {
    id,
    ts,
    crew_id,
    mission_id,
    kind,
    from,
    to,
    signal_type,
    payload
});
variant!(EventKind => Self::Signal);
variant!(GlobalState => Self::On);
record!(HumanInteraction {
    id,
    reason,
    owners,
    since
});
variant!(InstallOutcome => Self::Installed);
variant!(Lifecycle => Self::Starting);
impl<T: Sample> Sample for ListPage<T> {
    fn sample() -> Self {
        Self {
            items: Sample::sample(),
            total_count: 2,
            filtered_count: 1,
        }
    }
}
record!(LoginShellEnv { path, vars });
record!(McpCatalog { servers });
variant!(McpClientId => Self::ClaudeCode);
record!(McpClientStatus {
    registered,
    matches_current,
    command,
    args
});
record!(McpServerClientEntry {
    registered,
    native_text,
    definition,
    conflicting,
    error
});
variant!(McpServerDefinition => Self::Stdio { command: Sample::sample(), args: Sample::sample(), env: Sample::sample() });
record!(McpServerEntry { name, clients });
record!(Mission {
    id,
    crew_id,
    project_id,
    title,
    status,
    goal_override,
    cwd,
    started_at,
    stopped_at,
    pinned_at,
    archived_at
});
variant!(MissionActivityState => Self::Busy);
record!(MissionStart {
    crew_id,
    scope,
    title,
    goal_override,
    cwd
});
variant!(MissionStatus => Self::Running);
record!(MissionSummary {
    session_statuses,
    mission,
    crew_name,
    pending_ask_count,
    any_session_live,
    all_sessions_live,
    activity
});
record!(NodeRow {
    id,
    parent_id,
    position,
    node_type,
    name,
    ref_id,
    layout,
    pinned_position,
    last_completed_at,
    last_viewed_at,
    created_at
});
record!(NodeTabUpsertInput {
    id,
    parent_id,
    name,
    layout
});
variant!(NodeType => Self::Project);
variant!(ObservationSource => Self::Hook);
record!(OverrideValidationError { code, message });
variant!(PermissionMode => Self::Default);
record!(PostMessageInput {
    mission_id,
    from,
    text,
    to
});
record!(PostSignalInput {
    mission_id,
    from,
    signal_type,
    payload
});
record!(ProjectDeleteOutcome {
    archived_mission_ids,
    archived_session_ids
});
record!(ProjectRow {
    id,
    name,
    cwd,
    position,
    created_at
});
variant!(ProjectScope => Self::Infer);
variant!(RefreshReason => Self::Launch);
record!(Role {
    id,
    handle,
    display_name,
    runtime,
    command,
    args,
    working_dir,
    system_prompt,
    env,
    model,
    effort,
    codex_speed,
    created_at,
    updated_at
});
record!(RoleActivity {
    role_id,
    active_sessions,
    active_missions,
    crew_count,
    last_started_at,
    direct_session_id
});
record!(RoleWithActivity { role, activity });
variant!(RunnerCommandState => Self::NotInstalled);
record!(RunnerCommandStatus {
    state,
    path,
    shadowed_by
});
variant!(Runtime => Self::Codex);
record!(RuntimeCapabilities {
    usage,
    global_skill_toggle,
    skill_toggle_requires_marker,
    codex_speed,
    effort_needs_launch_model
});
variant!(RuntimeCatalogEntry => Self::for_runtime(Runtime::Codex).unwrap());
record!(RuntimeCatalogOption {
    value,
    label,
    description,
    supported_efforts
});
variant!(RuntimeCommandSource => Self::Override);
record!(RuntimeExecutableStatus {
    name,
    display_name,
    command,
    default_model,
    default_effort,
    detected_path,
    override_path,
    effective_command,
    effective_source,
    state,
    invalid_reason,
    installed_version,
    available_version
});
variant!(RuntimeRowState => Self::Detected);
record!(RuntimeStatusResponse { shell, runtimes });
record!(RuntimeUsage { value, error });
record!(Session {
    id,
    mission_id,
    role_id,
    slot_id,
    cwd,
    status,
    pid,
    started_at,
    stopped_at
});
variant!(SessionActivityState => Self::Busy);
record!(SessionRow {
    session,
    live_title,
    handle,
    runtime,
    lead,
    agent_session_key
});
variant!(SessionStatus => Self::Running);
record!(ShellDiscoveryStatus {
    shell,
    outcome,
    duration_ms,
    checking,
    using_last_known_good,
    last_known_good_captured_at
});
variant!(SignalType => Self(Sample::sample()));
record!(SkillCatalog {
    runtime,
    roots,
    root_exists,
    entries
});
record!(SkillEntry {
    name,
    description,
    path,
    marker,
    symlink,
    manual,
    hidden,
    problem,
    files,
    global
});
variant!(SkillRootState => Self::Missing);
record!(SkillRootStatus {
    root,
    folder,
    state
});
record!(Slot {
    id,
    crew_id,
    role_id,
    slot_handle,
    position,
    lead,
    runtime_override,
    model_override,
    effort_override,
    codex_speed_override,
    added_at
});
record!(SlotWithRole { slot, role });
record!(SpawnedSession {
    id,
    mission_id,
    role_id,
    handle,
    pid
});
record!(StartDirectSessionOutput {
    session,
    project_id,
    cwd
});
record!(StartMissionOutput { mission, goal });
variant!(Subject => Self::Mission(Sample::sample()));
variant!(TurnOutcome => Self::Completed);
variant!(UnavailableReason => Self::SignIn);
record!(UpdateCrewInput {
    name,
    system_prompt_addendum
});
record!(UpdateRoleInput {
    display_name,
    runtime,
    command,
    args,
    working_dir,
    system_prompt,
    env,
    model,
    effort,
    codex_speed,
    permission_mode
});
record!(UpdateSlotInput {
    slot_handle,
    runtime_override,
    model_override,
    effort_override,
    codex_speed_override
});
record!(UsageSnapshot {
    runtimes,
    last_fetch_at,
    refreshing
});
record!(UsageWindow {
    name,
    used_percent,
    resets_at
});
variant!(WaitReason => Self::Approval);
record!(WindowEntry {
    label,
    subjects,
    viewed_session_id,
    focused_at,
    focused
});
variant!(WorkDetail => Self::UsingTools);
fn round_trip<T: Serialize + DeserializeOwned + std::fmt::Debug>(value: T) {
    let bytes = serde_json::to_vec(&value).unwrap();
    let decoded: T = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(format!("{value:?}"), format!("{decoded:?}"));
}

macro_rules! round_trip_api {
    ($( $name:ident($( $arg:ident: $client_ty:ty => $wire_ty:ty = $convert:block ),*) -> $out:ty [$fast:literal] => $handler:expr; )*) => {
        #[test]
        fn every_request_and_success_or_error_response_crosses_json() {
            $(round_trip(Request::$name { $($arg: <$wire_ty>::sample()),* });
            round_trip(Response::$name(Ok(<$out>::sample())));
            round_trip(Response::$name(Err(ClientError::msg("daemon error 样例"))));)*
        }
    };
}
crate::daemon_api!(round_trip_api);

#[test]
fn updates_preserve_omitted_clear_and_set_fields() {
    for value in [None, Some(None), Some(Some("new model".to_owned()))] {
        let mut role = UpdateRoleInput::sample();
        role.model = value.clone();
        let decoded: UpdateRoleInput =
            serde_json::from_slice(&serde_json::to_vec(&role).unwrap()).unwrap();
        assert_eq!(decoded.model, value);
        let mut slot = UpdateSlotInput::sample();
        slot.model_override = value.clone();
        let decoded: UpdateSlotInput =
            serde_json::from_slice(&serde_json::to_vec(&slot).unwrap()).unwrap();
        assert_eq!(decoded.model_override, value);
    }
    round_trip(ClientEvent::sample());
}

impl<A: Sample, B: Sample, C: Sample> Sample for (A, B, C) {
    fn sample() -> Self {
        (A::sample(), B::sample(), C::sample())
    }
}
record!(ClientEvent { name, payload });

record!(TerminalMetadata {
    title,
    live_cwd,
    link_cwd,
    cols,
    rows
});
record!(RuntimeUpdateCommand {
    session_id,
    command,
    args,
    cwd,
    env,
    shell_path,
    size
});
impl Sample for InputEvent {
    fn sample() -> Self {
        Self::Paste {
            text: String::sample(),
        }
    }
}
impl Sample for TerminalPalette {
    fn sample() -> Self {
        Self {
            background: [1, 2, 3],
            foreground: [4, 5, 6],
            cursor: [7, 8, 9],
            cursor_accent: [10, 11, 12],
            selection: [13, 14, 15],
            ansi: [[16, 17, 18]; 16],
        }
    }
}
