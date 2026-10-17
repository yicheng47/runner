use std::io::{self, Write};

use runner_core::event_log::{EventLog, LogEntry};
use runner_core::model::{Event, EventDraft, EventKind, SignalType};
use runner_core::protocol::MissionReadCursor;

use crate::{env, roster};

const MAX_MESSAGE_BYTES: usize = 32 * 1024;

pub fn post(env: &env::MissionEnv, text: &str, to: &[String]) -> i32 {
    if text.len() > MAX_MESSAGE_BYTES {
        let size_kb = text.len().div_ceil(1024);
        eprintln!(
            "runner msg post: message is {size_kb}KB, limit is 32KB. \
             Write large content to a file and reference its path instead."
        );
        return 1;
    }

    let handles = match roster::load(&env.event_log) {
        Some(roster) => roster
            .into_iter()
            .map(|member| member.handle)
            .collect::<Vec<_>>(),
        None => {
            eprintln!("runner msg post: roster sidecar missing or unreadable");
            return 1;
        }
    };
    let resolved = match runner_core::addressing::resolve(text, to, &handles) {
        Ok(resolved) => resolved,
        Err(error) => {
            eprintln!("runner msg post: {error}");
            return 1;
        }
    };

    let Some(mission_dir) = env.event_log.parent() else {
        eprintln!(
            "runner: RUNNER_EVENT_LOG has no parent directory: {}",
            env.event_log.display()
        );
        return 2;
    };
    let log = match EventLog::open(mission_dir) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("runner: failed to open event log: {e}");
            return 1;
        }
    };

    let draft = EventDraft {
        crew_id: env.crew_id.clone(),
        mission_id: env.mission_id.clone(),
        kind: EventKind::Message,
        from: env.handle.clone(),
        to: resolved.recipients,
        signal_type: None,
        payload: serde_json::json!({ "text": resolved.text }),
    };
    match log.append(draft) {
        Ok(_ev) => 0,
        Err(e) => {
            eprintln!("runner: append failed: {e}");
            1
        }
    }
}

pub fn read(env: &env::MissionEnv, all: bool, since: Option<&str>, from: Option<&str>) -> i32 {
    let Some(mission_dir) = env.event_log.parent() else {
        return 2;
    };
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let log = EventLog::open(mission_dir)?;
        let cursor = runner_cli::client::mission_read_cursor(&env.mission_id, &env.handle);
        let batch = capture(&log, &env.handle, cursor, all, since, from)?;
        batch.print_and_ack(&log, env, &mut io::stdout().lock())?;
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("runner msg read: {error}");
            1
        }
    }
}

struct ReadBatch {
    messages: Vec<Event>,
    digest: Vec<Event>,
    ack: Option<String>,
}

fn capture(
    log: &EventLog,
    handle: &str,
    cursor: Option<MissionReadCursor>,
    all: bool,
    since: Option<&str>,
    from: Option<&str>,
) -> runner_core::Result<ReadBatch> {
    let history = all || since.is_some();
    let offset = cursor.as_ref().map_or(0, |cursor| {
        if history {
            cursor.start_offset
        } else {
            cursor.ack_offset.max(cursor.start_offset)
        }
    });
    let (entries, skipped) = log.read_from_lossy(offset)?;
    for skip in skipped {
        eprintln!(
            "runner msg read: skipping malformed line at offset {} ({})",
            skip.offset, skip.error
        );
    }
    Ok(select(
        entries,
        handle,
        cursor.unwrap_or_default(),
        all,
        since,
        from,
    ))
}

fn select(
    entries: Vec<LogEntry>,
    handle: &str,
    mut cursor: MissionReadCursor,
    all: bool,
    since: Option<&str>,
    from: Option<&str>,
) -> ReadBatch {
    let joined = entries.iter().find(|entry| {
        let event = &entry.event;
        event.kind == EventKind::Signal
            && event.from == "human"
            && event
                .signal_type
                .as_ref()
                .is_some_and(|kind| kind.as_str() == "role_joined")
            && event.payload["handle"].as_str() == Some(handle)
    });
    if let Some(joined) = joined {
        if joined.next_offset > cursor.start_offset {
            cursor = MissionReadCursor {
                start_offset: joined.next_offset,
                ..MissionReadCursor::default()
            };
        }
    }
    let mut inbox_ids = std::collections::BTreeSet::new();
    for entry in &entries {
        if entry.next_offset <= cursor.start_offset {
            continue;
        }
        let event = &entry.event;
        if event.addresses(handle) {
            inbox_ids.insert(event.id.as_str());
        }
        if event.kind == EventKind::Signal
            && event.from == handle
            && event
                .signal_type
                .as_ref()
                .is_some_and(|kind| kind.as_str() == "inbox_read")
        {
            if let Some(id) = event.payload["up_to"]
                .as_str()
                .and_then(|id| id.parse::<ulid::Ulid>().ok())
                .map(|id| id.to_string())
            {
                if inbox_ids.contains(id.as_str())
                    && cursor.ack_id.as_deref().is_none_or(|ack| id.as_str() > ack)
                {
                    cursor.ack_id = Some(id);
                }
            }
        }
    }
    let messages: Vec<_> = entries
        .iter()
        .filter(|entry| {
            let event = &entry.event;
            entry.next_offset > cursor.start_offset
                && event.kind == EventKind::Message
                && (all || event.addresses(handle))
                && since.is_none_or(|id| event.id.as_str() > id)
                && (all
                    || since.is_some()
                    || cursor
                        .ack_id
                        .as_deref()
                        .is_none_or(|id| event.id.as_str() > id))
                && from.is_none_or(|sender| event.from == sender)
        })
        .map(|entry| entry.event.clone())
        .collect();
    let last_inbox = messages.iter().rev().find(|event| event.addresses(handle));
    let ack = if since.is_none() && from.is_none() {
        last_inbox
            .filter(|event| {
                cursor
                    .ack_id
                    .as_deref()
                    .is_none_or(|id| event.id.as_str() > id)
            })
            .map(|event| event.id.clone())
    } else {
        None
    };
    let mut digest = Vec::new();
    if !all && since.is_none() && from.is_none() {
        if let Some(last) = last_inbox {
            digest = entries
                .iter()
                .filter(|entry| {
                    let event = &entry.event;
                    entry.next_offset > cursor.start_offset
                        && event.kind == EventKind::Message
                        && !event.recipients().is_empty()
                        && event.from != handle
                        && !event.addresses(handle)
                        && cursor
                            .ack_id
                            .as_deref()
                            .is_none_or(|id| event.id.as_str() > id)
                        && event.id <= last.id
                })
                .map(|entry| entry.event.clone())
                .collect();
            if digest.len() > 10 {
                digest.drain(..digest.len() - 10);
            }
        }
    }
    ReadBatch {
        messages,
        digest,
        ack,
    }
}

fn recipient_label(event: &Event) -> String {
    if event.recipients().is_empty() {
        "*".into()
    } else {
        event.recipients().join(", @")
    }
}

impl ReadBatch {
    fn print_and_ack(
        &self,
        log: &EventLog,
        env: &env::MissionEnv,
        output: &mut impl Write,
    ) -> io::Result<()> {
        for event in &self.messages {
            writeln!(
                output,
                "[{}] @{} -> @{}: {}",
                event.ts.to_rfc3339(),
                event.from,
                recipient_label(event),
                event.payload["text"].as_str().unwrap_or("")
            )?;
            writeln!(output, "  id: {}", event.id)?;
        }
        if !self.digest.is_empty() {
            writeln!(output, "Other crew traffic (latest 10; runner msg read --all for full messages and omitted entries):")?;
            for event in &self.digest {
                let line = event.payload["text"]
                    .as_str()
                    .unwrap_or("")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let preview: String = line.chars().take(120).collect();
                writeln!(
                    output,
                    "  @{} -> @{}: {}{}",
                    event.from,
                    recipient_label(event),
                    preview,
                    if line.chars().count() > 120 {
                        "…"
                    } else {
                        ""
                    }
                )?;
            }
        }
        output.flush()?;
        if let Some(id) = &self.ack {
            if let Err(error) = log.append(EventDraft::signal(
                env.crew_id.as_deref(),
                &env.mission_id,
                &env.handle,
                SignalType::new("inbox_read"),
                serde_json::json!({"up_to":id}),
            )) {
                eprintln!("runner msg read: failed to emit inbox_read: {error}");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, EventLog, env::MissionEnv) {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::open(dir.path()).unwrap();
        let env = env::MissionEnv {
            crew_id: None,
            mission_id: "m".into(),
            handle: "coder".into(),
            event_log: log.path().into(),
        };
        (dir, log, env)
    }
    fn message(log: &EventLog, from: &str, to: Option<&str>, text: &str) -> Event {
        log.append(EventDraft::message(
            None,
            "m",
            from,
            to.map(str::to_owned),
            text,
        ))
        .unwrap()
    }
    fn ack(log: &EventLog, id: &str) {
        log.append(EventDraft::signal(
            None,
            "m",
            "coder",
            "inbox_read",
            serde_json::json!({"up_to":id}),
        ))
        .unwrap();
    }
    fn ids(batch: &ReadBatch) -> Vec<&str> {
        batch
            .messages
            .iter()
            .map(|event| event.id.as_str())
            .collect()
    }

    #[test]
    fn read_contract_modes_and_filters() {
        let (_dir, log, env) = fixture();
        let a = message(&log, "qa", Some("coder"), "A");
        let other = message(&log, "qa", Some("reviewer"), "others");
        let own = message(&log, "coder", None, "sent");
        let b = message(&log, "reviewer", None, "B");
        for all in [false, true] {
            for since in [None, Some(a.id.as_str())] {
                for from in [None, Some("qa")] {
                    let batch = capture(&log, "coder", None, all, since, from).unwrap();
                    let expected: Vec<_> = [&a, &other, &own, &b]
                        .into_iter()
                        .filter(|event| {
                            (all || event.addresses("coder"))
                                && since.is_none_or(|id| event.id.as_str() > id)
                                && from.is_none_or(|sender| event.from == sender)
                        })
                        .map(|event| event.id.as_str())
                        .collect();
                    assert_eq!(
                        ids(&batch),
                        expected,
                        "all={all} since={since:?} from={from:?}"
                    );
                    assert_eq!(
                        batch.ack.as_deref(),
                        if since.is_none() && from.is_none() {
                            Some(b.id.as_str())
                        } else {
                            None
                        }
                    );
                    if since.is_some() || from.is_some() || all {
                        assert!(batch.digest.is_empty());
                    }
                }
            }
        }
        let filtered = capture(&log, "coder", None, false, Some(&a.id), None).unwrap();
        filtered.print_and_ack(&log, &env, &mut Vec::new()).unwrap();
        assert_eq!(
            ids(&capture(&log, "coder", None, false, None, None).unwrap()),
            vec![a.id.as_str(), b.id.as_str()]
        );
        let batch = capture(&log, "coder", None, false, None, None).unwrap();
        batch.print_and_ack(&log, &env, &mut Vec::new()).unwrap();
        let empty = capture(&log, "coder", None, false, None, None).unwrap();
        assert!(empty.messages.is_empty() && empty.digest.is_empty() && empty.ack.is_none());
        let history = capture(&log, "coder", None, true, None, None).unwrap();
        assert_eq!(
            ids(&history),
            vec![
                a.id.as_str(),
                other.id.as_str(),
                own.id.as_str(),
                b.id.as_str()
            ]
        );
        assert!(history.ack.is_none());
        let filtered_history = capture(&log, "coder", None, false, Some(&a.id), None).unwrap();
        assert_eq!(ids(&filtered_history), vec![b.id.as_str()]);
        assert!(filtered_history.ack.is_none());
        assert!(capture(&log, "coder", None, false, None, Some("qa"))
            .unwrap()
            .messages
            .is_empty());
    }

    #[test]
    fn history_acks_only_the_last_printed_inbox_message() {
        let (_dir, log, env) = fixture();
        let inbox = message(&log, "qa", Some("coder"), "inbox");
        let other = message(&log, "qa", Some("reviewer"), "later directed traffic");
        let own = message(&log, "coder", None, "later sent message");
        let history = capture(&log, "coder", None, true, None, None).unwrap();
        assert_eq!(
            ids(&history),
            vec![inbox.id.as_str(), other.id.as_str(), own.id.as_str()]
        );
        assert_eq!(history.ack.as_ref(), Some(&inbox.id));
        history.print_and_ack(&log, &env, &mut Vec::new()).unwrap();
        let empty = capture(&log, "coder", None, false, None, None).unwrap();
        assert!(empty.messages.is_empty() && empty.digest.is_empty() && empty.ack.is_none());
    }

    #[test]
    fn digest_is_latest_ten_in_ack_interval_and_never_advances_ack() {
        let (_dir, log, _) = fixture();
        let old = message(&log, "qa", None, "old");
        ack(&log, &old.id);
        let mut others = Vec::new();
        for i in 0..13 {
            others.push(message(
                &log,
                "qa",
                Some("reviewer"),
                &format!("other {i}\nnext line"),
            ));
        }
        let inbox = message(&log, "human", None, "inbox");
        message(&log, "qa", Some("reviewer"), "later");
        let batch = capture(&log, "coder", None, false, None, None).unwrap();
        assert_eq!(
            batch
                .digest
                .iter()
                .map(|event| &event.id)
                .collect::<Vec<_>>(),
            others[3..]
                .iter()
                .map(|event| &event.id)
                .collect::<Vec<_>>()
        );
        assert_eq!(batch.ack, Some(inbox.id));
    }

    #[test]
    fn captured_batch_does_not_ack_messages_arriving_before_its_ack() {
        let (_dir, log, env) = fixture();
        let a = message(&log, "qa", None, "A");
        let batch = capture(&log, "coder", None, false, None, None).unwrap();
        let b = message(&log, "qa", None, "B");
        batch.print_and_ack(&log, &env, &mut Vec::new()).unwrap();
        let offset = log.read_from(0).unwrap()[0].next_offset;
        let cursor = MissionReadCursor {
            ack_id: Some(a.id),
            ack_offset: offset,
            start_offset: 0,
        };
        assert_eq!(
            ids(&capture(&log, "coder", Some(cursor), false, None, None).unwrap()),
            vec![b.id.as_str()]
        );
        assert_eq!(
            ids(&capture(&log, "coder", None, false, None, None).unwrap()),
            vec![b.id.as_str()]
        );
    }

    #[test]
    fn stale_cursor_and_log_fallback_match_with_join_and_stale_acks() {
        let (_dir, log, _) = fixture();
        message(&log, "qa", Some("coder"), "before join");
        log.append(EventDraft::signal(
            None,
            "m",
            "coder",
            "role_joined",
            serde_json::json!({"handle":"coder"}),
        ))
        .unwrap();
        let join = log
            .append(EventDraft::signal(
                None,
                "m",
                "human",
                "role_joined",
                serde_json::json!({"handle":"coder"}),
            ))
            .unwrap();
        let a = message(&log, "qa", None, "A");
        let b = message(&log, "qa", None, "B");
        ack(&log, &b.id);
        ack(&log, &a.id);
        let c = message(&log, "qa", None, "C");
        let entries = log.read_from(0).unwrap();
        let start_offset = entries
            .iter()
            .find(|entry| entry.event.id == join.id)
            .unwrap()
            .next_offset;
        let offset = entries
            .iter()
            .find(|entry| entry.event.id == a.id)
            .unwrap()
            .next_offset;
        for cursor in [
            None,
            Some(MissionReadCursor {
                start_offset,
                ack_id: Some(a.id.clone()),
                ack_offset: offset,
            }),
        ] {
            assert_eq!(
                ids(&capture(&log, "coder", cursor.clone(), false, None, None).unwrap()),
                vec![c.id.as_str()]
            );
            assert_eq!(
                ids(&capture(&log, "coder", cursor, true, None, None).unwrap()),
                vec![a.id.as_str(), b.id.as_str(), c.id.as_str()]
            );
        }
    }

    #[test]
    fn failed_output_does_not_ack() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let (_dir, log, env) = fixture();
        message(&log, "qa", None, "A");
        let batch = capture(&log, "coder", None, false, None, None).unwrap();
        assert!(batch.print_and_ack(&log, &env, &mut Broken).is_err());
        assert_eq!(log.read_from(0).unwrap().len(), 1);
    }
}
