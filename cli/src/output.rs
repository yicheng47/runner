use runner_cli::client::ToolResponse;
use serde_json::Value;

const MAX_TABLE_CELL_WIDTH: usize = 48;
const MAX_KEY_VALUE_WIDTH: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Generic,
    Status,
    ProjectList,
    Project,
    RoleList,
    Role,
    CrewList,
    Crew,
    CrewShow,
    MissionList,
    Mission,
    MissionShow,
    MissionFeed,
    SessionList,
    SessionShow,
    Confirmation(&'static str),
}

pub fn print(response: &ToolResponse, json: bool, quiet: bool, view: View) {
    if let Some(lines) = machine_lines(response, json, quiet) {
        for line in lines {
            println!("{line}");
        }
        return;
    }
    for line in render_default(view, &response.value) {
        println!("{line}");
    }
}

fn machine_lines(response: &ToolResponse, json: bool, quiet: bool) -> Option<Vec<String>> {
    if json {
        Some(vec![response.raw_json.clone()])
    } else if quiet {
        Some(ids(&response.value))
    } else {
        None
    }
}

fn ids(value: &Value) -> Vec<String> {
    match value {
        Value::Array(values) => values.iter().flat_map(ids).collect(),
        Value::Object(map) => {
            for key in ["id", "session_id", "mission_id", "slot_id"] {
                if let Some(id) = map.get(key).and_then(Value::as_str) {
                    return vec![id.to_owned()];
                }
            }
            if let Some(value) = map.get("mission") {
                return ids(value);
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn render_default(view: View, value: &Value) -> Vec<String> {
    match view {
        View::Generic => render_generic(value),
        View::Status => render_status(value),
        View::ProjectList => render_project_list(value),
        View::Project => render_project(value),
        View::RoleList => render_role_list(value),
        View::Role => render_role(value),
        View::CrewList => render_crew_list(value),
        View::Crew => render_crew(value),
        View::CrewShow => render_crew_show(value),
        View::MissionList => render_mission_list(value),
        View::Mission => render_mission(value),
        View::MissionShow => render_mission_show(value),
        View::MissionFeed => render_mission_feed(value),
        View::SessionList => render_session_list(value),
        View::SessionShow => render_session_show(value),
        View::Confirmation(action) => render_confirmation(action, value),
    }
}

fn render_project_list(value: &Value) -> Vec<String> {
    let rows = array(value)
        .iter()
        .map(|row| {
            vec![
                cell(row.get("name")),
                cell(row.get("id")),
                cell(row.get("cwd")),
            ]
        })
        .collect();
    table(&["NAME", "ID", "PATH"], rows)
}

fn render_project(value: &Value) -> Vec<String> {
    key_values(&[
        ("NAME", value.get("name")),
        ("ID", value.get("id")),
        ("PATH", value.get("cwd")),
    ])
}

fn render_role_list(value: &Value) -> Vec<String> {
    let rows = array(value)
        .iter()
        .map(|row| {
            vec![
                cell(row.get("handle")),
                cell(row.get("display_name")),
                cell(row.get("runtime")),
                cell(row.get("model")),
                cell(row.get("effort")),
                cell(row.get("id")),
            ]
        })
        .collect();
    table(
        &["HANDLE", "NAME", "RUNTIME", "MODEL", "EFFORT", "ID"],
        rows,
    )
}

fn render_role(value: &Value) -> Vec<String> {
    let mut lines = key_values(&[
        ("HANDLE", value.get("handle")),
        ("NAME", value.get("display_name")),
        ("RUNTIME", value.get("runtime")),
        ("MODEL", value.get("model")),
        ("EFFORT", value.get("effort")),
        ("ID", value.get("id")),
    ]);
    lines.push(String::new());
    lines.push("PROMPT".to_owned());
    match value.get("system_prompt").and_then(Value::as_str) {
        Some(prompt) if !prompt.is_empty() => lines.extend(prompt.lines().map(str::to_owned)),
        _ => lines.push("-".to_owned()),
    }
    lines
}

fn render_crew_list(value: &Value) -> Vec<String> {
    let rows = array(value)
        .iter()
        .map(|row| {
            let slots = row
                .get("members")
                .and_then(Value::as_array)
                .map(|members| {
                    members
                        .iter()
                        .map(|member| {
                            let handle = text(member.get("slot_handle"));
                            let lead = member.get("lead").and_then(Value::as_bool) == Some(true);
                            let runtime = text(member.get("runtime"));
                            format!("{handle}{} ({runtime})", if lead { "*" } else { "" })
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|slots| !slots.is_empty())
                .unwrap_or_else(|| "-".to_owned());
            vec![cell(row.get("name")), cell(row.get("id")), slots]
        })
        .collect();
    table(&["NAME", "ID", "SLOTS"], rows)
}

fn render_crew(value: &Value) -> Vec<String> {
    key_values(&[
        ("NAME", value.get("name")),
        ("ID", value.get("id")),
        ("PURPOSE", value.get("purpose")),
        ("GOAL", value.get("goal")),
    ])
}

fn render_crew_show(value: &Value) -> Vec<String> {
    let crew = value.get("crew").unwrap_or(&Value::Null);
    let mut lines = render_crew(crew);
    let rows = value
        .get("slots")
        .and_then(Value::as_array)
        .map(|slots| {
            slots
                .iter()
                .map(|slot| {
                    let role = slot.get("role").unwrap_or(&Value::Null);
                    let runtime = effective_slot_value(slot, role, "runtime_override", "runtime");
                    let runtime_changed = slot
                        .get("runtime_override")
                        .and_then(Value::as_str)
                        .zip(role.get("runtime").and_then(Value::as_str))
                        .is_some_and(|(slot_runtime, role_runtime)| slot_runtime != role_runtime);
                    let model = if runtime_changed {
                        cell(slot.get("model_override"))
                    } else {
                        effective_slot_value(slot, role, "model_override", "model")
                    };
                    let effort = if runtime_changed {
                        cell(slot.get("effort_override"))
                    } else {
                        effective_slot_value(slot, role, "effort_override", "effort")
                    };
                    vec![
                        cell(slot.get("slot_handle")),
                        if slot.get("lead").and_then(Value::as_bool) == Some(true) {
                            "yes".to_owned()
                        } else {
                            "no".to_owned()
                        },
                        cell(role.get("handle")),
                        runtime,
                        model,
                        effort,
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !rows.is_empty() {
        lines.push(String::new());
        lines.extend(table(
            &["HANDLE", "LEAD", "ROLE", "RUNTIME", "MODEL", "EFFORT"],
            rows,
        ));
    }
    lines
}

fn effective_slot_value(slot: &Value, role: &Value, override_key: &str, role_key: &str) -> String {
    match slot.get(override_key) {
        Some(value) if !value.is_null() => cell(Some(value)),
        _ => cell(role.get(role_key)),
    }
}

fn render_mission_list(value: &Value) -> Vec<String> {
    let rows = array(value)
        .iter()
        .map(|row| {
            let statuses = row
                .get("session_statuses")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let running = statuses
                .iter()
                .filter(|entry| {
                    entry
                        .as_array()
                        .and_then(|pair| pair.get(1))
                        .and_then(|status| status.get("lifecycle"))
                        .and_then(Value::as_str)
                        == Some("running")
                })
                .count();
            vec![
                cell(row.get("id")),
                cell(row.get("title")),
                cell(row.get("crew_name")),
                cell(row.get("status")),
                format!("{running}/{}", statuses.len()),
                cell(row.get("pending_ask_count")),
                cell(row.get("project_id")),
            ]
        })
        .collect();
    table(
        &["ID", "TITLE", "CREW", "STATUS", "LIVE", "ASKS", "PROJECT"],
        rows,
    )
}

fn mission_value(value: &Value) -> &Value {
    value.get("mission").unwrap_or(value)
}

fn render_mission(value: &Value) -> Vec<String> {
    let mission = mission_value(value);
    key_values(&[
        ("ID", mission.get("id")),
        ("TITLE", mission.get("title")),
        ("STATUS", mission.get("status")),
        ("CREW", mission.get("crew_id")),
        ("PROJECT", mission.get("project_id")),
        ("CWD", mission.get("cwd")),
        ("STARTED", mission.get("started_at")),
    ])
}

fn render_mission_show(value: &Value) -> Vec<String> {
    let mission = mission_value(value);
    let crew = value.get("crew").unwrap_or(&Value::Null);
    let mut lines = key_values(&[
        ("ID", mission.get("id")),
        ("TITLE", mission.get("title")),
        ("STATUS", mission.get("status")),
        ("CREW", crew.get("name").or_else(|| mission.get("crew_id"))),
        ("PROJECT", mission.get("project_id")),
        ("CWD", mission.get("cwd")),
        ("STARTED", mission.get("started_at")),
    ]);

    let latest = value
        .get("latest_session_status_by_handle")
        .and_then(Value::as_object);
    let sessions = value
        .get("sessions")
        .and_then(Value::as_array)
        .map(|sessions| {
            sessions
                .iter()
                .map(|session| {
                    let handle = session.get("handle").and_then(Value::as_str).unwrap_or("");
                    let state = latest
                        .and_then(|latest| latest.get(handle))
                        .and_then(|status| status.get("state"));
                    vec![
                        cell(session.get("handle")),
                        if session.get("lead").and_then(Value::as_bool) == Some(true) {
                            "yes".to_owned()
                        } else {
                            "no".to_owned()
                        },
                        cell(session.get("runtime")),
                        cell(session.get("status")),
                        cell(state),
                        cell(session.get("id")),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !sessions.is_empty() {
        lines.push(String::new());
        lines.extend(table(
            &["HANDLE", "LEAD", "RUNTIME", "STATUS", "STATE", "SESSION ID"],
            sessions,
        ));
    }

    let asks = value
        .get("pending_asks")
        .and_then(Value::as_array)
        .map(|asks| {
            asks.iter()
                .map(|ask| {
                    vec![
                        cell(ask.get("question_id")),
                        cell(ask.get("asker")),
                        cell(ask.get("prompt")),
                        choices(ask.get("choices")),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !asks.is_empty() {
        lines.push(String::new());
        lines.extend(table(&["QUESTION ID", "ASKER", "PROMPT", "CHOICES"], asks));
    }

    let warnings = value
        .get("recent_warnings")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !warnings.is_empty() {
        lines.push(String::new());
        lines.push("RECENT WARNINGS".to_owned());
        for warning in warnings {
            lines.push(format!(
                "{}  {}  {}",
                clock(warning.get("ts")),
                cell_width(warning.get("from"), 20),
                cell_width(warning.get("message"), 72),
            ));
        }
    }
    lines
}

fn render_mission_feed(value: &Value) -> Vec<String> {
    let mut lines = feed_event_lines(value);
    let skipped = value
        .get("skipped")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if skipped > 0 {
        lines.push(format!("skipped      {skipped}"));
    }
    lines.push(format!("next_offset  {}", cell(value.get("next_offset"))));
    lines
}

fn feed_event_lines(value: &Value) -> Vec<String> {
    let mut events = value
        .get("events")
        .and_then(Value::as_array)
        .map(|events| events.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    events.sort_by(|left, right| {
        let left = left.get("event").unwrap_or(left);
        let right = right.get("event").unwrap_or(right);
        text(left.get("ts"))
            .cmp(&text(right.get("ts")))
            .then_with(|| text(left.get("id")).cmp(&text(right.get("id"))))
    });
    events
        .into_iter()
        .map(|entry| {
            let event = entry.get("event").unwrap_or(entry);
            let kind = event.get("type").or_else(|| event.get("kind"));
            format!(
                "{}  {} -> {}  {}  {}",
                clock(event.get("ts")),
                cell_width(event.get("from"), 20),
                event
                    .get("to")
                    .filter(|value| !value.is_null())
                    .map(|value| cell_width(Some(value), 20))
                    .unwrap_or_else(|| "all".to_owned()),
                cell_width(kind, 24),
                event_text(event),
            )
        })
        .collect()
}

pub fn write_feed_events(
    value: &Value,
    json: bool,
    quiet: bool,
    writer: &mut impl std::io::Write,
) -> std::io::Result<()> {
    let mut events = value
        .get("events")
        .and_then(Value::as_array)
        .map(|events| events.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    events.sort_by(|left, right| {
        let left = left.get("event").unwrap_or(left);
        let right = right.get("event").unwrap_or(right);
        text(left.get("ts"))
            .cmp(&text(right.get("ts")))
            .then_with(|| text(left.get("id")).cmp(&text(right.get("id"))))
    });
    let human = feed_event_lines(value);
    for (index, entry) in events.into_iter().enumerate() {
        let event = entry.get("event").unwrap_or(entry);
        if json {
            writeln!(writer, "{}", serde_json::to_string(event).unwrap())?;
        } else if quiet {
            writeln!(writer, "{}", text(event.get("id")))?;
        } else {
            writeln!(writer, "{}", human[index])?;
        }
        writer.flush()?;
    }
    Ok(())
}

fn event_text(event: &Value) -> String {
    let payload = event.get("payload").unwrap_or(&Value::Null);
    for key in ["text", "message", "prompt", "question", "note", "choice"] {
        if let Some(value) = payload.get(key) {
            return cell_width(Some(value), 72);
        }
    }
    cell_width(Some(payload), 72)
}

fn render_session_list(value: &Value) -> Vec<String> {
    let rows = array(value)
        .iter()
        .map(|row| {
            let title = row
                .get("title")
                .filter(|value| !value.is_null())
                .or_else(|| row.get("live_title").filter(|value| !value.is_null()))
                .or_else(|| row.get("display_name"));
            let role = row
                .get("handle")
                .filter(|value| !value.is_null())
                .or_else(|| row.get("role_id"));
            vec![
                cell(row.get("session_id")),
                cell(row.get("agent_runtime")),
                cell(role),
                cell(title),
                cell(row.get("status")),
                cell(row.get("activity")),
                cell(row.get("cwd")),
            ]
        })
        .collect();
    table(
        &[
            "ID", "RUNTIME", "ROLE", "TITLE", "STATUS", "ACTIVITY", "CWD",
        ],
        rows,
    )
}

fn render_session_show(value: &Value) -> Vec<String> {
    let status = value.get("agent_status").unwrap_or(&Value::Null);
    let observation = status.get("observation").unwrap_or(&Value::Null);
    let waits = observation
        .get("interactions")
        .and_then(Value::as_array)
        .map(|values| values.len().to_string())
        .unwrap_or_else(|| "0".into());
    let waits = Value::String(waits);
    key_values(&[
        ("ID", value.get("session_id").or_else(|| value.get("id"))),
        ("MISSION", value.get("mission_id")),
        ("RUNTIME", value.get("agent_runtime")),
        ("ROLE", value.get("handle").or_else(|| value.get("role_id"))),
        ("ROW STATUS", value.get("status")),
        ("LIFECYCLE", status.get("lifecycle")),
        ("ACTIVITY", observation.get("activity")),
        ("RAW ACTIVITY", value.get("activity")),
        ("SOURCE", observation.get("source")),
        ("OUTCOME", observation.get("outcome")),
        ("DETAIL", observation.get("detail")),
        ("WAITS", Some(&waits)),
        ("CWD", value.get("cwd")),
    ])
}

fn render_confirmation(action: &str, value: &Value) -> Vec<String> {
    let ids = ids(value);
    if ids.is_empty() {
        vec![action.to_owned()]
    } else {
        vec![format!("{action} {}", ids.join(", "))]
    }
}

fn render_generic(value: &Value) -> Vec<String> {
    match value {
        Value::Array(rows) => generic_table(rows),
        Value::Object(map) => {
            let pairs = map
                .iter()
                .map(|(key, value)| (key.as_str(), Some(value)))
                .collect::<Vec<_>>();
            key_values(&pairs)
        }
        Value::Null => Vec::new(),
        value => vec![cell(Some(value))],
    }
}

fn render_status(value: &Value) -> Vec<String> {
    let mut lines = key_values(&[
        ("CLI VERSION", value.get("cli_version")),
        ("APP VERSION", value.get("app_version")),
        ("SOCKET", value.get("socket")),
        ("SIDECAR", value.get("sidecar")),
        ("SIDECAR PRESENT", value.get("sidecar_present")),
        ("MODE", value.get("mode")),
    ]);
    if let Some(skills) = value.get("skills").and_then(Value::as_array) {
        lines.push(String::new());
        lines.push("SKILLS".into());
        lines.extend(skills.iter().map(|skill| {
            format!(
                "{}  {}",
                cell_width(skill.get("state"), 12),
                cell_width(skill.get("folder"), MAX_KEY_VALUE_WIDTH),
            )
        }));
    }
    lines
}

fn generic_table(rows: &[Value]) -> Vec<String> {
    let mut columns = Vec::new();
    for row in rows {
        if let Some(object) = row.as_object() {
            for key in object.keys() {
                if !columns.contains(key) {
                    columns.push(key.clone());
                }
            }
        }
    }
    if columns.is_empty() {
        return rows.iter().map(|row| cell(Some(row))).collect();
    }
    let values = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| cell(row.get(column)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let headers = columns.iter().map(String::as_str).collect::<Vec<_>>();
    table(&headers, values)
}

fn key_values(pairs: &[(&str, Option<&Value>)]) -> Vec<String> {
    let width = pairs.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{key:width$}  {}",
                cell_width(*value, MAX_KEY_VALUE_WIDTH),
                width = width
            )
        })
        .collect()
}

fn table(headers: &[&str], rows: Vec<Vec<String>>) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    let rows = rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|value| clean_and_truncate(&value, MAX_TABLE_CELL_WIDTH))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let widths = headers
        .iter()
        .enumerate()
        .map(|(index, header)| {
            rows.iter()
                .filter_map(|row| row.get(index))
                .map(|value| value.chars().count())
                .fold(header.chars().count(), usize::max)
                .min(MAX_TABLE_CELL_WIDTH)
        })
        .collect::<Vec<_>>();
    let mut lines = vec![table_line(
        &headers
            .iter()
            .map(|header| (*header).to_owned())
            .collect::<Vec<_>>(),
        &widths,
    )];
    lines.extend(rows.iter().map(|row| table_line(row, &widths)));
    lines
}

fn table_line(values: &[String], widths: &[usize]) -> String {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if index + 1 == values.len() {
                value.clone()
            } else {
                format!("{value:width$}", width = widths[index])
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

fn choices(value: Option<&Value>) -> String {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| text(Some(value)))
            .collect::<Vec<_>>()
            .join(", "),
        value => cell(value),
    }
}

fn clock(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .and_then(|value| value.get(11..19))
        .unwrap_or("--:--:--")
        .to_owned()
}

fn cell(value: Option<&Value>) -> String {
    cell_width(value, MAX_TABLE_CELL_WIDTH)
}

fn cell_width(value: Option<&Value>, width: usize) -> String {
    clean_and_truncate(&text(value), width)
}

fn text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "-".to_owned(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::String(value)) => value.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "?".to_owned()),
    }
}

fn clean_and_truncate(value: &str, width: usize) -> String {
    let clean = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = if clean.is_empty() { "-" } else { &clean };
    if clean.chars().count() <= width {
        return clean.to_owned();
    }
    let mut truncated = clean
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct FlushTrackingWriter {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl std::io::Write for FlushTrackingWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn quiet_ids_cover_nested_mission_lists_and_augmented_deletes() {
        assert_eq!(ids(&json!({"mission": {"id": "mission"}})), ["mission"]);
        assert_eq!(ids(&json!([{"id": "a"}, {"session_id": "b"}])), ["a", "b"]);
        assert_eq!(
            ids(&json!({"archived_session_ids": [], "id": "project"})),
            ["project"]
        );
    }

    #[test]
    fn machine_output_is_exact() {
        let response = ToolResponse {
            value: json!({"id": "01", "name": "Runner"}),
            raw_json: r#"{"id":"01","name":"Runner"}"#.into(),
        };
        assert_eq!(
            machine_lines(&response, true, false).unwrap(),
            [r#"{"id":"01","name":"Runner"}"#]
        );
        assert_eq!(machine_lines(&response, false, true).unwrap(), ["01"]);
        assert!(machine_lines(&response, false, false).is_none());
    }

    #[test]
    fn project_and_role_views_are_curated() {
        let project = json!({"id": "project-id", "name": "Runner", "cwd": "/repo", "nested": {"ignored": true}});
        assert_eq!(
            render_default(View::ProjectList, &json!([project.clone()])),
            ["NAME    ID          PATH", "Runner  project-id  /repo"]
        );
        assert_eq!(
            render_default(View::Project, &project),
            ["NAME  Runner", "ID    project-id", "PATH  /repo"]
        );

        let role = json!({
            "id": "role-id", "handle": "coder", "display_name": "Coder", "runtime": "codex",
            "model": null, "effort": "high", "system_prompt": "first line\nsecond line",
            "activity": {"nested": true}
        });
        assert_eq!(
            render_default(View::RoleList, &json!([role.clone()])),
            [
                "HANDLE  NAME   RUNTIME  MODEL  EFFORT  ID",
                "coder   Coder  codex    -      high    role-id",
            ]
        );
        assert_eq!(
            render_default(View::Role, &role),
            [
                "HANDLE   coder",
                "NAME     Coder",
                "RUNTIME  codex",
                "MODEL    -",
                "EFFORT   high",
                "ID       role-id",
                "",
                "PROMPT",
                "first line",
                "second line",
            ]
        );
    }

    #[test]
    fn crew_views_hide_conventions_and_render_slots() {
        let crew = json!({
            "id": "crew-id", "name": "Peer", "purpose": "Ship", "goal": null,
            "system_prompt_addendum": "multi\nline",
            "members": [
                {"slot_handle": "coder", "runtime": "codex", "lead": true},
                {"slot_handle": "reviewer", "runtime": "claude-code", "lead": false}
            ]
        });
        assert_eq!(
            render_default(View::CrewList, &json!([crew.clone()])),
            [
                "NAME  ID       SLOTS",
                "Peer  crew-id  coder* (codex), reviewer (claude-code)",
            ]
        );
        assert_eq!(
            render_default(View::Crew, &crew),
            [
                "NAME     Peer",
                "ID       crew-id",
                "PURPOSE  Ship",
                "GOAL     -"
            ]
        );
        let show = json!({
            "crew": crew,
            "slots": [{
                "slot_handle": "coder", "lead": true, "runtime_override": null,
                "model_override": "gpt", "effort_override": null,
                "role": {"handle": "coder", "runtime": "codex", "model": null, "effort": "high"}
            }]
        });
        assert_eq!(
            render_default(View::CrewShow, &show),
            [
                "NAME     Peer",
                "ID       crew-id",
                "PURPOSE  Ship",
                "GOAL     -",
                "",
                "HANDLE  LEAD  ROLE   RUNTIME  MODEL  EFFORT",
                "coder   yes   coder  codex    gpt    high",
            ]
        );
    }

    #[test]
    fn mission_views_render_live_counts_sessions_asks_and_warnings() {
        let summary = json!({
            "id": "mission-id", "title": "CLI", "crew_name": "Peer", "status": "running",
            "project_id": null, "pending_ask_count": 1,
            "session_statuses": [
                ["s1", {"lifecycle": "running"}], ["s2", {"lifecycle": "stopped"}]
            ]
        });
        assert_eq!(
            render_default(View::MissionList, &json!([summary])),
            [
                "ID          TITLE  CREW  STATUS   LIVE  ASKS  PROJECT",
                "mission-id  CLI    Peer  running  1/2   1     -",
            ]
        );
        let show = json!({
            "mission": {"id": "mission-id", "title": "CLI", "status": "running", "crew_id": "crew-id", "project_id": null, "cwd": "/repo", "started_at": "2026-09-18T10:20:30Z"},
            "crew": {"name": "Peer"},
            "sessions": [{"id": "session-id", "handle": "coder", "lead": true, "runtime": "codex", "status": "running"}],
            "latest_session_status_by_handle": {"coder": {"state": "busy"}},
            "pending_asks": [{"question_id": "question-id", "asker": "coder", "prompt": "Ship\nnow?", "choices": ["yes", "no"]}],
            "recent_warnings": [{"ts": "2026-09-18T10:21:31Z", "from": "router", "message": "retry\nlater", "payload": {"nested": true}}]
        });
        assert_eq!(
            render_default(View::MissionShow, &show),
            [
                "ID       mission-id",
                "TITLE    CLI",
                "STATUS   running",
                "CREW     Peer",
                "PROJECT  -",
                "CWD      /repo",
                "STARTED  2026-09-18T10:20:30Z",
                "",
                "HANDLE  LEAD  RUNTIME  STATUS   STATE  SESSION ID",
                "coder   yes   codex    running  busy   session-id",
                "",
                "QUESTION ID  ASKER  PROMPT     CHOICES",
                "question-id  coder  Ship now?  yes, no",
                "",
                "RECENT WARNINGS",
                "10:21:31  router  retry later",
            ]
        );
    }

    #[test]
    fn mutation_and_fallback_views_are_pinned() {
        let mission = json!({
            "mission": {
                "id": "mission-id", "title": "CLI", "status": "running", "crew_id": "crew-id",
                "project_id": null, "cwd": "/repo", "started_at": "2026-09-18T10:20:30Z"
            },
            "goal": "multi\nline"
        });
        assert_eq!(
            render_default(View::Mission, &mission),
            [
                "ID       mission-id",
                "TITLE    CLI",
                "STATUS   running",
                "CREW     crew-id",
                "PROJECT  -",
                "CWD      /repo",
                "STARTED  2026-09-18T10:20:30Z",
            ]
        );
        assert_eq!(
            render_default(
                View::Confirmation("Deleted project"),
                &json!({"id": "project-id", "nested": {"ignored": true}}),
            ),
            ["Deleted project project-id"]
        );
        assert_eq!(
            render_default(View::Generic, &json!({"nested": {"a": "one\ntwo"}})),
            [r#"nested  {"a":"one\ntwo"}"#]
        );
    }

    #[test]
    fn feed_and_session_views_are_one_line_per_row() {
        let feed = json!({
            "events": [
                {"event": {"id": "2", "ts": "2026-09-18T10:20:31Z", "kind": "signal", "from": "coder", "to": null, "type": "ask_lead", "payload": {"question": "Ship\nnow?"}}},
                {"event": {"id": "1", "ts": "2026-09-18T10:20:30Z", "kind": "message", "from": "human", "to": "coder", "payload": {"text": "go\nnow"}}}
            ],
            "skipped": [], "next_offset": 42
        });
        assert_eq!(
            render_default(View::MissionFeed, &feed),
            [
                "10:20:30  human -> coder  message  go now",
                "10:20:31  coder -> all  ask_lead  Ship now?",
                "next_offset  42",
            ]
        );
        let sessions = json!([{
            "session_id": "session-id", "agent_runtime": "codex", "handle": "coder",
            "title": null, "live_title": "Fix\nCLI", "display_name": "Coder", "status": "running", "activity": "working", "cwd": "/repo",
            "nested": {"ignored": true}
        }]);
        assert_eq!(
            render_default(View::SessionList, &sessions),
            [
                "ID          RUNTIME  ROLE   TITLE    STATUS   ACTIVITY  CWD",
                "session-id  codex    coder  Fix CLI  running  working   /repo",
            ]
        );
    }

    #[test]
    fn follow_json_is_flushed_ndjson_without_cursor_lines() {
        let feed = json!({
            "events": [
                {"next_offset": 20, "event": {"id": "2", "ts": "2026-09-18T10:20:31Z", "kind": "signal", "from": "coder", "type": "ask_lead", "payload": {}}},
                {"next_offset": 10, "event": {"id": "1", "ts": "2026-09-18T10:20:30Z", "kind": "message", "from": "human", "payload": {"text": "go"}}}
            ],
            "next_offset": 20
        });
        let mut writer = FlushTrackingWriter::default();
        write_feed_events(&feed, true, false, &mut writer).unwrap();
        assert_eq!(writer.flushes, 2);
        let lines = String::from_utf8(writer.bytes).unwrap();
        let parsed = lines
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["id"], "1");
        assert_eq!(parsed[1]["id"], "2");
        assert!(!lines.contains("next_offset"));
    }

    #[test]
    fn every_table_collapses_whitespace_truncates_and_prints_null_as_dash() {
        let long = "x".repeat(60);
        let rendered = table(
            &["A", "B", "C"],
            vec![vec!["one\ntwo".into(), long, "-".into()]],
        );
        assert_eq!(rendered[1].lines().count(), 1);
        assert!(rendered[1].contains("one two"));
        assert!(rendered[1].contains('…'));
        assert!(rendered[1].ends_with('-'));
    }
}
