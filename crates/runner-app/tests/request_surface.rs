use std::fs;
use std::path::{Path, PathBuf};

const ALLOWLIST: &[(&str, &str)] = &[
    (
        "bootstrap.rs",
        "1c: boot_core, NativeMcpServer, client host, wake installation and quit teardown",
    ),
    ("terminal/", "1b: TerminalSession and terminal renderer"),
    ("terminal_ime.rs", "1b: terminal input error type"),
    ("surfaces/agent_update.rs", "1b: UpdateTerminalEvents"),
    (
        "app_store.rs:TerminalBridge::new",
        "1b: terminal bridge construction",
    ),
];

fn sources(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

fn code_only(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut code = bytes.to_vec();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        if bytes[i..].starts_with(b"//") {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i..].starts_with(b"/*") {
            let mut depth = 1;
            i += 2;
            while i < bytes.len() && depth != 0 {
                if bytes[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if bytes[i] == b'r' && bytes.get(i + 1).is_some_and(|b| *b == b'#' || *b == b'"') {
            let mut quote = i + 1;
            while bytes.get(quote) == Some(&b'#') {
                quote += 1;
            }
            if bytes.get(quote) != Some(&b'"') {
                i += 1;
                continue;
            }
            let hashes = quote - i - 1;
            i = quote + 1;
            while i < bytes.len() {
                if bytes[i] == b'"'
                    && bytes
                        .get(i + 1..i + 1 + hashes)
                        .is_some_and(|tail| tail.iter().all(|b| *b == b'#'))
                {
                    i += 1 + hashes;
                    break;
                }
                i += 1;
            }
        } else if bytes[i] == b'"' || (bytes[i] == b'\'' && bytes.get(i + 2) == Some(&b'\'')) {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
        } else {
            i += 1;
            continue;
        }
        for byte in &mut code[start..i.min(bytes.len())] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(code).unwrap()
}

fn tokens(code: &str) -> Vec<(&str, usize)> {
    let mut tokens = Vec::new();
    let mut chars = code.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        let mut end = start + ch.len_utf8();
        if ch.is_alphanumeric() || ch == '_' {
            while let Some(&(offset, next)) = chars.peek() {
                if !next.is_alphanumeric() && next != '_' {
                    break;
                }
                end = offset + next.len_utf8();
                chars.next();
            }
        }
        tokens.push((&code[start..end], start));
    }
    tokens
}

fn without_tests<'a>(tokens: &[(&'a str, usize)]) -> Vec<(&'a str, usize)> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].0 == "#" && tokens.get(i + 1).is_some_and(|t| t.0 == "[") {
            let end = (i + 2..tokens.len()).find(|j| tokens[*j].0 == "]").unwrap();
            let attr = tokens[i..=end].iter().map(|t| t.0).collect::<String>();
            if attr == "#[cfg(test)]" || attr.starts_with("#[cfg(all(test,") {
                i = end + 1;
                let mut depth = 0;
                while i < tokens.len() {
                    let token = tokens[i].0;
                    i += 1;
                    if token == "{" {
                        depth += 1;
                    } else if token == "}" {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else if token == ";" && depth == 0 {
                        break;
                    }
                }
                continue;
            }
        }
        result.push(tokens[i]);
        i += 1;
    }
    result
}

fn violations(path: &str, source: &str) -> Vec<String> {
    if ALLOWLIST.iter().any(|(allowed, _)| {
        !allowed.contains(':') && (path == *allowed || path.starts_with(allowed))
    }) {
        return Vec::new();
    }
    let code = code_only(source);
    let tokens = tokens(&code);
    let tokens = without_tests(&tokens);
    let fields = [
        "db",
        "usage",
        "events",
        "runtime_discovery",
        "runtime_shell_env",
        "session_events",
        "mcp",
        "windows",
        "sessions",
        "broadcast_focus_map",
    ];
    let mut failures = Vec::new();
    for (i, &(token, offset)) in tokens.iter().enumerate() {
        let tail = tokens[i..].iter().take(6).map(|t| t.0).collect::<Vec<_>>();
        let backend = tail.starts_with(&["runner_backend", ":", ":"]) || token == "AppCore";
        let db = tail.starts_with(&[".", "db", ".", "get", "("]);
        let mut field_at = i + 1;
        if tokens.get(field_at).is_some_and(|token| token.0 == "(") {
            let mut depth = 1;
            field_at += 1;
            while field_at < tokens.len() && depth != 0 {
                match tokens[field_at].0 {
                    "(" => depth += 1,
                    ")" => depth -= 1,
                    _ => {}
                }
                field_at += 1;
            }
        }
        let field = (token == "core" || token.ends_with("_core"))
            && tokens.get(field_at).is_some_and(|token| token.0 == ".")
            && tokens
                .get(field_at + 1)
                .is_some_and(|token| fields.contains(&token.0));
        let update_host = tail.starts_with(&[".", "update_host", ".", "0"]);
        let terminal = tail.starts_with(&[".", "terminal_core", "("]);
        let bridge_construction = path == "app_store.rs"
            && source[..offset]
                .lines()
                .last()
                .is_some_and(|line| line.contains("TerminalBridge::new"));
        if backend || db || field || update_host || (terminal && !bridge_construction) {
            let line = source[..offset].bytes().filter(|b| *b == b'\n').count() + 1;
            failures.push(format!("{path}:{line}: {token}"));
        }
    }
    failures
}

#[test]
fn app_reaches_the_core_only_through_the_client() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    let mut failures = Vec::new();
    for file in files {
        let path = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if path.ends_with("tests.rs")
            || path.contains("/tests/")
            || ["test_support.rs", "theme_snapshot.rs", "catalog_golden.rs"]
                .contains(&path.as_str())
        {
            continue;
        }
        failures.extend(violations(&path, &fs::read_to_string(file).unwrap()));
    }
    assert!(
        failures.is_empty(),
        "direct core access:\n{}\nallowlist: {ALLOWLIST:?}",
        failures.join("\n")
    );
}

#[test]
fn guard_detects_planted_access_and_skips_test_modules() {
    for source in [
        "fn bad() { this.core(cx).usage.snapshot(); }",
        "fn bad() { store.update_host.0.events.subscribe(); }",
        "fn action() { runner_backend::ops::role::role_list(core); }",
        "fn action() { core.db.get(); }",
        "fn action() { core.usage.refresh(); }",
        "fn action() { let core: AppCore; }",
    ] {
        assert!(!violations("surfaces/example.rs", source).is_empty());
    }
    assert!(violations("surfaces/example.rs", "#[cfg(test)] mod tests { use runner_backend::AppCore; fn fixture() { core.db.get(); } }\nfn render() { let text = \"runner_backend::ops\"; }").is_empty());
    assert!(!violations(
        "surfaces/example.rs",
        "#[cfg(test)] mod tests { }\nfn action() { runner_backend::ops::role::role_list(core); }"
    )
    .is_empty());
}
