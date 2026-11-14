use super::terminal::*;
use super::*;
use super::{agent_skill::*, command::*, skills::*};
use crate::model::Recipients;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

impl Sample for crate::companion::ClientMessage {
    fn sample() -> Self {
        Self::Overview(crate::companion::OverviewRequest { id: 7 })
    }
}
impl Sample for crate::remote::RemoteResult {
    fn sample() -> Self {
        Self {
            result: serde_json::json!({"session":"s"}),
            features: vec!["startChat".into()],
        }
    }
}
impl Sample for crate::remote::Computer {
    fn sample() -> Self {
        Self::default()
    }
}
impl Sample for crate::remote::RemoteOverview {
    fn sample() -> Self {
        Self(crate::companion::Overview {
            projects: Vec::new(),
            tabs: Vec::new(),
            missions: Vec::new(),
            sessions: BTreeMap::new(),
            sidebar: None,
            terminal_palette: None,
        })
    }
}
impl Sample for crate::remote::RemoteTrust {
    fn sample() -> Self {
        Self::default()
    }
}
impl Sample for crate::remote::Secret {
    fn sample() -> Self {
        Self("0123456789".into())
    }
}
impl Sample for crate::remote::RemoteStatus {
    fn sample() -> Self {
        Self::default()
    }
}
impl Sample for crate::remote::RemoteLogout {
    fn sample() -> Self {
        Self::Removed
    }
}

trait Sample {
    fn sample() -> Self;
}
macro_rules! scalar { ($($ty:ty => $value:expr_2021),* $(,)?) => { $(impl Sample for $ty { fn sample() -> Self { $value } })* }; }
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
    ($ty:ident => $value:expr_2021) => {
        impl Sample for $ty {
            fn sample() -> Self {
                $value
            }
        }
    };
}
variant!(Activity => Self::Working);
record!(HookReport {
    bridge_unavailable,
    version,
    runtime,
    session_id,
    generation,
    event,
    payload,
    caller_thread_id
});
record!(AgentObservation {
    activity,
    source,
    outcome,
    interactions,
    detail,
    program_status,
    background
});
record!(ProgramStatusEvidence { message });
record!(StartupError {
    runtime,
    message,
    output
});
record!(AgentStatus {
    startup_error,
    lifecycle,
    observation,
    exit_code,
    error_since,
    failed_since,
    unread_since,
    interrupted_since
});
record!(UsageCredits {
    balance,
    has_credits,
    unlimited
});
record!(AgentUsage {
    windows,
    credits,
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
    claude_account,
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
    archived_at,
    linked_chat
});
record!(LinkedChat {
    session_id,
    title,
    archived,
    reports
});
record!(MissionStarter { session_id, title });
record!(MissionLinkInput {
    mission_id,
    session_id,
    no_report,
    caller_session_id
});
variant!(RatingVerdict => Self::Up);
record!(MissionRating {
    verdict,
    note,
    rated_at
});
record!(MissionRateInput {
    mission_id,
    verdict,
    note
});
record!(SlotUsage {
    handle,
    runtime,
    model,
    effort,
    prompt_hash,
    input_tokens,
    output_tokens,
    cached_tokens,
    reasoning_tokens,
    steps,
    input_includes_cached,
    wall_minutes,
    not_recorded_reason,
    recorded_at
});
record!(MissionUsageRecordInput { mission_id, slots });
variant!(BudgetState => Self::Measured);
record!(MissionBudget {
    runtime,
    window,
    state,
    first_percent,
    latest_percent,
    used_points,
    first_read_at,
    latest_read_at,
    readings,
    other_sessions,
    attributed_low,
    attributed_high,
    rounding_margin,
    window_points,
    share_low,
    share_high
});
record!(CallUsage {
    usage_id,
    runtime,
    session_key,
    at,
    input_tokens,
    output_tokens,
    cached_tokens,
    reasoning_tokens,
    points_cached_in_full,
    points_cached_free
});
record!(BackfillSummary { measured, skipped });
record!(MissionUsageSummary {
    mission_id,
    title,
    status,
    started_at,
    stopped_at,
    last_read_at,
    last_call_at,
    runtime,
    attributed_low,
    attributed_high,
    window_points,
    verdict
});
record!(MissionReview {
    mission_id,
    rating,
    slots,
    budget,
    calls
});
variant!(MissionActivityState => Self::Busy);
variant!(MissionFeedOrder => Self::OldestFirst);
record!(MissionFeedArgs {
    mission_id,
    limit,
    order,
    since_offset
});
record!(MissionFeed {
    mission_id,
    events,
    next_offset,
    skipped
});
record!(MissionFeedEntry { next_offset, event });
record!(SkippedEventLine {
    offset,
    next_offset,
    error
});
record!(MissionReadCursor {
    start_offset,
    ack_id,
    ack_offset
});
record!(MissionStatusSnapshot {
    mission,
    phase,
    interrupted_at,
    started_by,
    crew,
    sessions,
    latest_session_status_by_handle,
    pending_asks,
    pending_ask_count,
    live_session_count,
    stopped_session_count,
    crashed_session_count,
    recent_warnings,
    last_event_id,
    last_event_offset,
    skipped_event_count
});
record!(SessionStatusSnapshot {
    state,
    event_id,
    ts,
    source
});
record!(PendingAskSnapshot {
    question_id,
    asker,
    prompt,
    choices,
    on_behalf_of,
    event_id,
    ts
});
record!(MissionWarningSnapshot {
    event_id,
    ts,
    from,
    message,
    payload
});
variant!(MissionControlLane => Self::Backlog);
variant!(MissionControlSourceKind => Self::Ticket);
record!(MissionControlEvidence { kind, path });
record!(MissionControlFieldProvenance { field, evidence });
record!(MissionControlSourceHealth {
    kind,
    path,
    warning
});
record!(MissionControlMission {
    id,
    title,
    active_slots,
    pending_question,
    newest_handoff,
    branch,
    worktree
});
record!(MissionControlCard {
    ticket_id,
    summary,
    severity,
    blockers,
    triage_state,
    lane,
    mission,
    evidence,
    provenance
});
record!(MissionControlSnapshot {
    project_id,
    project_path,
    cards,
    health
});
record!(MissionStart {
    crew_id,
    scope,
    title,
    goal_override,
    cwd,
    started_by_session_id,
    no_report
});
variant!(MissionStatus => Self::Running);
record!(MissionSummary {
    session_statuses,
    mission,
    crew_name,
    pending_ask_count,
    any_session_live,
    all_sessions_live,
    activity,
    phase
});
variant!(MissionPhase => Self::Interrupted);
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
variant!(Recipients => Self::new(vec!["coder".into(), "qa".into()]));
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
record!(ResumeMissionOutput {
    mission_id,
    resumed_session_ids,
    sessions
});
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
    launch_options,
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
    available_version,
    min_version,
    below_minimum
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
    role_handle,
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
record!(StartDirectSessionArgs {
    role_id,
    runtime,
    model,
    effort,
    speed,
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
record!(ClaudeAccount {
    id,
    name,
    folder,
    native
});
record!(ClaudeAuthStatus {
    logged_in,
    email,
    organization,
    plan,
    signing_in,
    sign_in_url,
    error
});
record!(ClaudeAccounts {
    accounts,
    main_id,
    active_id,
    statuses,
    sharing,
    pending_main_id
});
record!(ClaudeSessionAccount {
    account_id,
    folder,
    native,
    pinned,
    origin_folder,
    permission_mode
});
record!(ClaudeAccountSharing {
    using_main,
    own_items,
    shared_skills,
    shared_plugins
});
variant!(ClaudeAccountAction => Self::AddAndLogin { name: Sample::sample(), folder: Sample::sample(), use_main_settings: Sample::sample() });
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
#[test]
fn a_full_file_index_of_the_costliest_paths_fits_a_frame() {
    for piece in ["\u{1}", "\"", "\\", "a"] {
        let path = piece.repeat(4000);
        let mut budget = files::IndexBudget::default();
        let mut paths = Vec::new();
        while budget.take(path.as_bytes()) {
            paths.push(path.clone());
        }
        let encoded = serde_json::to_vec(&paths).unwrap().len();
        assert!(encoded <= files::MAX_INDEX_BYTES, "{piece:?}: {encoded}");
        let response = Response::files_index(Ok(files::FileIndex {
            paths,
            truncated: true,
        }));
        let frame = wire::Frame::json(wire::RESPONSE, &response).unwrap();
        assert!(frame.check_size().unwrap() <= wire::MAX_FRAME, "{piece:?}");
    }
}

#[test]
fn the_index_budget_stops_at_either_limit() {
    let mut files = files::IndexBudget::new(2, 100);
    assert!(files.take(b"a") && files.take(b"b") && !files.take(b"c"));
    let mut bytes = files::IndexBudget::new(100, 8);
    assert!(bytes.take(b"aaaaa"), "five bytes, two quotes and a comma");
    assert!(!bytes.take(b""), "nothing left for an empty string's three");
}

fn round_trip<T: Serialize + DeserializeOwned + std::fmt::Debug>(value: T) {
    let bytes = serde_json::to_vec(&value).unwrap();
    let decoded: T = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(format!("{value:?}"), format!("{decoded:?}"));
}

impl Sample for MissionAddRoleInput {
    fn sample() -> Self {
        Self {
            mission_id: "mission".into(),
            role_id: "role".into(),
            handle: Some("worker-2".into()),
            runtime: Some(Runtime::Codex),
            model: Some("model".into()),
            effort: Some("high".into()),
            speed: Some(CodexSpeed::Fast),
            task: Some("tiny task".into()),
        }
    }
}
impl Sample for MissionRoleChange {
    fn sample() -> Self {
        Self {
            mission_id: "mission".into(),
            handle: "worker-2".into(),
            session_id: "session".into(),
        }
    }
}

macro_rules! round_trip_api {
    ($( $name:ident($( $arg:ident: $client_ty:ty => $wire_ty:ty = $convert:block ),*) -> $out:ty [$fast:literal] => $handler:expr_2021; )*) => {
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
    rows,
    controller
});

impl Sample for TerminalController {
    fn sample() -> Self {
        Self::Desktop
    }
}
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

#[test]
fn remote_pin_request_redacts_debug_and_trust_contains_no_pin() {
    let pin = "001234567890";
    let request = Request::remote_set_pin {
        pin: crate::remote::Secret(pin.into()),
    };
    assert!(!format!("{request:?}").contains(pin));
    assert!(format!("{request:?}").contains("[redacted]"));
    let trust = crate::remote::RemoteTrust {
        pin_set: true,
        ..Default::default()
    };
    let response = Response::remote_set_pin(Ok(trust));
    assert!(!serde_json::to_string(&response).unwrap().contains(pin));
    assert!(Request::remote_trust {}.is_fast());
    assert!(!request.is_fast());
}

#[test]
fn an_interrupted_summary_decodes_on_builds_that_predate_the_phase() {
    #[derive(serde::Deserialize, Debug, PartialEq)]
    #[serde(rename_all = "snake_case")]
    enum OldPhase {
        NeedsYou,
        Failed,
        Working,
        Settled,
        Ended,
    }
    #[derive(serde::Deserialize)]
    struct OldSummary {
        #[serde(default)]
        phase: Option<OldPhase>,
    }
    let mut summary = MissionSummary::sample();
    summary.phase = Some(MissionPhase::Interrupted);
    let wire = serde_json::to_value(&summary).unwrap();
    assert_eq!(wire["phase"], "working");
    assert_eq!(wire["interrupted"], true);
    let old: OldSummary = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(old.phase, Some(OldPhase::Working));
    let new: MissionSummary = serde_json::from_value(wire).unwrap();
    assert_eq!(new.phase, Some(MissionPhase::Interrupted));

    summary.phase = Some(MissionPhase::Settled);
    let wire = serde_json::to_value(&summary).unwrap();
    assert!(wire.get("interrupted").is_none());
    let new: MissionSummary = serde_json::from_value(wire).unwrap();
    assert_eq!(new.phase, Some(MissionPhase::Settled));

    let mut wire = serde_json::to_value(&summary).unwrap();
    wire.as_object_mut().unwrap().remove("phase");
    let older_daemon: MissionSummary = serde_json::from_value(wire).unwrap();
    assert_eq!(older_daemon.phase, None);

    let future: MissionPhase = serde_json::from_value(serde_json::json!("paused")).unwrap();
    assert_eq!(future, MissionPhase::Unknown);
    let mut status = AgentStatus::sample();
    status.interrupted_since = None;
    assert!(serde_json::to_value(&status)
        .unwrap()
        .get("interrupted_since")
        .is_none());
}
impl Sample for files::FilesSubject {
    fn sample() -> Self {
        Self::Mission("mission 样例".into())
    }
}
impl Sample for files::FilesRootOutcome {
    fn sample() -> Self {
        Self::Root(files::FilesRoot {
            path: PathBuf::sample(),
            name: String::sample(),
            git: Some(files::GitCheckout {
                top: PathBuf::sample(),
                worktree: files::WorktreeKind::Linked,
                branch: Some("feat/634".into()),
                commit: None,
            }),
            cwd: Some("src/deep".into()),
            source: files::FilesRootSource::Saved,
        })
    }
}
impl Sample for files::DirEntry {
    fn sample() -> Self {
        Self {
            name: String::sample(),
            kind: files::EntryKind::Symlink,
            ignored: true,
            size: Some(7),
        }
    }
}
impl Sample for files::GitStatus {
    fn sample() -> Self {
        Self::new(
            vec![files::StatusEntry {
                path: "new 名.rs".into(),
                status: files::FileStatus::Renamed {
                    from: "old\nname.rs".into(),
                },
            }],
            vec!["target/".into()],
        )
    }
}
impl Sample for files::WorktreeEntry {
    fn sample() -> Self {
        Self {
            path: PathBuf::sample(),
            branch: None,
            commit: Some("0123456".into()),
            kind: files::WorktreeKind::Main,
            locked: true,
            prunable: true,
        }
    }
}
impl Sample for files::FileIndex {
    fn sample() -> Self {
        Self {
            paths: vec!["src/样例.rs".into()],
            truncated: true,
        }
    }
}
impl Sample for files::FileContent {
    fn sample() -> Self {
        Self::Text {
            bytes: crate::companion::TerminalBytes(vec![0, 255, 10]),
            size: 3,
            modified: Some(Sample::sample()),
        }
    }
}

impl Sample for RuntimeLaunchOptions {
    fn sample() -> Self {
        Self::default()
    }
}

variant!(RuntimeLaunchOption => Self::Bypass);
record!(RuntimeCommandPreview { lines });
record!(RuntimeCommandLine {
    command,
    note,
    option
});

variant!(RuntimeCommandNote => Self::ActiveAccount { name: "Work".into() });

#[test]
fn optional_catalog_account_preserves_older_requests() {
    let request = Request::skill_catalogs {
        account_id: Some("work".into()),
    };
    round_trip(request);
    let legacy: Request = serde_json::from_value(serde_json::json!({"skill_catalogs":{}})).unwrap();
    assert!(matches!(
        legacy,
        Request::skill_catalogs { account_id: None }
    ));
    round_trip(Request::usage_set_interval { seconds: 1800 });
}
variant!(RuntimeUpdateState => Self::Running);
record!(RuntimeUpdateTask {
    id,
    runtime,
    from_version,
    to_version,
    state,
    step,
    message,
    exit_code,
    installed_version,
    running_sessions
});
