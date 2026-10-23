use std::collections::BTreeSet;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

use super::*;
use crate::protocol::{
    Activity, AgentObservation, AgentStatus, HumanInteraction, Lifecycle, WaitReason,
};

const CLIENT_FIXTURES: &[&str] = &[
    "hello",
    "layout_split",
    "start_chat_role",
    "catalog",
    "folders",
    "start_chat",
    "start_shell",
    "stop",
    "resume",
    "restart",
    "fork",
    "close",
    "rename",
    "archive",
    "layout",
    "pin",
    "move",
    "reorder_pinned",
    "create_project",
    "delete_project",
    "start_mission",
    "stop_mission",
    "resume_mission",
    "overview_request",
    "feed_request",
    "follow",
    "unfollow",
    "answer",
    "post",
    "post_everyone",
    "attach",
    "attach_if_free",
    "input",
    "take_control",
    "viewport",
    "detach",
    "v1_without_later_fields",
    "unknown_request",
    "unknown_control",
];
const HOST_FIXTURES: &[&str] = &[
    "welcome",
    "result_catalog",
    "result_folders",
    "result_session",
    "result_mission",
    "result_resume_mission",
    "result_layout",
    "result_project",
    "result_delete_project",
    "result_empty",
    "result_overview",
    "result_feed",
    "result_feed_start",
    "error_not_found",
    "error_bad_request",
    "error_bad_request_body",
    "error_unknown_request",
    "error_not_running",
    "error_failed",
    "unsupported_version",
    "overview_push",
    "overview_without_sidebar",
    "unknown_sidebar_kind",
    "status",
    "feed_appended",
    "feed_entry_too_large",
    "snapshot",
    "snapshot_final",
    "snapshot_rep",
    "snapshot_rep_unfinished",
    "output",
    "resized",
    "resized_desktop",
    "resized_browser",
    "resized_none",
    "closed",
    "closed_stopped",
    "closed_archived",
    "closed_unknown",
    "unknown_status",
    "unknown_reason",
    "unknown_push",
    "unknown_push_string_id",
    "unknown_controller",
    "unknown_closed_reason",
    "unknown_error_code",
    "unknown_overview_enums",
    "unknown_pane",
    "unknown_feed_kind",
];

const CLIENT_COMPAT_FIXTURES: &[(&str, &str)] = &[
    ("unknown_field", "hello"),
    ("unknown_field_start_chat", "start_chat"),
    ("start_chat_minimal", "start_chat"),
];
const HOST_COMPAT_FIXTURES: &[(&str, &str)] = &[
    ("unknown_field_status_id", "status"),
    ("unknown_fields_resized_you", "resized"),
    ("unknown_fields_resized_desktop", "resized_desktop"),
    ("unknown_fields_resized_browser", "resized_browser"),
    ("unknown_fields_resized_none", "resized_none"),
];

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/companion/fixtures")
}

fn fixture(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(fixtures_dir().join(format!("{name}.json"))).unwrap())
        .unwrap()
}

fn decode<T: DeserializeOwned>(name: &str) -> T {
    serde_json::from_value(fixture(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn round_trip<T: DeserializeOwned + Serialize>(name: &str) {
    let value: T = decode(name);
    assert_eq!(
        serde_json::to_value(value).unwrap(),
        fixture(name),
        "{name}"
    );
}

#[test]
fn overview_sidebar_nodes_are_optional_and_preserve_order_metadata() {
    let HostMessage::Overview(old) = decode("overview_without_sidebar") else {
        panic!("overview expected")
    };
    assert!(old.sidebar.is_none());
    let current: Overview =
        serde_json::from_value(fixture("result_overview")["result"].clone()).unwrap();
    let nodes = current.sidebar.unwrap();
    assert_eq!(nodes.len(), 3);
    assert_eq!(nodes[0].kind, SidebarNodeKind::Project);
    assert_eq!(
        nodes[1].parent.as_deref(),
        Some("01KZ0000000000000000000000")
    );
    assert_eq!(nodes[1].position, 1);
    assert_eq!(nodes[2].pinned_position, Some(0));
    let HostMessage::Overview(future) = decode("unknown_sidebar_kind") else {
        panic!("overview expected")
    };
    assert_eq!(
        future.sidebar.unwrap()[3].kind,
        SidebarNodeKind::Unrecognized("futureKind".into())
    );
}

#[test]
fn golden_messages_round_trip_and_compatibility_files_decode_to_known_fields() {
    for name in CLIENT_FIXTURES {
        round_trip::<ClientMessage>(name);
    }
    for name in HOST_FIXTURES {
        round_trip::<HostMessage>(name);
    }
    for (name, known) in CLIENT_COMPAT_FIXTURES {
        assert_eq!(
            decode::<ClientMessage>(name),
            decode::<ClientMessage>(known),
            "{name}"
        );
        assert_eq!(
            serde_json::to_value(decode::<ClientMessage>(name)).unwrap(),
            fixture(known)
        );
    }
    for (name, known) in HOST_COMPAT_FIXTURES {
        assert_eq!(
            decode::<HostMessage>(name),
            decode::<HostMessage>(known),
            "{name}"
        );
        assert_eq!(
            serde_json::to_value(decode::<HostMessage>(name)).unwrap(),
            fixture(known)
        );
    }
    let listed: BTreeSet<_> = CLIENT_FIXTURES
        .iter()
        .copied()
        .chain(HOST_FIXTURES.iter().copied())
        .chain(
            CLIENT_COMPAT_FIXTURES
                .iter()
                .chain(HOST_COMPAT_FIXTURES)
                .map(|(name, _)| *name),
        )
        .map(|s| format!("{s}.json"))
        .collect();
    let present: BTreeSet<_> = std::fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(listed, present, "Every golden must be tested.");
}

#[test]
fn client_bytes_decoder_matches_serde_on_all_golden_requests() {
    for name in CLIENT_FIXTURES
        .iter()
        .copied()
        .chain(CLIENT_COMPAT_FIXTURES.iter().map(|(name, _)| *name))
    {
        let bytes = serde_json::to_vec(&fixture(name)).unwrap();
        let message = ClientMessage::decode(&bytes).unwrap();
        assert_eq!(message, decode::<ClientMessage>(name), "{name}");
    }
}

#[test]
fn malformed_request_decode_preserves_a_readable_id_in_its_error() {
    let bytes = br#"{"type":"attach","id":7,"session":"s"}"#;
    let error = ClientMessage::decode(bytes).unwrap_err();
    assert_eq!(error.id, Some(7));
    assert_eq!(error.code, ErrorCode::BadRequest);
    assert!(serde_json::from_slice::<ClientMessage>(bytes)
        .unwrap_err()
        .to_string()
        .contains(&error.message));
    assert_eq!(
        serde_json::to_value(HostMessage::Error(*error)).unwrap(),
        fixture("error_bad_request_body")
    );

    for value in [json!({"id":7}), json!({"type":false,"id":7})] {
        let error = ClientMessage::decode(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(error.id, Some(7));
        assert_eq!(error.code, ErrorCode::BadRequest);
    }
}

#[test]
fn bad_request_messages_identify_missing_fields_in_each_request_body() {
    for (fixture_name, field) in [
        ("hello", "versions"),
        ("feed_request", "limit"),
        ("follow", "mission"),
        ("unfollow", "mission"),
        ("answer", "question"),
        ("post", "to"),
        ("attach", "cols"),
        ("input", "data"),
        ("take_control", "rows"),
        ("viewport", "session"),
        ("detach", "session"),
    ] {
        let mut value = fixture(fixture_name);
        value.as_object_mut().unwrap().remove(field);
        let error = ClientMessage::decode(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(error.code, ErrorCode::BadRequest);
        assert_eq!(
            error.message,
            format!(
                "{}: missing field {field}.",
                value["type"].as_str().unwrap()
            )
        );
    }
}

#[test]
fn bad_request_messages_explain_mistyped_fields_without_echoing_client_values() {
    let private = "private terminal data".repeat(10_000);
    for (fixture_name, field, value, expected) in [
        (
            "hello",
            "versions",
            json!([private]),
            "an array of unsigned 32-bit integers",
        ),
        (
            "attach",
            "cols",
            json!(private),
            "an unsigned 16-bit integer",
        ),
        (
            "attach",
            "history",
            json!(-1),
            "an unsigned 32-bit integer or null",
        ),
        (
            "feed_request",
            "before",
            json!({"secret":private}),
            "a string or null",
        ),
        ("post", "to", json!([false]), "an array of strings"),
        ("answer", "text", json!([private]), "a string"),
        ("input", "data", json!(private), "standard padded base64"),
    ] {
        let mut body = fixture(fixture_name);
        body[field] = value;
        let error = ClientMessage::decode(&serde_json::to_vec(&body).unwrap()).unwrap_err();
        assert_eq!(error.code, ErrorCode::BadRequest);
        assert_eq!(error.id, body.get("id").and_then(Value::as_u64));
        assert_eq!(
            error.message,
            format!(
                "{}: field {field} must be {expected}.",
                body["type"].as_str().unwrap()
            )
        );
        assert!(error.message.len() <= 120);
        assert!(!error.message.contains("private"));
    }
}

#[test]
fn malformed_request_without_a_readable_id_has_an_uncorrelated_error() {
    for bytes in [
        &br#"{"type":"attach","session":"s"}"#[..],
        &br#"{"type":"attach","id":"not-a-number"}"#[..],
        &br#"["overview",1]"#[..],
        &b"null"[..],
        &b"{"[..],
    ] {
        let error = ClientMessage::decode(bytes).unwrap_err();
        assert_eq!(error.id, None);
        assert_eq!(error.code, ErrorCode::BadRequest);
    }
}

#[test]
fn push_dispatch_ignores_unknown_id_fields_while_replies_validate_theirs() {
    let HostMessage::Unknown(push) = decode("unknown_push_string_id") else {
        panic!()
    };
    assert_eq!(push.push_type, "notice");
    assert_eq!(push.extensions["id"], json!("01ME9000000000000000000000"));
    let HostMessage::Status(push) = decode("unknown_field_status_id") else {
        panic!()
    };
    assert_eq!(push.status.status, Status::NeedsYou);
    for value in [
        json!({"type":"result","id":"not-a-number","result":null}),
        json!({"type":"error","id":"not-a-number","code":"failed","message":"Failed."}),
    ] {
        assert!(serde_json::from_value::<HostMessage>(value).is_err());
    }
}

#[test]
fn session_cwd_is_nullable_and_defaults_when_missing() {
    let value = fixture("overview_push")["sessions"]["01M90000000000000000000000"].clone();
    let session: Session = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(session.cwd, None);
    let mut missing = value.clone();
    missing.as_object_mut().unwrap().remove("cwd");
    let session: Session = serde_json::from_value(missing).unwrap();
    assert_eq!(session.cwd, None);
    assert_eq!(serde_json::to_value(session).unwrap(), value);
    let HostMessage::Overview(overview) = decode("overview_push") else {
        panic!()
    };
    assert_eq!(
        overview.sessions["01M60000000000000000000000"]
            .cwd
            .as_deref(),
        Some("/Users/me/runner")
    );
}

#[test]
fn result_payloads_decode_as_the_request_specific_types() {
    let HostMessage::Result(result) = decode("result_overview") else {
        panic!()
    };
    let overview: Overview = serde_json::from_value(result.result.clone()).unwrap();
    assert_eq!(serde_json::to_value(&overview).unwrap(), result.result);
    assert!(matches!(overview.tabs[0].panes, Pane::Split { .. }));
    assert_eq!(overview.missions[0].asks[1].choices, None);
    for name in ["result_feed", "result_feed_start"] {
        let HostMessage::Result(result) = decode(name) else {
            panic!()
        };
        let page: FeedPage = serde_json::from_value(result.result.clone()).unwrap();
        assert_eq!(serde_json::to_value(page).unwrap(), result.result);
    }
    let HostMessage::FeedAppended(feed) = decode("feed_appended") else {
        panic!()
    };
    assert!(matches!(
        feed.entries[0].content,
        FeedContent::Message { .. }
    ));
    assert!(matches!(feed.entries[1].content, FeedContent::Ask { .. }));
    assert!(matches!(
        feed.entries[3].content,
        FeedContent::Answer { .. }
    ));
    assert!(matches!(
        feed.entries[4].content,
        FeedContent::Signal { .. }
    ));
}

#[test]
fn unknown_fields_do_not_change_the_known_fields() {
    let ClientMessage::Hello(hello) = decode("unknown_field") else {
        panic!()
    };
    let ClientMessage::Hello(known) = decode("hello") else {
        panic!()
    };
    assert_eq!(hello, known);
}

#[test]
fn an_older_v1_message_defaults_a_later_field() {
    // This test-only next revision proves the addition rule without inventing a shipped v1 field.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FutureHello {
        #[serde(flatten)]
        hello: Hello,
        #[serde(default)]
        future_capability: bool,
    }
    let future: FutureHello = decode("v1_without_later_fields");
    assert_eq!(future.hello.versions, [VERSION]);
    assert!(!future.future_capability);
}

#[test]
fn unknown_requests_keep_the_id_and_produce_the_golden_error() {
    let ClientMessage::Unknown(request) = decode("unknown_request") else {
        panic!()
    };
    assert_eq!(request.id, 24);
    assert_eq!(request.request_type, "futureRequest");
    assert_eq!(
        serde_json::to_value(request.response()).unwrap(),
        fixture("error_unknown_request")
    );
}

#[test]
fn malformed_known_messages_do_not_become_unknown() {
    for value in [
        json!({"type":"attach","id":1}),
        json!({"type":"input","id":2,"session":"s","data":"not base64"}),
        json!({"type":"futureRequest"}),
        json!({"type":"futureRequest","id":"12"}),
        json!({"type":"feed","id":12,"mission":"m","limit":-1}),
        json!([]),
        json!(["overview", 1]),
        json!(["hello", null]),
    ] {
        assert!(
            serde_json::from_value::<ClientMessage>(value.clone()).is_err(),
            "{value}"
        );
    }
    assert!(serde_json::from_value::<HostMessage>(json!({"type":"snapshot"})).is_err());
    for value in [json!(["status", null]), json!(["welcome", null])] {
        assert!(
            serde_json::from_value::<HostMessage>(value.clone()).is_err(),
            "{value}"
        );
    }
}

#[test]
fn known_controllers_ignore_unknown_fields() {
    for (name, known) in &HOST_COMPAT_FIXTURES[1..] {
        let HostMessage::Resized(resized) = decode(name) else {
            panic!()
        };
        assert!(!matches!(resized.controller, Controller::Unknown(_)));
        assert_eq!(resized.controller.effective(), resized.controller);
        assert_eq!(
            serde_json::to_value(HostMessage::Resized(resized)).unwrap(),
            fixture(known)
        );
    }
}

#[test]
fn unknown_push_and_each_enum_use_the_documented_fallback() {
    assert!(matches!(
        decode::<HostMessage>("unknown_push"),
        HostMessage::Unknown(_)
    ));
    let HostMessage::Status(status) = decode("unknown_status") else {
        panic!()
    };
    assert!(matches!(status.status.status, Status::Unrecognized(_)));
    assert_eq!(status.status.status.effective(), Status::Idle);
    let HostMessage::Status(status) = decode("unknown_reason") else {
        panic!()
    };
    assert_eq!(
        status.status.reason.unwrap().effective(),
        NeedsYouReason::Unknown
    );
    let ClientMessage::Attach(attach) = decode("unknown_control") else {
        panic!()
    };
    assert_eq!(attach.control.effective(), ControlMode::IfFree);
    let HostMessage::Resized(resized) = decode("unknown_controller") else {
        panic!()
    };
    assert_eq!(resized.controller.effective(), Controller::None);
    let HostMessage::Closed(closed) = decode("unknown_closed_reason") else {
        panic!()
    };
    assert_eq!(closed.reason.effective(), ClosedReason::Unknown);
    let HostMessage::Error(error) = decode("unknown_error_code") else {
        panic!()
    };
    assert_eq!(error.code.effective(), ErrorCode::Failed);
    assert_eq!(error.message, "A future host error.");
    let HostMessage::Overview(overview) = decode("unknown_overview_enums") else {
        panic!()
    };
    assert_eq!(
        overview.missions[0].state.effective(),
        MissionState::Stopped
    );
    assert_eq!(
        overview.sessions["01M60000000000000000000000"]
            .kind
            .effective(),
        SessionKind::Chat
    );
    let Pane::Split { split, .. } = &overview.tabs[0].panes else {
        panic!()
    };
    assert_eq!(split.effective(), SplitDirection::Row);
    let HostMessage::Overview(overview) = decode("unknown_pane") else {
        panic!()
    };
    assert!(matches!(overview.tabs[0].panes, Pane::Unknown(_)));
    assert!(matches!(
        overview.tabs[0].panes.effective(),
        Pane::Leaf { session: None, .. }
    ));
    let HostMessage::FeedAppended(feed) = decode("unknown_feed_kind") else {
        panic!()
    };
    assert!(matches!(feed.entries[0].content, FeedContent::Unknown(_)));
}

#[test]
fn highest_common_version_is_selected_independent_of_offer_order() {
    let host = VersionRange { min: 1, max: 3 };
    assert_eq!(host.negotiate(&[2, 3, 4]).unwrap(), 3);
    assert_eq!(host.negotiate(&[3, 1, 2]).unwrap(), 3);
    assert_eq!(host.negotiate(&[0, 1, 5, 1]).unwrap(), 1);
    for offered in [&[0, 4][..], &[]] {
        let error = host.negotiate(offered).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedVersion);
        assert_eq!(error.id, None);
        assert_eq!(error.versions, Some(host));
    }
    let mismatch = VersionRange {
        min: 1,
        max: VERSION,
    }
    .negotiate(&[2])
    .unwrap_err();
    assert_eq!(
        serde_json::to_value(HostMessage::Error(*mismatch)).unwrap(),
        fixture("unsupported_version")
    );
}

#[test]
fn terminal_chunk_boundaries_and_base64_are_stable() {
    assert_eq!(MAX_MESSAGE_SIZE, 1_048_576);
    assert_eq!(TERMINAL_CHUNK_SIZE, 32_768);
    assert_eq!(terminal_chunks(&[]).count(), 0);
    let exact = vec![0xff; TERMINAL_CHUNK_SIZE];
    assert_eq!(terminal_chunks(&exact).collect::<Vec<_>>(), [&exact[..]]);
    let over = vec![0xff; TERMINAL_CHUNK_SIZE + 1];
    let chunks: Vec<_> = terminal_chunks(&over).collect();
    assert_eq!(
        chunks.iter().map(|chunk| chunk.len()).collect::<Vec<_>>(),
        [TERMINAL_CHUNK_SIZE, 1]
    );
    assert_eq!(chunks.concat(), over);
    let bytes = TerminalBytes(vec![0, 255, 251, 10]);
    assert_eq!(serde_json::to_value(&bytes).unwrap(), json!("AP/7Cg=="));
    assert_eq!(
        serde_json::from_value::<TerminalBytes>(json!("AP/7Cg==")).unwrap(),
        bytes
    );
    for bad in ["AP_7Cg==", "AP/7Cg", "!"] {
        assert!(serde_json::from_value::<TerminalBytes>(json!(bad)).is_err());
    }
}

#[test]
fn terminal_history_and_viewport_defaults() {
    let mut missing_history = fixture("attach_if_free");
    missing_history.as_object_mut().unwrap().remove("history");
    let ClientMessage::Attach(older) = serde_json::from_value(missing_history).unwrap() else {
        panic!()
    };
    assert_eq!(older.history_lines(), 1_000);
    let ClientMessage::Attach(attach) = decode("attach_if_free") else {
        panic!()
    };
    assert_eq!(attach.history_lines(), 1_000);
    let ClientMessage::Attach(attach) = decode("attach") else {
        panic!()
    };
    assert_eq!(attach.history_lines(), 100);
    assert_eq!(attach.clamped_size(), (100, 30));
    assert_eq!(
        AttachRequest {
            cols: 1,
            rows: 1,
            ..attach
        }
        .clamped_size(),
        (20, 8)
    );
    let request = ViewportRequest {
        id: 1,
        session: "s".into(),
        cols: 1,
        rows: 1,
    };
    assert_eq!(request.clamped_size(), (20, 8));
    assert_eq!(
        ViewportRequest {
            cols: 100,
            rows: 30,
            ..request
        }
        .clamped_size(),
        (100, 30)
    );
}

fn agent(lifecycle: Lifecycle, activity: Activity, reason: Option<WaitReason>) -> AgentStatus {
    AgentStatus {
        lifecycle,
        observation: AgentObservation {
            activity,
            interactions: reason
                .into_iter()
                .map(|reason| HumanInteraction {
                    id: "q".into(),
                    reason,
                    owners: vec![],
                    since: 1,
                })
                .collect(),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn status_needs_you_precedes_activity_and_preserves_reason() {
    for (reason, expected) in [
        (WaitReason::Approval, NeedsYouReason::Approval),
        (WaitReason::Answer, NeedsYouReason::Answer),
        (WaitReason::Unknown, NeedsYouReason::Unknown),
    ] {
        let status =
            SessionStatus::from(&agent(Lifecycle::Running, Activity::Working, Some(reason)));
        assert_eq!(status.status, Status::NeedsYou);
        assert_eq!(status.reason, Some(expected));
    }
}

#[test]
fn status_working() {
    let status = SessionStatus::from(&agent(Lifecycle::Running, Activity::Working, None));
    assert_eq!(status.status, Status::Working);
    assert_eq!(status.reason, None);
}

#[test]
fn status_idle_including_shell_and_unavailable_observations() {
    for activity in [Activity::Idle, Activity::Ready, Activity::Unavailable] {
        let status = SessionStatus::from(&agent(Lifecycle::Running, activity, None));
        assert_eq!(status.status, Status::Idle);
        assert_eq!(status.reason, None);
    }
}

#[test]
fn status_starting_and_resuming_precede_interactions() {
    for lifecycle in [Lifecycle::Starting, Lifecycle::Resuming] {
        let status = SessionStatus::from(&agent(
            lifecycle,
            Activity::Working,
            Some(WaitReason::Approval),
        ));
        assert_eq!(status.status, Status::Starting);
        assert_eq!(status.reason, None);
    }
}

#[test]
fn status_stopped_precedes_stale_interactions() {
    let status = SessionStatus::from(&agent(
        Lifecycle::Stopped,
        Activity::Working,
        Some(WaitReason::Approval),
    ));
    assert_eq!(status.status, Status::Stopped);
    assert_eq!(status.reason, None);
}

#[test]
fn status_error_precedes_stale_interactions() {
    let status = SessionStatus::from(&agent(
        Lifecycle::Error,
        Activity::Working,
        Some(WaitReason::Approval),
    ));
    assert_eq!(status.status, Status::Error);
    assert_eq!(status.reason, None);
}

#[test]
fn status_unread_is_independent_of_the_flattened_status() {
    let mut value = agent(Lifecycle::Running, Activity::Idle, None);
    assert!(!SessionStatus::from(&value).unread);
    value.unread_since = Some(1);
    assert!(SessionStatus::from(&value).unread);
}

#[test]
fn browser_sizes_clamp_both_axes_to_the_approved_bounds() {
    for (cols, rows, expected) in [
        (0, 0, (20, 8)),
        (80, 24, (80, 24)),
        (u16::MAX, u16::MAX, (512, 256)),
        (10, u16::MAX, (20, 256)),
        (u16::MAX, 1, (512, 8)),
    ] {
        let attach = AttachRequest {
            id: 1,
            session: "test".into(),
            cols,
            rows,
            control: ControlMode::Take,
            history: None,
        };
        let viewport = ViewportRequest {
            id: 2,
            session: "test".into(),
            cols,
            rows,
        };
        assert_eq!(attach.clamped_size(), expected);
        assert_eq!(viewport.clamped_size(), expected);
    }
}

#[test]
fn snapshot_continuation_defaults_and_validation() {
    let mut value = fixture("snapshot_rep_unfinished");
    let snapshot: Snapshot = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(snapshot.preceding_char, Some('c'));
    assert_eq!(snapshot.unfinished_len, 3);
    value.as_object_mut().unwrap().remove("precedingChar");
    value.as_object_mut().unwrap().remove("unfinishedLen");
    let snapshot: Snapshot = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(snapshot.preceding_char, None);
    assert_eq!(snapshot.unfinished_len, 0);
    value["precedingChar"] = json!("two");
    assert!(serde_json::from_value::<Snapshot>(value.clone()).is_err());
    value["precedingChar"] = json!(null);
    value["unfinishedLen"] = json!(-1);
    assert!(serde_json::from_value::<Snapshot>(value).is_err());
}

#[test]
fn action_fields_are_required_when_load_bearing_and_debug_keeps_bodies_private() {
    for name in [
        "start_chat",
        "start_shell",
        "resume",
        "restart",
        "fork",
        "start_mission",
        "resume_mission",
    ] {
        for field in ["cols", "rows"] {
            let mut value = fixture(name);
            value.as_object_mut().unwrap().remove(field);
            let error = ClientMessage::decode(&serde_json::to_vec(&value).unwrap()).unwrap_err();
            assert_eq!(error.id, Some(42));
            assert_eq!(error.code, ErrorCode::BadRequest);
            assert!(error.message.contains(&format!("missing field {field}.")));
        }
    }
    for value in [
        json!({"type":"post","id":7,"mission":"m","to":[],"text":"private-body-canary"}),
        json!({"type":"futureSettings","id":8,"credential":"private-body-canary"}),
    ] {
        let message = ClientMessage::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(!format!("{message:?}").contains("private-body-canary"));
    }
    let result = HostMessage::Result(ResultMessage {
        id: 7,
        result: json!({"text":"private-body-canary"}),
    });
    assert!(!format!("{result:?}").contains("private-body-canary"));
    let response = crate::protocol::Response::remote_request(Ok(crate::remote::RemoteResult {
        result: json!({"text":"private-body-canary"}),
        features: vec![],
    }));
    assert!(!format!("{response:?}").contains("private-body-canary"));
}

#[test]
fn catalog_and_folder_results_have_their_own_wire_types() {
    let catalog: Catalog =
        serde_json::from_value(fixture("result_catalog")["result"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(catalog).unwrap(),
        fixture("result_catalog")["result"]
    );
    let folders: FolderPage =
        serde_json::from_value(fixture("result_folders")["result"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(folders).unwrap(),
        fixture("result_folders")["result"]
    );
    for name in CLIENT_FIXTURES {
        let value = fixture(name);
        if ActionRequest::NAMES.contains(&value["type"].as_str().unwrap()) {
            let mut future = value.clone();
            future["futureField"] = json!({"new":true});
            assert_eq!(
                serde_json::to_value(
                    ClientMessage::decode(&serde_json::to_vec(&future).unwrap()).unwrap()
                )
                .unwrap(),
                value
            );
        }
    }
    let mut invalid = fixture("layout_split");
    invalid["panes"]["sizes"] = json!([1]);
    assert!(ClientMessage::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
}
