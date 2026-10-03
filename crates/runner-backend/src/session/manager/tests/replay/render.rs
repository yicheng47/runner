use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Write;

const HEADER: &[&str] = &["runtime", "name", "rules", "bugs", "known_wrong"];
const STATE: &[&str] = &["status", "activity", "completion_armed", "quiescent", "key"];
const EVENTS: &[&str] = &["ms", "result", "published", "delivery_events"];
const OBSERVATION: &[&str] = &["activity", "source", "outcome", "detail", "interactions"];
const STATUS: &[&str] = &[
    "lifecycle",
    "observation",
    "exit_code",
    "error_since",
    "failed_since",
    "unread_since",
];

fn require_fields(value: &Value, fields: &[&str]) {
    assert!(value.is_object(), "expected an object: {value}");
    for field in fields {
        assert!(value.get(*field).is_some(), "missing {field}: {value}");
    }
}

fn validate_status(value: &Value) {
    require_fields(value, STATUS);
    let observation = &value["observation"];
    require_fields(observation, OBSERVATION);
    for field in ["activity", "source"] {
        assert!(
            observation[field].is_string(),
            "invalid {field}: {observation}"
        );
    }
    for field in ["outcome", "detail"] {
        assert!(
            observation[field].is_null() || observation[field].is_string(),
            "invalid {field}: {observation}"
        );
    }
    assert!(
        observation["interactions"].is_array(),
        "invalid interactions: {observation}"
    );
    assert!(value["lifecycle"].is_string(), "invalid lifecycle: {value}");
    for field in ["exit_code", "error_since", "failed_since", "unread_since"] {
        assert!(
            value[field].is_null() || value[field].as_i64().is_some(),
            "invalid {field}: {value}"
        );
    }
}

fn validate_row(value: &Value, published: bool) {
    require_fields(value, &["state", "source"]);
    assert!(
        value["state"].is_string() && value["source"].is_string(),
        "invalid row: {value}"
    );
    if published {
        require_fields(value, &["session_id", "status"]);
        assert!(
            value["session_id"].is_string(),
            "invalid session_id: {value}"
        );
    }
    if let Some(status) = value.get("status").filter(|status| !status.is_null()) {
        validate_status(status);
    }
}

fn validate(value: &Value) {
    require_fields(value, HEADER);
    require_fields(value, &["timeline", "session_status_rows"]);
    assert!(
        value["runtime"].is_string() && value["name"].is_string(),
        "invalid header: {value}"
    );
    for field in [
        "rules",
        "bugs",
        "known_wrong",
        "timeline",
        "session_status_rows",
    ] {
        assert!(value[field].is_array(), "invalid {field}: {value}");
    }
    for row in value["session_status_rows"].as_array().unwrap() {
        validate_row(row, false);
    }
    for step in value["timeline"].as_array().unwrap() {
        require_fields(step, STATE);
        require_fields(step, EVENTS);
        assert!(step["ms"].is_u64(), "invalid ms: {step}");
        assert!(
            step["completion_armed"].is_boolean() && step["quiescent"].is_boolean(),
            "invalid state flags: {step}"
        );
        for field in ["activity", "key"] {
            assert!(
                step[field].is_null() || step[field].is_string(),
                "invalid {field}: {step}"
            );
        }
        validate_status(&step["status"]);
        assert!(
            step["published"].is_array() && step["delivery_events"].is_array(),
            "invalid events: {step}"
        );
        for row in step["published"].as_array().unwrap() {
            validate_row(row, true);
        }
    }
}

fn atom(value: &Value) -> String {
    match value.as_str() {
        Some(text)
            if !text.is_empty()
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')) =>
        {
            text.to_owned()
        }
        _ => value.to_string(),
    }
}

fn extras(output: &mut String, value: &Value, known: &[&str], prefix: &str) {
    for (key, value) in value.as_object().unwrap() {
        if !known.contains(&key.as_str()) {
            write!(output, " {prefix}{key}={value}").unwrap();
        }
    }
}

fn status(value: &Value) -> String {
    if !value["observation"].is_object() {
        return value.to_string();
    }
    let observation = &value["observation"];
    let mut output = format!(
        "{}/{}/{}",
        atom(&value["lifecycle"]),
        atom(&observation["activity"]),
        atom(&observation["source"])
    );
    for (fields, keys) in [
        (observation, &["outcome", "detail", "interactions"][..]),
        (
            value,
            &["exit_code", "error_since", "failed_since", "unread_since"][..],
        ),
    ] {
        for key in keys {
            let field = &fields[*key];
            if !field.is_null() && !(*key == "interactions" && field.as_array().unwrap().is_empty())
            {
                write!(output, " {key}={field}").unwrap();
            }
        }
    }
    extras(&mut output, observation, OBSERVATION, "observation.");
    extras(&mut output, value, STATUS, "status.");
    output
}

fn row(value: &Value, published: bool) -> String {
    let mut output = format!(
        "{}@{} [{}]",
        atom(&value["state"]),
        atom(&value["source"]),
        value
            .get("status")
            .map(status)
            .unwrap_or_else(|| "<absent>".into())
    );
    if let Some(id) = value
        .get("session_id")
        .filter(|id| !published || *id != super::ID)
    {
        write!(output, " session_id={id}").unwrap();
    }
    extras(
        &mut output,
        value,
        &["state", "source", "status", "session_id"],
        "",
    );
    output
}

pub(super) fn render(value: &Value) -> String {
    validate(value);
    let mut output = HEADER
        .iter()
        .map(|key| format!("{key}={}", atom(&value[*key])))
        .collect::<Vec<_>>()
        .join(" ");
    extras(
        &mut output,
        value,
        &[
            "runtime",
            "name",
            "rules",
            "bugs",
            "known_wrong",
            "timeline",
            "session_status_rows",
        ],
        "",
    );
    output.push('\n');
    let timeline = value["timeline"].as_array().unwrap();
    let published = timeline
        .iter()
        .flat_map(|step| step["published"].as_array().unwrap())
        .map(|row| {
            let mut row = row.clone();
            if row.get("session_id").is_some_and(|id| id == super::ID) {
                row.as_object_mut().unwrap().remove("session_id");
            }
            row
        })
        .collect::<Vec<_>>();
    if value["session_status_rows"] == Value::Array(published) {
        output.push_str("session_status_rows=published\n");
    } else {
        output.push_str("session_status_rows:\n");
        for entry in value["session_status_rows"].as_array().unwrap() {
            writeln!(output, "  {}", row(entry, false)).unwrap();
        }
    }
    let mut previous = &Value::Null;
    for step in timeline {
        write!(output, "{}", step["ms"]).unwrap();
        let unknown = step
            .as_object()
            .unwrap()
            .keys()
            .chain(previous.as_object().into_iter().flat_map(|map| map.keys()))
            .map(String::as_str)
            .filter(|key| !STATE.contains(key) && !EVENTS.contains(key))
            .collect::<BTreeSet<_>>();
        for key in STATE.iter().copied().chain(unknown) {
            if previous.is_null() || step.get(key) != previous.get(key) {
                let field = match step.get(key) {
                    Some(value) if key == "status" => status(value),
                    Some(value) => value.to_string(),
                    None => "<absent>".into(),
                };
                write!(output, " {key}={field}").unwrap();
            }
        }
        if !step["result"].is_null() {
            write!(output, " result={}", step["result"]).unwrap();
        }
        if !step["delivery_events"].as_array().unwrap().is_empty() {
            write!(output, " delivery={}", step["delivery_events"]).unwrap();
        }
        output.push('\n');
        for entry in step["published"].as_array().unwrap() {
            writeln!(output, "  {}", row(entry, true)).unwrap();
        }
        previous = step;
    }
    output
}

#[test]
fn compact_deltas_preserve_resets_and_repeated_events() {
    let status = serde_json::json!({
        "lifecycle":"running", "observation": {
            "activity":"ready", "source":"hook", "outcome":null,
            "detail":null, "interactions":[]
        }, "exit_code":null, "error_since":null, "failed_since":null, "unread_since":null
    });
    let first = serde_json::json!({"ms":0,"status":status,"activity":"idle",
        "completion_armed":false,"quiescent":true,"key":"key",
        "result":false,"delivery_events":["InputCleared"],"published":[]});
    let mut second = first.clone();
    second["ms"] = 1.into();
    second["key"] = Value::Null;
    let value = serde_json::json!({"runtime":"test","name":"delta","rules":[],
        "bugs":[],"known_wrong":[],"timeline":[first,second],"session_status_rows":[]});
    assert_eq!(render(&value), concat!(
        "runtime=test name=delta rules=[] bugs=[] known_wrong=[]\n",
        "session_status_rows=published\n",
        "0 status=running/ready/hook activity=\"idle\" completion_armed=false quiescent=true key=\"key\" result=false delivery=[\"InputCleared\"]\n",
        "1 key=null result=false delivery=[\"InputCleared\"]\n"
    ));
}

#[test]
fn compact_unknown_fields_and_nonconstant_ids_are_retained() {
    let status = serde_json::json!({"lifecycle":"running","status_future":null,
        "observation":{"activity":"ready","source":"hook","outcome":null,
            "detail":"two\nlines", "interactions":[], "future":{"flag":true}},
        "exit_code":null,"error_since":null,"failed_since":null,"unread_since":null});
    let entry = serde_json::json!({"state":"idle","source":"hook","status":status,
        "session_id":"other-session","row_future":[]});
    let first = serde_json::json!({"ms":0,"status":status,"activity":null,
        "completion_armed":false,"quiescent":true,"key":null,"step_future":null,
        "result":null,"delivery_events":[],"published":[entry]});
    let mut second = first.clone();
    second["ms"] = 1.into();
    second.as_object_mut().unwrap().remove("step_future");
    second["published"] = serde_json::json!([]);
    let value = serde_json::json!({"runtime":"test","name":"unknown","rules":[],
        "bugs":[],"known_wrong":[],"header_future":{"value":0},
        "timeline":[first,second],"session_status_rows":[]});
    let text = render(&value);
    for retained in [
        "header_future={\"value\":0}",
        "observation.future={\"flag\":true}",
        "status.status_future=null",
        "detail=\"two\\nlines\"",
        "step_future=null",
        "1 step_future=<absent>",
        "session_id=\"other-session\"",
        "row_future=[]",
    ] {
        assert!(text.contains(retained), "missing {retained}: {text}");
    }
    assert!(text.contains("session_status_rows:\n"));
}

#[test]
fn compact_mission_only_rows_distinguish_missing_and_null_status() {
    let value = serde_json::json!({"runtime":"test","name":"wake","rules":[],
    "bugs":[],"known_wrong":[],"timeline":[],"session_status_rows":[
        {"state":"busy","source":"wake","future":null},
        {"state":"busy","source":"wake","status":null}
    ]});
    assert_eq!(
        render(&value),
        concat!(
            "runtime=test name=wake rules=[] bugs=[] known_wrong=[]\n",
            "session_status_rows:\n",
            "  busy@wake [<absent>] future=null\n",
            "  busy@wake [null]\n"
        )
    );
}

#[test]
fn compact_omitted_defaults_reject_missing_or_mistyped_fields() {
    let status = serde_json::json!({"lifecycle":"running","observation":{
        "activity":"idle","source":"hook","outcome":null,"detail":null,"interactions":[]},
        "exit_code":null,"error_since":null,"failed_since":null,"unread_since":null});
    let published =
        serde_json::json!({"session_id":super::ID,"state":"idle","source":"hook","status":status});
    let mission = serde_json::json!({"state":"idle","source":"hook","status":status});
    let value = serde_json::json!({"runtime":"test","name":"shape","rules":[],"bugs":[],
        "known_wrong":[],"timeline":[{"ms":0,"status":status,"activity":null,
            "completion_armed":false,"quiescent":true,"key":null,"result":null,
            "published":[published],"delivery_events":[]}],"session_status_rows":[mission]});
    let original = render(&value);
    for (parent, fields) in [
        ("", HEADER),
        ("/timeline/0", STATE),
        ("/timeline/0", EVENTS),
        ("/timeline/0/status", STATUS),
        ("/timeline/0/status/observation", OBSERVATION),
        ("/timeline/0/published/0", &["session_id", "status"][..]),
        ("/timeline/0/published/0/status", STATUS),
        ("/timeline/0/published/0/status/observation", OBSERVATION),
        ("/session_status_rows/0/status", STATUS),
        ("/session_status_rows/0/status/observation", OBSERVATION),
    ] {
        for field in fields {
            let mut changed = value.clone();
            changed
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(*field);
            assert!(
                std::panic::catch_unwind(|| render(&changed)).is_err(),
                "missing {parent}/{field} was accepted"
            );
        }
    }
    for (path, replacement) in [
        ("/timeline/0/status/observation/interactions", Value::Null),
        ("/timeline/0/published/0/session_id", Value::Null),
        ("/timeline/0/status/exit_code", serde_json::json!([])),
        ("/timeline/0/status/error_since", serde_json::json!([])),
        ("/timeline/0/status/failed_since", serde_json::json!([])),
        ("/timeline/0/status/unread_since", serde_json::json!([])),
        (
            "/timeline/0/status/observation/outcome",
            serde_json::json!([]),
        ),
        (
            "/timeline/0/status/observation/detail",
            serde_json::json!([]),
        ),
        ("/timeline/0/result", serde_json::json!([])),
        ("/timeline/0/delivery_events", serde_json::json!([null])),
    ] {
        let mut changed = value.clone();
        *changed.pointer_mut(path).unwrap() = replacement;
        if let Ok(text) = std::panic::catch_unwind(|| render(&changed)) {
            assert_ne!(text, original, "mutated {path} rendered like its default");
        }
    }
    let mut changed = value.clone();
    changed["session_status_rows"][0]["session_id"] = super::ID.into();
    assert_ne!(render(&changed), original);
    assert!(render(&changed).contains("session_id=\"scenario-session\""));
}
