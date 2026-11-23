use super::*;
use tempfile::TempDir;

fn paths(dir: &TempDir) -> BTreeMap<McpClientId, PathBuf> {
    McpClientId::all()
        .into_iter()
        .map(|c| (c, dir.path().join(c.key())))
        .collect()
}

fn catalog(paths: &BTreeMap<McpClientId, PathBuf>) -> McpCatalog {
    catalog_at(paths)
}

fn claude_code_write_at(path: &Path) -> Result<()> {
    write_entry_at(path, McpClientId::ClaudeCode, "runner", None, false)
}

fn codex_write_at(path: &Path) -> Result<()> {
    write_entry_at(path, McpClientId::Codex, "runner", None, false)
}

fn copilot_write_at(path: &Path) -> Result<()> {
    write_entry_at(path, McpClientId::Copilot, "runner", None, false)
}

#[test]
fn truncated_json_reports_an_error_instead_of_panicking() {
    for raw in ["{", "{\"a\"", "{\"a\":", "{\"a\": 1", "{\"a\": 1,"] {
        assert!(
            json_members(raw).is_err(),
            "expected an error for {raw:?}, not a panic"
        );
    }
    assert!(json_members("{\"a\": 1}").is_ok());
}

#[test]
fn toml_key_comments_stay_single_across_saves_copies_and_removal() {
    use super::test_clients::*;
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    let prefix = "# user settings\nmodel = 'gpt-5'\n\n[mcp_servers] # keep parent\n";
    let entry = "\n[mcp_servers.github]\n# token comes from the keychain\ncommand = 'gh-mcp'\n\nstartup_timeout_sec = 60\n";
    let suffix = "\n[ui]\ntheme = 'dark'\n";
    let original = format!("{prefix}{entry}{suffix}");
    std::fs::write(
        &paths[&ClaudeCode],
        r#"{"mcpServers":{"github":{"command":"new-mcp"}}}"#,
    )
    .unwrap();
    for client in [Codex, Trae] {
        std::fs::write(&paths[&client], &original).unwrap();
        for _ in 0..2 {
            let text = read_entries_at(&paths[&client], client).unwrap()["github"].text("github");
            edit_at(&paths, client, "github", &text, &[]).unwrap();
            assert_eq!(std::fs::read_to_string(&paths[&client]).unwrap(), original);
        }
        for _ in 0..2 {
            copy_at(&paths, ClaudeCode, client, "github").unwrap();
            let copied = std::fs::read_to_string(&paths[&client]).unwrap();
            assert!(copied.starts_with(prefix));
            assert!(copied.ends_with(suffix));
            assert_eq!(copied.matches("# token comes from the keychain").count(), 1);
            assert!(copied.contains("\n\nstartup_timeout_sec = 60\n"));
        }
        write_entry_at(&paths[&client], client, "github", None, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths[&client]).unwrap(),
            format!("{prefix}{suffix}")
        );
    }
}

#[test]
fn toml_writes_preserve_crlf_missing_final_newline_and_interleaved_tables() {
    use super::test_clients::*;
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    let prefix = "# user settings\r\nmodel = 'gpt-5'\r\n\r\n[mcp_servers] # parent\r\n";
    let middle = "\r\n[mcp_servers.other]\r\ncommand = 'other' # untouched\r\n";
    let suffix = "\r\n[ui]\r\ntheme = 'dark'";
    let raw = format!("{prefix}\r\n[mcp_servers.github]\r\ncommand = 'old'\r\nstartup_timeout_sec = 60\r\n{middle}\r\n[mcp_servers.github.env]\r\nTOKEN = 'old'\r\n{suffix}");
    std::fs::write(&paths[&Codex], &raw).unwrap();
    std::fs::write(
        &paths[&ClaudeCode],
        r#"{"mcpServers":{"github":{"command":"new","env":{"TOKEN":"new"}}}}"#,
    )
    .unwrap();
    copy_at(&paths, ClaudeCode, Codex, "github").unwrap();
    let after = std::fs::read_to_string(&paths[&Codex]).unwrap();
    assert!(after.starts_with(prefix));
    assert!(after.contains(middle));
    assert!(after.ends_with(suffix));
    assert!(after.contains("startup_timeout_sec = 60"));
    let body = read_entries_at(&paths[&Codex], Codex).unwrap()["github"].text("github");
    edit_at(&paths, Codex, "github", &body.replace("new", "edited"), &[]).unwrap();
    let after = std::fs::read_to_string(&paths[&Codex]).unwrap();
    assert!(after.starts_with(prefix));
    assert!(after.contains(middle));
    assert!(after.ends_with(suffix));
    write_entry_at(&paths[&Codex], Codex, "github", None, false).unwrap();
    let removed = std::fs::read_to_string(&paths[&Codex]).unwrap();
    assert_eq!(removed, format!("{prefix}{middle}{suffix}"));
}

#[test]
fn inline_and_dotted_toml_entries_keep_siblings_and_parent() {
    for raw in [
            "model = 'gpt'\r\nmcp_servers = { github = { command = 'old' }, other = { command = 'other' } }\r\n# trailing",
            "[mcp_servers]\r\ngithub = { command = 'old' }\r\nother = { command = 'other' }\r\n# trailing",
            "mcp_servers.github.command = 'old'\r\nmcp_servers.other.command = 'other'\r\n# trailing",
        ] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, raw).unwrap();
            let mut replacement = toml_edit::Table::new();
            replacement["command"] = toml_edit::value("new");
            write_entry_at(&path, McpClientId::Codex, "github", Some(NativeEntry::Toml(replacement)), false).unwrap();
            let updated = std::fs::read_to_string(&path).unwrap();
            assert!(updated.contains("command = 'other'"));
            assert!(updated.contains("\r\n# trailing"));
            write_entry_at(&path, McpClientId::Codex, "github", None, false).unwrap();
            let removed = std::fs::read_to_string(&path).unwrap();
            assert!(removed.contains("command = 'other'"), "input: {raw:?}\nremoved: {removed:?}");
            assert!(!removed.contains("github"));
        }
    let raw = "mcp_servers = { github = { command = 'old' } }\r\nmodel = 'gpt'";
    assert_eq!(
        splice_toml_entry(raw, "github", None).unwrap(),
        "mcp_servers = {  }\r\nmodel = 'gpt'"
    );
}

#[test]
fn runner_status_reads_inline_registration_without_overwriting_it() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    for raw in [
        "mcp_servers = { runner = { command = '/other/runner' } }",
        "[mcp_servers]\nrunner = { command = '/other/runner' }",
    ] {
        std::fs::write(&path, raw).unwrap();
        let status = codex_status_at(&path, "/current/runner").unwrap();
        assert!(status.registered && !status.matches_current);
        assert_eq!(status.command.as_deref(), Some("/other/runner"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
    }
}

#[cfg(unix)]
#[test]
fn read_only_claude_file_fails_by_path_while_codex_edit_lands() {
    use super::test_clients::*;
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    let json = r#"{"mcpServers":{"github":{"command":"old"}}, "theme" : "dark"}"#;
    std::fs::write(&paths[&ClaudeCode], json).unwrap();
    std::fs::set_permissions(&paths[&ClaudeCode], std::fs::Permissions::from_mode(0o400)).unwrap();
    let result = edit_at(
        &paths,
        ClaudeCode,
        "github",
        r#"{"command":"new"}"#,
        &[Codex],
    );
    std::fs::set_permissions(&paths[&ClaudeCode], std::fs::Permissions::from_mode(0o600)).unwrap();
    let error = result.unwrap_err().to_string();
    assert!(error.contains(paths[&ClaudeCode].to_str().unwrap()));
    assert_eq!(std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(), json);
    assert!(read_entries_at(&paths[&Codex], Codex).unwrap()["github"]
        .text("github")
        .contains("new"));
}

#[test]
fn empty_catalog_does_not_seed_a_runner_server() {
    let dir = TempDir::new().unwrap();
    assert!(catalog(&paths(&dir)).servers.is_empty());
}

#[test]
fn catalog_copies_conflicts_and_edits_without_touching_other_entries() {
    use super::test_clients::*;
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    let claude = "{\n  \"theme\" : \"dark\", \"mcpServers\" : {\"github\": {\"command\":\"gh-mcp\"}, \"runner\": {\"command\":\"/runner\"}}, \"n\":1e2\n}\n";
    let rest = "# model comment\nmodel = 'gpt-5'\n\n[tools]\nweb = true # stay\n\n[mcp_servers.other]\ncommand = 'other'\n\n[mcp_servers.other.env]\nTOKEN = 'secret'\n";
    std::fs::write(&paths[&ClaudeCode], claude).unwrap();
    std::fs::write(&paths[&Codex], rest).unwrap();
    let first = catalog(&paths);
    assert_eq!(
        first
            .servers
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["github", "other", "runner"]
    );
    let github = &first.servers[0];
    assert!(github.clients[&ClaudeCode].registered);
    assert!(!github.clients[&Codex].registered);
    assert!(!github.clients[&Trae].registered);
    assert!(first.servers[2].clients[&ClaudeCode].registered);
    copy_at(&paths, ClaudeCode, Codex, "github").unwrap();
    let copied = std::fs::read_to_string(&paths[&Codex]).unwrap();
    assert!(copied.starts_with(rest), "{copied}");
    assert!(copied.contains("[mcp_servers.github]"));
    assert_eq!(
        std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(),
        claude
    );
    let conflicting = format!(
        "{rest}\n[mcp_servers.github]\ncommand = 'different'\nstartup_timeout_sec = 90 # keep\n"
    );
    std::fs::write(&paths[&Codex], &conflicting).unwrap();
    let second = catalog(&paths);
    assert!(second.servers[0].clients[&ClaudeCode].conflicting);
    assert!(second.servers[0].clients[&Codex].conflicting);
    edit_at(
        &paths,
        ClaudeCode,
        "github",
        r#"{"command":"fixed","extra":42}"#,
        &[Codex],
    )
    .unwrap();
    let after = std::fs::read_to_string(&paths[&Codex]).unwrap();
    assert!(after.starts_with(rest));
    assert!(after.contains("startup_timeout_sec = 90 # keep\n"));
    assert!(!catalog(&paths).servers[0].clients[&Codex].conflicting);
    let after_claude = std::fs::read_to_string(&paths[&ClaudeCode]).unwrap();
    assert_eq!(
        after_claude,
        claude.replace(
            r#"{"command":"gh-mcp"}"#,
            "{\n  \"command\": \"fixed\",\n  \"extra\": 42\n}"
        )
    );
    for client in [ClaudeCode, Codex] {
        write_entry_at(&paths[&client], client, "github", None, false).unwrap();
    }
    assert_eq!(
        catalog(&paths)
            .servers
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["other", "runner"]
    );
    assert!(std::fs::read_to_string(&paths[&Codex])
        .unwrap()
        .starts_with(rest));
    assert!(catalog(&paths).servers[1].clients[&ClaudeCode].registered);
}

#[test]
fn copying_onto_existing_entry_keeps_unmodelled_keys_and_switches_transport() {
    use super::test_clients::*;
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    std::fs::write(&paths[&ClaudeCode], r#"{"mcpServers":{"github":{"type":"http","url":"https://example.test","headers":{"Authorization":"secret"}}}}"#).unwrap();
    let rest = "# untouched\nmodel = 'old'\n\n";
    std::fs::write(&paths[&Codex], format!("{rest}[mcp_servers.github]\ncommand = 'old'\nargs = ['a']\nenv = {{ KEY = 'value' }}\nstartup_timeout_sec = 60 # keep\nenabled = false\ncwd = '/tmp'\n")).unwrap();
    copy_at(&paths, ClaudeCode, Codex, "github").unwrap();
    let after = std::fs::read_to_string(&paths[&Codex]).unwrap();
    assert!(after.starts_with(rest));
    assert!(after.contains("startup_timeout_sec = 60 # keep\nenabled = false\ncwd = '/tmp'\n"));
    let entries = read_entries_at(&paths[&Codex], Codex).unwrap();
    assert_eq!(
        entries["github"].definition(),
        read_entries_at(&paths[&ClaudeCode], ClaudeCode).unwrap()["github"].definition()
    );
    assert!(!after.contains("command ="));
    assert!(!after.contains("env ="));
    copy_at(&paths, Codex, Trae, "github").unwrap();
    assert_eq!(
        read_entries_at(&paths[&Trae], Trae).unwrap()["github"].definition(),
        entries["github"].definition()
    );
}

#[test]
fn invalid_and_untranslatable_edits_validate_before_any_write() {
    use super::test_clients::*;
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    let json = r#"{"mcpServers":{"github":{"command":"old"}}}"#;
    let toml = "[mcp_servers.github]\ncommand = 'old'\n";
    std::fs::write(&paths[&ClaudeCode], json).unwrap();
    std::fs::write(&paths[&Codex], toml).unwrap();
    for invalid in ["{", "[]", r#"{"type":"sse","url":"https://example.test"}"#] {
        assert!(edit_at(&paths, ClaudeCode, "github", invalid, &[Codex]).is_err());
        assert_eq!(std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(), json);
        assert_eq!(std::fs::read_to_string(&paths[&Codex]).unwrap(), toml);
    }
    for invalid in [
        "[",
        "[mcp_servers.renamed]\ncommand = 'new'",
        "[mcp_servers.github]\ncommand = 'new'\n[other]\nx = 1",
    ] {
        assert!(edit_at(&paths, Codex, "github", invalid, &[ClaudeCode]).is_err());
        assert_eq!(std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(), json);
        assert_eq!(std::fs::read_to_string(&paths[&Codex]).unwrap(), toml);
    }
    let sse = r#"{"type":"sse","url":"https://example.test"}"#;
    edit_at(&paths, ClaudeCode, "github", sse, &[]).unwrap();
    let entry = catalog(&paths).servers.remove(0);
    assert!(entry.clients[&ClaudeCode].registered);
    assert!(entry.clients[&ClaudeCode].definition.is_none());
    assert!(copy_at(&paths, ClaudeCode, Codex, "github")
        .unwrap_err()
        .to_string()
        .contains("CLI"));
    assert_eq!(std::fs::read_to_string(&paths[&Codex]).unwrap(), toml);
}

#[test]
fn malformed_client_is_reported_on_every_row_and_independent_edits_continue() {
    use super::test_clients::*;
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    std::fs::write(&paths[&ClaudeCode], "{ broken").unwrap();
    std::fs::write(
        &paths[&Codex],
        "[mcp_servers.github]\ncommand = 'old'\n[mcp_servers.other]\ncommand = 'other'\n",
    )
    .unwrap();
    let catalog = catalog(&paths);
    assert!(catalog.servers.iter().all(|e| e.clients[&ClaudeCode]
        .error
        .as_ref()
        .unwrap()
        .contains(paths[&ClaudeCode].to_str().unwrap())));
    let error = edit_at(
        &paths,
        ClaudeCode,
        "github",
        r#"{"command":"new"}"#,
        &[Codex, Trae],
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains(paths[&ClaudeCode].to_str().unwrap()));
    assert_eq!(
        std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(),
        "{ broken"
    );
    for client in [Codex, Trae] {
        assert_eq!(
            read_entries_at(&paths[&client], client).unwrap()["github"].definition(),
            Some(McpServerDefinition::Stdio {
                command: "new".into(),
                args: vec![],
                env: BTreeMap::new()
            })
        );
    }
}

#[test]
fn last_removal_preserves_parent_and_every_unowned_byte() {
    use super::test_clients::*;
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    let json = "{ \"before\":1e2, \"mcpServers\" : {  \"runner\" : {\"command\":\"old\"}  }, \"after\":\"\\u0061\" }\n";
    std::fs::write(&paths[&ClaudeCode], json).unwrap();
    claude_code_write_at(&paths[&ClaudeCode]).unwrap();
    assert_eq!(
        std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(),
        json.replace("\"runner\" : {\"command\":\"old\"}", "")
    );
    let toml = "# before\nmodel = 'gpt-5'\n\n# parent comment\n[mcp_servers] # keep parent\n\n[mcp_servers.runner]\ncommand = 'old'\n\n[after]\nflag = true\n";
    std::fs::write(&paths[&Codex], toml).unwrap();
    codex_write_at(&paths[&Codex]).unwrap();
    assert_eq!(
        std::fs::read_to_string(&paths[&Codex]).unwrap(),
        toml.replace("\n[mcp_servers.runner]\ncommand = 'old'\n", "")
    );
}

#[test]
fn json_member_splicing_handles_first_middle_last_and_escaped_names() {
    let raw = r#" { "a": {"s":"x,}[]\""}, "b\"x": [1,2], "c": 1e2 } "#;
    for (name, expected) in [
        ("a", r#" {  "b\"x": [1,2], "c": 1e2 } "#),
        ("b\"x", r#" { "a": {"s":"x,}[]\""},  "c": 1e2 } "#),
        ("c", r#" { "a": {"s":"x,}[]\""}, "b\"x": [1,2]  } "#),
    ] {
        let edited = replace_json_member(raw, name, None).unwrap();
        assert_eq!(edited, expected);
        serde_json::from_str::<serde_json::Value>(&edited).unwrap();
    }
}

#[test]
fn toml_edit_roundtrips_named_table_subtables_and_extra_keys() {
    use super::test_clients::*;
    let text = "[mcp_servers.\"a.b\"]\ncommand = 'mcp'\nstartup_timeout_sec = 20\n\n[mcp_servers.\"a.b\".env]\nTOKEN = 'secret'\n";
    let parsed = parse_native(Codex, "a.b", text).unwrap();
    assert_eq!(parsed.text("a.b"), text);
    assert_eq!(
        parse_native(Trae, "a.b", &parsed.text("a.b"))
            .unwrap()
            .definition(),
        parsed.definition()
    );
    assert!(
        McpServerDefinition::from_claude(&json!({"type":"ws","url":"wss://example.test"}))
            .is_none()
    );
    assert!(McpServerDefinition::from_claude(&json!({"extra": true})).is_none());
}

#[test]
fn copilot_status_and_removal_preserve_other_servers_and_formatting() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".copilot/mcp-config.json");
    let existing = "{ \"before\":1e2, \"mcpServers\" : { \"runner\" : {\"type\":\"local\",\"command\":\"/runner-mcp\",\"args\":[],\"tools\":[\"*\"]}, \"other\" : {\"type\":\"local\",\"command\":\"other-mcp\",\"tools\":[\"read\"]} }, \"after\":\"\\u0061\" }\n";
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, existing).unwrap();
    assert!(
        copilot_status_at(&path, "/runner-mcp")
            .unwrap()
            .matches_current
    );
    assert!(!copilot_status_at(&path, "/other").unwrap().matches_current);
    copilot_write_at(&path).unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(value["mcpServers"].get("runner").is_none());
    assert_eq!(value["mcpServers"]["other"]["tools"], json!(["read"]));
}

#[test]
fn copilot_client_identity_and_cross_client_copy_use_json_local_entries() {
    use super::test_clients::*;
    assert_eq!(McpClientId::parse("copilot").unwrap(), Copilot);
    assert_eq!(Copilot.runtime(), crate::model::Runtime::Copilot);
    assert_eq!(
        McpClientId::for_runtime(crate::model::Runtime::Copilot),
        Some(Copilot)
    );
    assert_eq!(Copilot.label(), "GitHub Copilot CLI");
    assert_eq!(Copilot.config_file(), "~/.copilot/mcp-config.json");
    assert_eq!(Copilot.entry_key("runner"), "mcpServers.runner");
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    std::fs::write(
        &paths[&ClaudeCode],
        r#"{"mcpServers":{"other":{"type":"stdio","command":"other-mcp","args":["--stdio"]}}}"#,
    )
    .unwrap();
    copy_at(&paths, ClaudeCode, Copilot, "other").unwrap();
    let entry = read_entries_at(&paths[&Copilot], Copilot)
        .unwrap()
        .remove("other")
        .unwrap();
    let native: serde_json::Value = serde_json::from_str(&entry.text("other")).unwrap();
    assert_eq!(native["type"], "local");
    assert_eq!(native["tools"], json!(["*"]));
    assert!(entry.definition().is_some());
    copy_at(&paths, Copilot, Codex, "other").unwrap();
    assert_eq!(
        read_entries_at(&paths[&Codex], Codex).unwrap()["other"].definition(),
        entry.definition()
    );
    assert_eq!(
        catalog(&paths).servers[0].clients.len(),
        McpClientId::all().len()
    );
}

#[test]
fn antigravity_client_reads_an_empty_file_and_copies_in_agys_stdio_shape() {
    use super::test_clients::*;
    assert_eq!(McpClientId::parse("antigravity").unwrap(), Antigravity);
    assert_eq!(Antigravity.runtime(), crate::model::Runtime::Antigravity);
    assert_eq!(
        McpClientId::for_runtime(crate::model::Runtime::Antigravity),
        Some(Antigravity)
    );
    assert_eq!(Antigravity.key(), "antigravity");
    assert_eq!(Antigravity.label(), "Antigravity CLI");
    assert_eq!(
        Antigravity.config_file(),
        "~/.gemini/config/mcp_config.json"
    );
    assert_eq!(Antigravity.entry_key("runner"), "mcpServers.runner");
    assert_eq!(
        Antigravity.config_path(Path::new("/home/me")),
        Path::new("/home/me/.gemini/config/mcp_config.json")
    );

    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    // agy ships the file at 0 bytes.
    std::fs::write(&paths[&Antigravity], "").unwrap();
    assert!(read_entries_at(&paths[&Antigravity], Antigravity)
        .unwrap()
        .is_empty());
    std::fs::write(
            &paths[&ClaudeCode],
            r#"{"mcpServers":{"fs":{"type":"stdio","command":"fs-mcp","args":["/work"],"env":{"TOKEN":"x"}},"bare":{"command":"bare-mcp"},"web":{"type":"http","url":"https://example.com/mcp"}}}"#,
        )
        .unwrap();
    assert!(catalog(&paths).servers.iter().all(|server| server
        .clients
        .get(&Antigravity)
        .is_some_and(|entry| entry.error.is_none())));

    copy_at(&paths, ClaudeCode, Antigravity, "fs").unwrap();
    copy_at(&paths, ClaudeCode, Antigravity, "bare").unwrap();
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&paths[&Antigravity]).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_string(&written).unwrap(),
        r#"{"mcpServers":{"fs":{"args":["/work"],"command":"fs-mcp","disabled":false,"env":{"TOKEN":"x"}},"bare":{"command":"bare-mcp","disabled":false}}}"#
    );
    let entries = read_entries_at(&paths[&Antigravity], Antigravity).unwrap();
    let claude = read_entries_at(&paths[&ClaudeCode], ClaudeCode).unwrap();
    assert_eq!(entries["fs"].definition(), claude["fs"].definition());
    assert!(
        !catalog(&paths)
            .servers
            .iter()
            .find(|server| server.name == "fs")
            .unwrap()
            .clients[&Antigravity]
            .conflicting
    );

    let refused = copy_at(&paths, ClaudeCode, Antigravity, "web").unwrap_err();
    assert!(refused.to_string().contains("agy mcp add"), "{refused}");
    assert!(!read_entries_at(&paths[&Antigravity], Antigravity)
        .unwrap()
        .contains_key("web"));

    copy_at(&paths, Antigravity, Codex, "fs").unwrap();
    assert_eq!(
        read_entries_at(&paths[&Codex], Codex).unwrap()["fs"].definition(),
        claude["fs"].definition()
    );
    mcp_remove_server_at(&paths, Antigravity, "bare");
    assert!(!read_entries_at(&paths[&Antigravity], Antigravity)
        .unwrap()
        .contains_key("bare"));
}

#[test]
fn antigravity_translated_edits_and_copies_keep_an_existing_server_disabled() {
    use super::test_clients::*;
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    std::fs::write(
        &paths[&ClaudeCode],
        r#"{"mcpServers":{"fs":{"command":"old"}}}"#,
    )
    .unwrap();
    std::fs::write(
        &paths[&Antigravity],
        r#"{"mcpServers":{"fs":{"command":"old","disabled":true,"custom":123}}}"#,
    )
    .unwrap();

    edit_at(
        &paths,
        ClaudeCode,
        "fs",
        r#"{"command":"edited"}"#,
        &[Antigravity],
    )
    .unwrap();
    let read_agy = || -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(&paths[&Antigravity]).unwrap()).unwrap()
    };
    assert_eq!(
        read_agy()["mcpServers"]["fs"],
        json!({"command":"edited","disabled":true,"custom":123})
    );

    std::fs::write(
        &paths[&ClaudeCode],
        r#"{"mcpServers":{"fs":{"command":"copied"}}}"#,
    )
    .unwrap();
    copy_at(&paths, ClaudeCode, Antigravity, "fs").unwrap();
    assert_eq!(
        read_agy()["mcpServers"]["fs"],
        json!({"command":"copied","disabled":true,"custom":123})
    );

    edit_at(
        &paths,
        Antigravity,
        "fs",
        r#"{"command":"copied","disabled":false,"custom":123}"#,
        &[],
    )
    .unwrap();
    assert_eq!(read_agy()["mcpServers"]["fs"]["disabled"], false);
}

#[test]
fn an_http_edit_that_includes_antigravity_writes_no_file() {
    use super::test_clients::*;
    let dir = TempDir::new().unwrap();
    let paths = paths(&dir);
    let claude = r#"{"mcpServers":{"web":{"type":"http","url":"https://old.example/mcp"}}}"#;
    let codex = "[mcp_servers.web]\nurl = 'https://old.example/mcp'\n";
    let agy = r#"{"mcpServers":{"fs":{"command":"fs-mcp","disabled":false}}}"#;
    std::fs::write(&paths[&ClaudeCode], claude).unwrap();
    std::fs::write(&paths[&Codex], codex).unwrap();
    std::fs::write(&paths[&Antigravity], agy).unwrap();
    let text = r#"{"type":"http","url":"https://new.example/mcp"}"#;

    let error = validate_mcp_edit(ClaudeCode, "web", text, &[Codex, Antigravity]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Turn off Also update for Antigravity CLI"),
        "{error}"
    );
    assert!(edit_at(&paths, ClaudeCode, "web", text, &[Codex, Antigravity]).is_err());
    assert_eq!(
        std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(),
        claude
    );
    assert_eq!(std::fs::read_to_string(&paths[&Codex]).unwrap(), codex);
    assert_eq!(std::fs::read_to_string(&paths[&Antigravity]).unwrap(), agy);

    edit_at(&paths, ClaudeCode, "web", text, &[Codex]).unwrap();
    assert!(std::fs::read_to_string(&paths[&Codex])
        .unwrap()
        .contains("https://new.example/mcp"));
    assert_eq!(std::fs::read_to_string(&paths[&Antigravity]).unwrap(), agy);
}

fn mcp_remove_server_at(paths: &BTreeMap<McpClientId, PathBuf>, client: McpClientId, name: &str) {
    write_entry_at(&paths[&client], client, name, None, false).unwrap();
}

#[test]
fn claude_code_status_false_when_file_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".claude.json");
    assert!(
        !claude_code_status_at(&path, "/test/runner")
            .unwrap()
            .registered
    );
}

#[test]
fn claude_code_status_and_removal_preserve_other_servers_and_top_level_keys() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".claude.json");
    std::fs::write(
            &path,
            r#"{"mcpServers":{"runner":{"command":"/test/runner-mcp"},"github":{"command":"gh-mcp","args":[]}},"theme":"dark"}"#,
        )
        .unwrap();
    let status = claude_code_status_at(&path, "/test/runner-mcp").unwrap();
    assert!(status.registered && status.matches_current);
    assert!(
        !claude_code_status_at(&path, "/other/runner-mcp")
            .unwrap()
            .matches_current
    );

    claude_code_write_at(&path).unwrap();
    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(after["mcpServers"].get("runner").is_none());
    assert_eq!(after["mcpServers"]["github"]["command"], "gh-mcp");
    assert_eq!(after["theme"], "dark");
}

#[test]
fn integration_removal_requires_an_exact_command_without_arguments() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".claude.json");
    let original = r#"{"mcpServers":{"runner":{"command":"/test/runner-mcp","args":["--stdio"]}}}"#;
    std::fs::write(&path, original).unwrap();

    assert!(!remove_runner_entry(McpClientId::ClaudeCode, &path, "/test/runner-mcp").unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}

#[cfg(windows)]
#[test]
fn windows_bridge_path_matches_json_and_toml_registrations() {
    let dir = TempDir::new().unwrap();
    let binary = PathBuf::from(r"C:\Users\Agent User\AppData\Roaming\com.wycstudios.runner")
        .join("bin")
        .join(runner_core::cli_install::MCP_DEST_BIN_NAME);
    let binary = binary.to_str().unwrap();

    let claude = dir.path().join(".claude.json");
    std::fs::write(
        &claude,
        serde_json::to_string(&json!({
            "mcpServers": {"runner": {"command": binary}}
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(
        claude_code_status_at(&claude, binary)
            .unwrap()
            .matches_current
    );

    let codex = dir.path().join("config.toml");
    std::fs::write(
        &codex,
        format!("[mcp_servers.runner]\ncommand = '{binary}'\n"),
    )
    .unwrap();
    assert!(codex_status_at(&codex, binary).unwrap().matches_current);
}

#[test]
fn claude_code_removal_errors_on_malformed_json_without_overwriting() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".claude.json");
    std::fs::write(&path, "{ not valid json").unwrap();
    let err = claude_code_write_at(&path).unwrap_err();
    assert!(err.to_string().contains("parse"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not valid json");
}

#[test]
fn codex_status_false_when_file_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".codex").join("config.toml");
    assert!(
        !codex_status_at(&path, "/test/runner-mcp")
            .unwrap()
            .registered
    );
}

#[test]
fn codex_status_and_removal_preserve_other_tables_and_top_level_keys() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
            &path,
            "model = \"gpt-5\"\n\n[mcp_servers.runner]\ncommand = \"/test/runner-mcp\"\n\n[mcp_servers.github]\ncommand = \"gh-mcp\"\nargs = []\n",
        )
        .unwrap();
    let status = codex_status_at(&path, "/test/runner-mcp").unwrap();
    assert!(status.registered && status.matches_current);
    assert!(
        !codex_status_at(&path, "/other/runner-mcp")
            .unwrap()
            .matches_current
    );

    codex_write_at(&path).unwrap();
    let after: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert!(after["mcp_servers"].get("runner").is_none());
    assert_eq!(
        after["mcp_servers"]["github"]["command"].as_str(),
        Some("gh-mcp")
    );
    assert_eq!(after["model"].as_str(), Some("gpt-5"));
}

#[test]
fn trae_write_preserves_auth_hooks_and_other_servers() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("traecli.toml");
    std::fs::write(
            &path,
            "auth_token = \"secret\"\n\n[hooks.state]\nstop = \"trusted\"\n\n[mcp_servers.runner]\ncommand = \"/test/runner-mcp\"\n\n[mcp_servers.github]\ncommand = \"gh-mcp\"\nargs = []\n",
        )
        .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

    codex_write_at(&path).unwrap();
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640,
    );
    let after: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert_eq!(after["auth_token"].as_str(), Some("secret"));
    assert_eq!(after["hooks"]["state"]["stop"].as_str(), Some("trusted"));
    assert_eq!(
        after["mcp_servers"]["github"]["command"].as_str(),
        Some("gh-mcp")
    );
    assert!(after["mcp_servers"].get("runner").is_none());
}

#[test]
fn codex_write_errors_on_malformed_toml_without_overwriting() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[unclosed-table").unwrap();

    let err = codex_write_at(&path).unwrap_err();

    assert!(err.to_string().contains("parse"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[unclosed-table");
}

#[test]
fn codex_write_errors_when_mcp_servers_is_not_table() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "mcp_servers = \"bad\"\n").unwrap();

    let err = codex_write_at(&path).unwrap_err();

    assert!(err.to_string().contains("mcp_servers is not a table"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "mcp_servers = \"bad\"\n"
    );
}
