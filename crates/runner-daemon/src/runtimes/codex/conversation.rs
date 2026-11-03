use super::capture::{is_rollout_file, prompt_marker, rollout_paths_for_dates, session_meta};
use chrono::{DateTime, Local, Utc};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::{Duration, SystemTime};

// sessions/YYYY/MM/DD; archived_sessions is flat.
const ROLLOUT_DEPTH: usize = 3;
const OPENING_LINES: usize = 200;

/// Codex compresses older rollouts to `.jsonl.zst` and still resumes them,
/// so both count; an unreadable or absent sessions root proves nothing.
pub(super) fn conversation_exists(home: &Path, key: &str) -> Option<bool> {
    let key = uuid::Uuid::parse_str(key).ok()?.hyphenated().to_string();
    let suffixes = [format!("-{key}.jsonl"), format!("-{key}.jsonl.zst")];
    if find_rollout(&home.join("sessions"), &suffixes, ROLLOUT_DEPTH).ok()? {
        return Some(true);
    }
    match find_rollout(&home.join("archived_sessions"), &suffixes, ROLLOUT_DEPTH) {
        Ok(found) => Some(found),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

fn find_rollout(dir: &Path, suffixes: &[String], depth: usize) -> std::io::Result<bool> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if depth > 0 && find_rollout(&entry.path(), suffixes, depth - 1)? {
                return Ok(true);
            }
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("rollout-") && suffixes.iter().any(|suffix| name.ends_with(suffix)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A key replaced by a foreign Codex session (#1109) leaves the row's first
/// conversation on disk, opening with the row's capture marker. It is taken
/// back only when it is the one such rollout and was still being written
/// after the replacement key was minted, so a conversation the chat had
/// already left behind (a restart or `/new`) is never revived.
pub(super) fn recover_conversation(
    home: &Path,
    session_id: &str,
    missing_key: &str,
) -> Option<String> {
    let minted = uuid_v7_time(missing_key)?;
    let created = DateTime::<Utc>::from(ulid::Ulid::from_string(session_id).ok()?.datetime())
        .with_timezone(&Local);
    let dates = [
        created - chrono::Duration::days(1),
        created,
        created + chrono::Duration::days(1),
    ];
    let marker = prompt_marker(session_id);
    let mut recovered = None;
    for path in rollout_paths_for_dates(&home.join("sessions"), &dates) {
        if !is_rollout_file(&path)
            || !std::fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .is_ok_and(|modified| modified >= minted)
        {
            continue;
        }
        if let Some(id) = opened_with_marker(&path, &marker) {
            if recovered.replace(id).is_some() {
                return None;
            }
        }
    }
    recovered
}

fn uuid_v7_time(key: &str) -> Option<SystemTime> {
    let uuid = uuid::Uuid::parse_str(key).ok()?;
    if uuid.get_version_num() != 7 {
        return None;
    }
    let millis = uuid.as_bytes()[..6]
        .iter()
        .fold(0u64, |millis, byte| millis << 8 | u64::from(*byte));
    Some(SystemTime::UNIX_EPOCH + Duration::from_millis(millis))
}

/// The conversation's id when its opening prompt or startup hook, before any
/// reply, carries `marker`. Forks and subagent threads copy their parent's opening
/// prompt, and sibling agents can quote a marker later on, so neither counts.
fn opened_with_marker(path: &Path, marker: &str) -> Option<String> {
    let meta = session_meta(path)?;
    let payload = meta.get("payload")?;
    if payload
        .get("forked_from_id")
        .is_some_and(|value| !value.is_null())
        || payload
            .get("source")
            .is_some_and(|source| !source.is_string())
    {
        return None;
    }
    let id = payload.get("id")?.as_str()?;
    uuid::Uuid::parse_str(id).ok()?;
    let file = std::fs::File::open(path).ok()?;
    for line in BufReader::new(file).lines().skip(1).take(OPENING_LINES) {
        let line = line.ok()?;
        let Ok(item) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if item["type"] != "response_item" || item["payload"]["type"] != "message" {
            continue;
        }
        match item["payload"]["role"].as_str() {
            Some("assistant") => return None,
            Some("user" | "developer") if line.contains(marker) => return Some(id.to_owned()),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn v7(at: SystemTime) -> String {
        let millis = at
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut bytes = *uuid::Uuid::new_v4().as_bytes();
        bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..]);
        bytes[6] = 0x70 | (bytes[6] & 0x0f);
        uuid::Uuid::from_bytes(bytes).to_string()
    }

    fn day_dir(home: &Path, at: DateTime<Local>) -> PathBuf {
        let dir = home
            .join("sessions")
            .join(at.format("%Y").to_string())
            .join(at.format("%m").to_string())
            .join(at.format("%d").to_string());
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn message(role: &str, text: &str) -> serde_json::Value {
        json!({"type": "response_item", "payload": {
            "type": "message", "role": role,
            "content": [{"type": "input_text", "text": text}],
        }})
    }

    fn write_rollout(
        dir: &Path,
        id: &str,
        meta: serde_json::Value,
        items: &[serde_json::Value],
    ) -> PathBuf {
        let mut payload = json!({"id": id, "cwd": "/work", "source": "cli"});
        payload
            .as_object_mut()
            .unwrap()
            .extend(meta.as_object().unwrap().clone());
        let mut body = json!({"type": "session_meta", "payload": payload}).to_string() + "\n";
        for item in items {
            body.push_str(&item.to_string());
            body.push('\n');
        }
        let path = dir.join(format!("rollout-2026-11-03T09-22-56-{id}.jsonl"));
        std::fs::write(&path, body).unwrap();
        path
    }

    fn opening(marker: &str) -> Vec<serde_json::Value> {
        vec![
            message("developer", "rules"),
            message("user", "<environment_context/>"),
            message("user", &format!("goal\n\n{marker}")),
            message("assistant", "on it"),
        ]
    }

    #[test]
    fn probe_counts_plain_compressed_and_archived_rollouts() {
        let home = tempfile::tempdir().unwrap();
        let dir = day_dir(home.path(), Local::now());
        let [plain, compressed, archived, missing] = std::array::from_fn(|_| v7(SystemTime::now()));
        std::fs::write(
            dir.join(format!("rollout-2026-11-03T09-22-56-{plain}.jsonl")),
            "",
        )
        .unwrap();
        std::fs::write(
            dir.join(format!(
                "rollout-2026-10-20T09-22-56-{compressed}.jsonl.zst"
            )),
            "",
        )
        .unwrap();
        std::fs::create_dir(home.path().join("archived_sessions")).unwrap();
        std::fs::write(
            home.path().join(format!(
                "archived_sessions/rollout-2026-10-01T09-22-56-{archived}.jsonl"
            )),
            "",
        )
        .unwrap();

        for key in [&plain, &compressed, &archived, &plain.to_uppercase()] {
            assert_eq!(conversation_exists(home.path(), key), Some(true), "{key}");
        }
        assert_eq!(conversation_exists(home.path(), &missing), Some(false));
        assert_eq!(conversation_exists(home.path(), "not-a-uuid"), None);
    }

    #[test]
    fn probe_without_a_sessions_root_proves_nothing() {
        let home = tempfile::tempdir().unwrap();
        let key = v7(SystemTime::now());
        assert_eq!(conversation_exists(home.path(), &key), None);
        std::fs::create_dir(home.path().join("archived_sessions")).unwrap();
        assert_eq!(conversation_exists(home.path(), &key), None);
    }

    #[test]
    fn recovery_takes_back_only_the_rollout_that_opened_with_the_row_marker() {
        let home = tempfile::tempdir().unwrap();
        let session = ulid::Ulid::new();
        let marker = prompt_marker(&session.to_string());
        let dir = day_dir(
            home.path(),
            DateTime::<Utc>::from(session.datetime()).into(),
        );
        let minted = SystemTime::now() - Duration::from_secs(60);
        let missing = v7(minted);
        let original = v7(minted - Duration::from_secs(3600));
        write_rollout(&dir, &original, json!({}), &opening(&marker));
        write_rollout(
            &dir,
            &v7(minted),
            json!({}),
            &[
                message("user", "review the coder's branch"),
                message("assistant", "reading"),
                message("user", &marker),
            ],
        );
        write_rollout(
            &dir,
            &v7(minted),
            json!({"source": {"subagent": {"other": "guardian"}}}),
            &opening(&marker),
        );
        write_rollout(
            &dir,
            &v7(minted),
            json!({"forked_from_id": original}),
            &opening(&marker),
        );

        assert_eq!(
            recover_conversation(home.path(), &session.to_string(), &missing),
            Some(original)
        );
    }

    #[test]
    fn recovery_accepts_startup_hook_marker_without_a_launch_prompt() {
        let home = tempfile::tempdir().unwrap();
        let session = ulid::Ulid::new();
        let marker = prompt_marker(&session.to_string());
        let dir = day_dir(
            home.path(),
            DateTime::<Utc>::from(session.datetime()).into(),
        );
        let minted = SystemTime::now() - Duration::from_secs(60);
        let missing = v7(minted);
        let original = v7(minted - Duration::from_secs(60));
        write_rollout(
            &dir,
            &original,
            json!({}),
            &[
                message("developer", &marker),
                message("user", "Remember the word banana. Reply OK."),
                message("assistant", "OK"),
            ],
        );
        write_rollout(
            &dir,
            &v7(minted),
            json!({}),
            &[
                message("user", "a different conversation"),
                message("assistant", "OK"),
                message("developer", &marker),
            ],
        );
        assert_eq!(
            recover_conversation(home.path(), &session.to_string(), &missing),
            Some(original)
        );
    }

    #[test]
    fn recovery_refuses_stale_ambiguous_or_untimed_candidates() {
        let home = tempfile::tempdir().unwrap();
        let session = ulid::Ulid::new();
        let id = session.to_string();
        let marker = prompt_marker(&id);
        let dir = day_dir(
            home.path(),
            DateTime::<Utc>::from(session.datetime()).into(),
        );
        let minted = SystemTime::now() - Duration::from_secs(60);
        let missing = v7(minted);
        let original = write_rollout(&dir, &v7(minted), json!({}), &opening(&marker));

        assert_eq!(
            recover_conversation(home.path(), &id, &uuid::Uuid::new_v4().to_string()),
            None,
            "a key without a v7 timestamp gives no point to compare against"
        );

        std::fs::File::options()
            .write(true)
            .open(&original)
            .unwrap()
            .set_modified(minted - Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            recover_conversation(home.path(), &id, &missing),
            None,
            "a conversation left before the key was replaced is not revived"
        );

        std::fs::File::options()
            .write(true)
            .open(&original)
            .unwrap()
            .set_modified(SystemTime::now())
            .unwrap();
        write_rollout(&dir, &v7(minted), json!({}), &opening(&marker));
        assert_eq!(recover_conversation(home.path(), &id, &missing), None);
    }
}
