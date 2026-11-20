use super::*;
use crate::error::{Error, Result};

pub(super) fn effective_env(input: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    #[cfg(test)]
    use crate::golden::config_var_os as var_os;
    #[cfg(not(test))]
    use std::env::var_os;
    let mut env = input.clone();
    for key in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "OPENCODE_CONFIG_CONTENT",
        "OPENCODE_CLI_CONFIG_CONTENT",
        "OPENCODE_DB",
    ] {
        if !env.contains_key(key) {
            if let Some(value) = var_os(key).and_then(|v| v.into_string().ok()) {
                env.insert(key.into(), value);
            }
        }
    }
    env
}

pub(super) fn config_dir(env: &BTreeMap<String, String>, home: Option<&Path>) -> Option<PathBuf> {
    let env = effective_env(env);
    let home = env
        .get(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(Path::to_path_buf));
    let base = env
        .get("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".config")))?;
    Some(base.join("opencode"))
}
pub(super) fn global_config_path(home: &Path, env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let dir = config_dir(env, Some(home))?;
    let jsonc = dir.join("opencode.jsonc");
    Some(if jsonc.exists() {
        jsonc
    } else {
        dir.join("opencode.json")
    })
}
pub(super) fn defaults(
    home: &Path,
    env: &BTreeMap<String, String>,
) -> crate::runtime_defaults::RuntimeDefaults {
    let env = effective_env(env);
    let mut model = None;
    if let Some(dir) = config_dir(&env, Some(home)) {
        for file in [dir.join("opencode.json"), dir.join("opencode.jsonc")] {
            if let Ok(raw) = std::fs::read_to_string(file) {
                if let Ok(value) = parse(&raw) {
                    model = model_value(&value).or(model);
                }
            }
        }
    }
    if let Some(raw) = env.get("OPENCODE_CONFIG_CONTENT") {
        if let Ok(value) = parse(raw) {
            model = model_value(&value).or(model);
        }
    }
    crate::runtime_defaults::RuntimeDefaults {
        model,
        effort: None,
    }
}
fn model_value(value: &Value) -> Option<String> {
    if let Some(model) = value["model"].as_str() {
        return trim_some(Some(model)).map(str::to_owned);
    }
    Some(format!(
        "{}/{}",
        value["model"]["providerID"].as_str()?,
        value["model"]["model"].as_str()?
    ))
}
pub(super) fn db_path(env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let env = effective_env(env);
    #[cfg(test)]
    let home = test_home();
    #[cfg(not(test))]
    let home = runner_core::app_paths::home_dir();
    let home = env
        .get(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or(home);
    let data = env
        .get("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(".local/share")))
        .map(|data| data.join("opencode"));
    if let Some(override_db) = env.get("OPENCODE_DB").filter(|v| !v.is_empty()) {
        if override_db == ":memory:" {
            return None;
        }
        let path = PathBuf::from(override_db);
        return if path.is_absolute() {
            Some(path)
        } else {
            data.map(|dir| dir.join(path))
        };
    }
    data.map(|dir| dir.join("opencode.db"))
}

pub(super) fn conversation_exists(key: &str, ctx: &ProbeContext<'_>) -> Option<bool> {
    if !valid_key(key) {
        return Some(false);
    }
    let env: BTreeMap<_, _> = ctx
        .role_env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let path = db_path(&env)?;
    if !path.exists() {
        return Some(false);
    }
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM session_v2 WHERE id = ?1)",
            [key],
            |row| row.get(0),
        )
        .ok()
}

// Mask JSONC syntax in place so serde parses structure while edit spans still
// address the original UTF-8 bytes. Never deserialize and rewrite a user file.
fn without_comments(raw: &str) -> Result<Vec<u8>> {
    let mut bytes = raw.as_bytes().to_vec();
    let mut i = 0;
    let mut quoted = false;
    while i < bytes.len() {
        if quoted {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                quoted = false;
            }
        } else if bytes[i] == b'"' {
            quoted = true;
        } else if bytes.get(i..i + 2) == Some(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                bytes[i] = b' ';
                i += 1;
            }
            continue;
        } else if bytes.get(i..i + 2) == Some(b"/*") {
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
            let mut closed = false;
            while i < bytes.len() {
                if bytes.get(i..i + 2) == Some(b"*/") {
                    bytes[i] = b' ';
                    bytes[i + 1] = b' ';
                    i += 2;
                    closed = true;
                    break;
                }
                if !matches!(bytes[i], b'\n' | b'\r') {
                    bytes[i] = b' ';
                }
                i += 1;
            }
            if !closed {
                return Err(Error::msg("unterminated JSONC comment"));
            }
            continue;
        }
        i += 1;
    }
    Ok(bytes)
}
fn normalized(raw: &str) -> Result<String> {
    let mut bytes = without_comments(raw)?;
    let mut quoted = false;
    let mut i = 0;
    while i < bytes.len() {
        if quoted {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                quoted = false;
            }
        } else if bytes[i] == b'"' {
            quoted = true;
        } else if bytes[i] == b',' {
            let next = bytes[i + 1..]
                .iter()
                .copied()
                .find(|b| !b.is_ascii_whitespace());
            if matches!(next, Some(b'}' | b']')) {
                bytes[i] = b' ';
            }
        }
        i += 1;
    }
    String::from_utf8(bytes).map_err(|e| Error::msg(e.to_string()))
}
pub(super) fn parse(raw: &str) -> Result<Value> {
    Ok(serde_json::from_str(&normalized(raw)?)?)
}
pub(super) fn member_range(raw: &str, key: &str) -> Result<Option<std::ops::Range<usize>>> {
    let clean = normalized(raw)?;
    if !serde_json::from_str::<Value>(&clean)?.is_object() {
        return Err(Error::msg("expected JSONC object"));
    }
    let (members, _) = crate::ops::mcp::json_members(&clean)?;
    if members.iter().filter(|m| m.key == key).count() > 1 {
        return Err(Error::msg("duplicate JSONC member"));
    }
    Ok(members.into_iter().find(|m| m.key == key).map(|m| m.value))
}
pub(super) fn replace(raw: &str, key: &str, value: Option<&Value>) -> Result<String> {
    let clean = normalized(raw)?;
    if !serde_json::from_str::<Value>(&clean)?.is_object() {
        return Err(Error::msg("expected JSONC object"));
    }
    let (members, close) = crate::ops::mcp::json_members(&clean)?;
    if members.iter().filter(|m| m.key == key).count() > 1 {
        return Err(Error::msg("duplicate JSONC member"));
    }
    let mut out = raw.to_owned();
    let mask = without_comments(raw)?;
    if let Some((index, member)) = members.iter().enumerate().find(|(_, m)| m.key == key) {
        if let Some(value) = value {
            out.replace_range(member.value.clone(), &serde_json::to_string_pretty(value)?);
        } else {
            let comma = (member.value.end..close)
                .find(|i| !mask[*i].is_ascii_whitespace())
                .filter(|i| mask[*i] == b',');
            if let Some(comma) = comma {
                out.replace_range(member.key_start..comma + 1, "");
            } else {
                out.replace_range(member.key_start..member.value.end, "");
                if index > 0 {
                    if let Some(comma) =
                        (members[index - 1].value.end..member.key_start).find(|i| mask[*i] == b',')
                    {
                        out.remove(comma);
                    }
                }
            }
        }
    } else if let Some(value) = value {
        let mut text = format!(
            "{}: {}",
            serde_json::to_string(key)?,
            serde_json::to_string_pretty(value)?
        );
        if let Some(last) = members.last() {
            let comma = mask[last.value.end..close].contains(&b',');
            if !comma {
                text.insert(0, ',');
            }
        }
        out.insert_str(close, &text);
    }
    parse(&out)?;
    Ok(out)
}
