use super::*;

#[test]
fn native_identity_is_captured_and_only_used_for_resume() {
    let first = OpenCode.resume_plan(None);
    assert!(first.args.is_empty());
    assert!(first.assigned_key.is_none());
    assert!(!first.resuming);
    assert!(matches!(OpenCode.key_capture(), KeyCapture::Hook));
    let native_key = "ses_ecc290331ffeqzayUqWErOjpRo";
    let resumed = OpenCode.resume_plan(Some(native_key));
    assert!(resumed.resuming);
    assert_eq!(resumed.args, strings(&["--session", native_key]));
    assert_eq!(resumed.assigned_key.as_deref(), Some(native_key));
    let invalid = OpenCode.resume_plan(Some("../../ses_unsafe"));
    assert!(!invalid.resuming);
    assert!(invalid.args.is_empty());
    assert!(invalid.assigned_key.is_none());
    let args = strings(&[
        "--continue",
        "--session=ses_other",
        "--prompt",
        "old",
        "--server",
        "http://other",
        "--keep",
    ]);
    assert_eq!(OpenCode.launch_role_args(&args), strings(&["--keep"]));
    assert_eq!(
        OpenCode.first_turn_argv(Some("中文\nspaces ' \" $()")),
        strings(&["--prompt", "中文\nspaces ' \" $()"])
    );
    assert!(OpenCode.fork_plan("ses_a", "label").is_none());
}
#[test]
fn attended_permissions_and_mission_resume_keep_native_denies() {
    use crate::router::runtime::PermissionMode;
    let args = strings(&["--auto=false", "--auto", "--keep"]);
    assert_eq!(PERMISSIONS.strip(&args), strings(&["--keep"]));
    assert_eq!(
        PERMISSIONS.apply_mission(&args),
        strings(&["--keep", "--auto"])
    );
    assert_eq!(
        PERMISSIONS.infer(&strings(&["--auto=true"])),
        PermissionMode::Bypass
    );
}
#[test]
fn exact_native_history_lookup_never_selects_newest_or_child() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("opencode/opencode.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE session_v2(id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT); INSERT INTO session_v2 VALUES ('ses_first',NULL,'/same'),('ses_latest',NULL,'/same'),('ses_child','ses_first','/same');").unwrap();
    let env = HashMap::from([("XDG_DATA_HOME".into(), home.path().display().to_string())]);
    let context = ProbeContext {
        cwd: Some("/same"),
        role_env: &env,
    };
    assert_eq!(
        OpenCode.conversation_exists("ses_first", &context),
        Some(true)
    );
    assert_eq!(
        OpenCode.conversation_exists("ses_missing", &context),
        Some(false)
    );
    db.execute("DELETE FROM session_v2 WHERE id = 'ses_first'", [])
        .unwrap();
    assert_eq!(
        OpenCode.conversation_exists("ses_first", &context),
        Some(false)
    );
}
#[test]
fn dynamic_models_keep_full_ids_and_model_specific_variants() {
    use crate::runtime_status::models::Reason;
    assert_eq!(models::parse(br#"{"data":[]}"#), Err(Reason::EmptyCatalog));
    for raw in [
        "{}",
        "not json",
        r#"{"data":[{"id":"bad model","providerID":"p"}]}"#,
        r#"{"data":[{"id":"model","providerID":"p","variants":{}}]}"#,
        r#"{"data":[{"id":"model","providerID":"p","variants":[{"id":"bad#variant"}]}]}"#,
        r#"{"data":[{"id":"model","providerID":"p","variants":[{}]}]}"#,
    ] {
        assert_eq!(models::parse(raw.as_bytes()), Err(Reason::InvalidOutput));
    }
    let parsed = models::parse(br#"{"data":[{"id":"model","providerID":"p","enabled":true},{"id":"model","providerID":"p"},{"id":"offline","providerID":"p","enabled":false}]}"#).unwrap();
    assert_eq!(parsed.models.len(), 1);
    assert_eq!(parsed.models[0].value, "p/model");
    assert_eq!(parsed.models[0].supported_efforts, Some(Vec::new()));

    let parsed = models::parse(br#"{"data":[
        {"id":"reasoner","providerID":"p","variants":[{"id":"low"},{"id":"high"},{"id":"high"}]},
        {"id":"custom/nested","providerID":"q","variants":[{"id":"deep","settings":{"reasoningEffort":"high"}}]},
        {"id":"ling-3.1-flash-free","providerID":"opencode","variants":[]}
    ]}"#).unwrap();
    let mut entry = OpenCode.catalog().unwrap().into_entry();
    assert!(entry.capabilities.effort_needs_launch_model);
    entry.efforts = OpenCode.discovered_efforts(&parsed).unwrap();
    entry.models = parsed.models;
    let efforts = |model| {
        entry
            .efforts_for_model(model)
            .into_iter()
            .map(|option| option.value)
            .collect::<Vec<_>>()
    };
    assert_eq!(efforts("p/reasoner"), strings(&["", "low", "high"]));
    assert_eq!(efforts("q/custom/nested"), strings(&["", "deep"]));
    assert_eq!(efforts("opencode/ling-3.1-flash-free"), strings(&[""]));
    assert_eq!(efforts(""), strings(&[""]));
}

#[test]
fn model_variant_is_configured_only_for_fresh_explicit_selection() {
    let root = tempfile::tempdir().unwrap();
    let env = BTreeMap::from([("OPENCODE_CONFIG_CONTENT".into(), r#"{"permissions":[{"action":"edit","effect":"deny","resource":"*"}],"plugins":["foreign"]}"#.into())]);
    let mut ctx = LaunchContext {
        default_model: None,
        env: &env,
        cwd: None,
        role_args: &[],
        app_data_dir: root.path(),
        session_id: "runner-session",
        resuming: false,
        mission: false,
        model: Some("p/custom/nested"),
        effort: Some("deep"),
        codex_speed: None,
        system_prompt: None,
        first_turn: Some("first turn"),
    };
    let document =
        config::parse(&OpenCode.launch_context_env(&ctx)["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(document["model"], "p/custom/nested");
    assert_eq!(document["permissions"][0]["effect"], "deny");
    assert_eq!(document["plugins"][0], "foreign");
    assert_eq!(
        document["plugins"][1]["options"]["model"],
        json!({"providerID":"p","id":"custom/nested","variant":"deep"})
    );
    ctx.resuming = true;
    let document =
        config::parse(&OpenCode.launch_context_env(&ctx)["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert!(document.get("model").is_none());
    assert_eq!(document["plugins"], json!(["foreign"]));
    ctx.resuming = false;
    ctx.effort = None;
    let document =
        config::parse(&OpenCode.launch_context_env(&ctx)["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(document["plugins"], json!(["foreign"]));
    ctx.effort = Some("high");
    ctx.model = None;
    let document =
        config::parse(&OpenCode.launch_context_env(&ctx)["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(document["plugins"], json!(["foreign"]));
}

#[test]
#[cfg(unix)]
fn model_query_waits_for_settlement_and_reaps_its_private_server() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("opencode");
    std::fs::write(
        &executable,
        r#"#!/bin/sh
test -n "$OPENCODE_PASSWORD" || exit 2
case "$1" in
serve)
  echo $$ > "$SMOKE_ROOT/pid"
  printf '{"url":"http://127.0.0.1:12345"}\n'
  exec /bin/sleep 30
  ;;
api)
  test "$2" = --server || exit 3
  test "$3" = http://127.0.0.1:12345 || exit 4
  count=0
  test ! -f "$SMOKE_ROOT/count" || read -r count < "$SMOKE_ROOT/count"
  count=$((count + 1))
  echo "$count" > "$SMOKE_ROOT/count"
  if test "$count" -eq 1 || test "$EMPTY_FOREVER" = yes; then
    printf '{"data":[]}\n'
  else
    printf '{"data":[{"providerID":"test","id":"settled","enabled":true}]}\n'
  fi
  ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut env = crate::shell_path::LoginShellEnv::default();
    env.vars
        .insert("SMOKE_ROOT".into(), root.path().display().to_string());
    // Process startup competes with the full suite; use the production success budget.
    let result = models::query(executable.to_str().unwrap(), &env).unwrap();
    assert_eq!(result.models[0].value, "test/settled");
    assert_eq!(
        std::fs::read_to_string(root.path().join("count"))
            .unwrap()
            .trim(),
        "2"
    );
    let pid: i32 = std::fs::read_to_string(root.path().join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );

    std::fs::remove_file(root.path().join("pid")).unwrap();
    std::fs::remove_file(root.path().join("count")).unwrap();
    env.vars.insert("EMPTY_FOREVER".into(), "yes".into());
    // Expiry between polls reports EmptyCatalog; expiry inside a query reports Timeout.
    assert!(matches!(
        models::query_with_timeout(executable.to_str().unwrap(), &env, Duration::from_secs(3)),
        Err(crate::runtime_status::models::Reason::EmptyCatalog
            | crate::runtime_status::models::Reason::Timeout)
    ));
    assert!(
        std::fs::read_to_string(root.path().join("count"))
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap()
            > 0
    );
    let pid: i32 = std::fs::read_to_string(root.path().join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
#[test]
fn classify_startup_error_matches_v1_standalone_failures_only() {
    let adapter = crate::runtimes::adapter(Runtime::OpenCode);
    for v1_output in [
        "opencode [project]\n\nCommands:\n  opencode [project]  start opencode tui\n\nOptions:\n  -h, --help     Show help\n  -v, --version  Show version\n\nUnknown argument: standalone\n",
        "\x1b[1mUsage:\x1b[22m opencode [options]\n\x1b[1mOptions:\x1b[22m\n  -m, --model  Model to use\n",
        "error: unknown option '--standalone'\n",
    ] {
        let classified = adapter.classify_startup_error(v1_output).unwrap();
        assert_eq!(classified.runtime, Runtime::OpenCode);
        assert!(classified.message.contains("--standalone"));
        assert!(classified.message.contains("OpenCode v2 (2.1.0 or newer)"));
    }

    for non_v1_output in [
        "Error: API key is missing for provider anthropic",
        "Usage: opencode [project]\nOptions:\n  --standalone  Run isolated private server\n",
        "error: unknown option '--bogus'\nUsage: opencode [project]\nOptions:\n  --standalone   Run isolated private server\n",
        "",
    ] {
        assert!(
            adapter.classify_startup_error(non_v1_output).is_none(),
            "unexpected classification for {non_v1_output:?}"
        );
    }
}

#[test]
fn jsonc_edits_preserve_comments_unknown_fields_and_disabled_servers() {
    use crate::ops::mcp::{McpServerDefinition, NativeEntry};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("opencode.jsonc");
    let raw = "{\n// 中文 config\n\"model\":\"p/model\",\"other\":1e2,\"mcp\":{\"timeout\":{\"startup\":1000},\"servers\":{\n// keep\n\"old\":{\"type\":\"local\",\"command\":[\"old\"],\"disabled\":true,\"custom\":7},\n\"foreign\":{\"type\":\"remote\",\"url\":\"https://foreign\"},\n}},\n}";
    std::fs::write(&path, raw).unwrap();
    let entry = mcp::entry(&McpServerDefinition::Stdio {
        command: "new".into(),
        args: strings(&["空 格"]),
        env: Default::default(),
    })
    .unwrap();
    mcp::write(&path, "old", Some(entry), true).unwrap();
    let changed = std::fs::read_to_string(&path).unwrap();
    assert!(changed.contains("// 中文 config"));
    assert!(changed.contains("// keep"));
    assert!(changed.contains("1e2"));
    let NativeEntry::Json(old, _) = &mcp::read(&path).unwrap()["old"] else {
        panic!()
    };
    assert_eq!(old["disabled"], true);
    assert_eq!(old["custom"], 7);
    assert_eq!(old["command"], json!(["new", "空 格"]));
    mcp::write(&path, "old", None, false).unwrap();
    assert!(mcp::read(&path).unwrap().contains_key("foreign"));
    assert!(!mcp::read(&path).unwrap().contains_key("old"));
    let before = std::fs::read(&path).unwrap();
    std::fs::write(&path, "{ invalid").unwrap();
    assert!(mcp::write(&path, "new", None, false).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ invalid");
    assert!(!before.is_empty());
    for raw in ["{\"a\":1,}", "{\"a\":1}", "{/* x */}"] {
        let inserted = config::replace(raw, "b", Some(&json!(2))).unwrap();
        assert_eq!(config::parse(&inserted).unwrap()["b"], 2);
        let removed = config::replace(&inserted, "b", None).unwrap();
        assert!(config::parse(&removed).unwrap().get("b").is_none());
    }
}

#[test]
fn native_cli_config_preserves_user_plugins_and_inline_overrides() {
    let root = tempfile::tempdir().unwrap();
    let native = root.path().join("opencode");
    std::fs::create_dir(&native).unwrap();
    let raw = r#"{"plugins":["foreign", "-disabled"],"tabs":{"mode":"off"}}"#;
    std::fs::write(native.join("cli.json"), raw).unwrap();
    let mut env = BTreeMap::from([("XDG_CONFIG_HOME".into(), root.path().display().to_string())]);
    let document: Value =
        serde_json::from_str(&status::launch_config(root.path(), &env).unwrap()).unwrap();
    assert_eq!(document["plugins"][0], "foreign");
    assert_eq!(document["plugins"][1], "-disabled");
    assert_eq!(
        std::fs::read_to_string(native.join("cli.json")).unwrap(),
        raw
    );
    env.insert(
        "OPENCODE_CLI_CONFIG_CONTENT".into(),
        r#"{"plugins":["inline"],"tabs":{"mode":"off"}}"#.into(),
    );
    let document: Value =
        serde_json::from_str(&status::launch_config(root.path(), &env).unwrap()).unwrap();
    assert_eq!(document["plugins"].as_array().unwrap().len(), 2);
    assert_eq!(document["plugins"][0], "inline");
    assert_eq!(document["tabs"]["mode"], "off");
    env.insert("OPENCODE_CLI_CONFIG_CONTENT".into(), "bad".into());
    assert!(status::launch_config(root.path(), &env).is_err());
}

#[test]
fn inherited_inline_config_and_data_home_are_not_discarded() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("inherited-data/opencode/opencode.db");
    std::fs::create_dir_all(data.parent().unwrap()).unwrap();
    let db = rusqlite::Connection::open(data).unwrap();
    db.execute_batch(
        "CREATE TABLE session_v2(id TEXT PRIMARY KEY);INSERT INTO session_v2 VALUES ('ses_owned')",
    )
    .unwrap();
    crate::golden::with_config_env(BTreeMap::from([
        ("XDG_DATA_HOME",root.path().join("inherited-data").into_os_string()),
        ("XDG_CONFIG_HOME",root.path().join("inherited-config").into_os_string()),
        ("OPENCODE_CONFIG_CONTENT",r#"{"permissions":[{"action":"edit","resource":"*","effect":"deny"}],"model":"p/inherited"}"#.into()),
        ("OPENCODE_CLI_CONFIG_CONTENT",r#"{"plugins":["foreign"],"tabs":{"mode":"off"}}"#.into()),
    ]),|| {
        let env=config::effective_env(&Default::default());
        assert_eq!(config::parse(&env["OPENCODE_CONFIG_CONTENT"]).unwrap()["permissions"][0]["effect"],"deny");
        let cli: Value=serde_json::from_str(&status::launch_config(root.path(),&Default::default()).unwrap()).unwrap();assert_eq!(cli["plugins"][0],"foreign");assert_eq!(cli["tabs"]["mode"],"off");
        assert_eq!(OpenCode.conversation_exists("ses_owned",&ProbeContext { cwd:None,role_env:&Default::default() }),Some(true));
        let override_env=BTreeMap::from([("OPENCODE_CONFIG_CONTENT".into(),"{\"model\":\"p/role\"}".into())]);
        assert_eq!(config::parse(&config::effective_env(&override_env)["OPENCODE_CONFIG_CONTENT"]).unwrap()["model"],"p/role");
    });
}

#[test]
fn active_snapshots_rekey_home_and_ignore_children_stale_reload_and_generation() {
    use crate::session::hook_queue::TestHookRoute;
    use crate::session::state::agent::AgentEvent;
    let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "current".into());
    let mut watcher = status::Watcher::new(
        receiver,
        &SpawnSpec {
            args: strings(&["--session", "ses_first"]),
            ..Default::default()
        },
    );
    let mut keys = Vec::new();
    let mut drain = |watcher: &mut status::Watcher| {
        let mut events = Vec::new();
        watcher
            .drain_events(
                0,
                &mut |event| {
                    events.push(event);
                    Default::default()
                },
                &mut |key| keys.push(key),
            )
            .unwrap();
        events
    };
    let snapshot = |epoch, sequence, id| json!({"hook_event_name":"snapshot","epoch":epoch,"sequence":sequence,"session_id":id});
    drop(route.admit(snapshot(100, 1, "ses_first")).unwrap());
    assert_eq!(
        drain(&mut watcher),
        vec![
            AgentEvent::ProgramSessionStarted {
                conversation_changed: false,
                compacted: false,
            },
            AgentEvent::ProgramOutcomeUnavailable,
        ]
    );
    let mut child = snapshot(100, 2, "ses_child");
    child["child"] = json!(true);
    drop(route.admit(child).unwrap());
    drop(route.admit(snapshot(100, 3, "ses_next")).unwrap());
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramSessionStarted {
            conversation_changed: true,
            compacted: false,
        }]
    );
    // Out-of-order sequence is ignored.
    drop(route.admit(snapshot(100, 2, "ses_first")).unwrap());
    assert!(drain(&mut watcher).is_empty());
    // Higher epoch reloads and rekeys.
    drop(route.admit(snapshot(200, 1, "ses_loaded")).unwrap());
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramSessionStarted {
            conversation_changed: true,
            compacted: false,
        }]
    );
    // Older epoch is ignored.
    drop(route.admit(snapshot(100, 99, "ses_first")).unwrap());
    assert!(drain(&mut watcher).is_empty());
    // Stale generation is rejected at admission.
    let mut stale = snapshot(300, 1, "ses_stale");
    stale["generation"] = json!("old");
    assert!(route.admit(stale).is_err());
    // Legacy state/outcome fields on snapshot never arm status hooks.
    let mut legacy = snapshot(200, 2, "ses_loaded");
    legacy["state"] = json!("idle");
    legacy["outcome"] = json!("interrupted");
    drop(route.admit(legacy).unwrap());
    assert!(drain(&mut watcher).is_empty());
    // Navigating to home clears key.
    drop(route.admit(snapshot(200, 3, "")).unwrap());
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramSessionStarted {
            conversation_changed: true,
            compacted: false,
        }]
    );
    assert_eq!(keys, ["ses_next", "ses_loaded", ""]);
}

#[test]
fn opencode_skills_use_xdg_and_shared_managed_root() {
    let home = tempfile::tempdir().unwrap();
    crate::golden::with_config_env(
        BTreeMap::from([("XDG_CONFIG_HOME", home.path().join("xdg").into_os_string())]),
        || {
            let roots = OpenCode
                .skill_roots_with_env(home.path(), None, &Default::default())
                .unwrap();
            assert_eq!(roots[0], home.path().join("xdg/opencode/skills"));
            assert_eq!(
                Runtime::OpenCode.managed_skill_root(),
                Some(".agents/skills")
            );
            assert_eq!(roots[1], home.path().join(".agents/skills"));
            assert!(OpenCode.skills().toggle.is_none());
        },
    );
}

#[test]
fn global_jsonc_defaults_and_dual_file_mcp_refusal_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let config_dir = root.path().join(".config/opencode");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("opencode.json"), r#"{"model":"p/json"}"#).unwrap();
    std::fs::write(
        config_dir.join("opencode.jsonc"),
        "{ // preferred\n\"model\":{\"providerID\":\"p\",\"model\":\"jsonc\"},}",
    )
    .unwrap();
    assert_eq!(
        OpenCode.native_defaults(root.path()).model.as_deref(),
        Some("p/jsonc")
    );
    assert!(mcp::read(&config_dir.join("opencode.jsonc"))
        .err()
        .unwrap()
        .to_string()
        .contains("one global file"));
    let env = BTreeMap::from([(
        "OPENCODE_CONFIG_CONTENT".into(),
        r#"{"model":"p/inline"}"#.into(),
    )]);
    assert_eq!(
        OpenCode
            .native_defaults_with_env(root.path(), &env)
            .model
            .as_deref(),
        Some("p/inline")
    );
}

#[cfg(unix)]
#[test]
fn mcp_symlink_keeps_target_and_unrelated_content() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("linked.jsonc");
    let path = root.path().join("opencode.jsonc");
    std::fs::write(&target, "{ // foreign config\n\"mcp\":{\"servers\":{}}}").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let entry = mcp::entry(&crate::ops::mcp::McpServerDefinition::Http {
        url: "https://example/mcp".into(),
        headers: Default::default(),
    })
    .unwrap();
    mcp::write(&path, "remote", Some(entry), false).unwrap();
    assert!(path.is_symlink());
    assert!(std::fs::read_to_string(&target)
        .unwrap()
        .contains("// foreign config"));
    assert!(mcp::read(&path).unwrap().contains_key("remote"));
}

#[test]
fn variant_plugin_preserves_history_children_and_later_native_choices() {
    use std::process::Command;
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping OpenCode variant stub: node is not on PATH");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let source = include_str!("model.mjs").replace(
        "import { Plugin } from \"@opencode/plugin\";",
        "const Plugin = { define: value => value };",
    );
    std::fs::write(root.path().join("plugin.mjs"), source).unwrap();
    std::fs::write(root.path().join("driver.mjs"), r#"
import assert from 'node:assert/strict';
import { setup } from './plugin.mjs';
const model = {providerID:'p',id:'model',variant:'deep'};
const now = Date.now() + 1000;
const sessions = new Map([
  ['old', {time:{created:0}}],
  ['child', {parentID:'root',time:{created:now}}],
  ['history', {time:{created:now}}],
  ['root', {time:{created:now}}],
  ['next', {time:{created:now}}],
]);
let hook;
let disposed = false;
let fail = true;
const changes = [];
const context = {options:{model},session:{
  hook:async (name, callback) => { assert.equal(name,'prompt');hook=callback;return {dispose:()=>{disposed=true;}}; },
  get:async ({sessionID}) => sessions.get(sessionID),
  context:async ({sessionID}) => sessionID==='history' ? [{}] : [],
  switchModel:async (input) => { if(fail) throw new Error('native resolution failed');changes.push(input); },
}};
const cleanup = await setup(context);
for (const sessionID of ['old','child','history']) await hook({sessionID});
assert.deepEqual(changes,[]);
await assert.rejects(hook({sessionID:'root'}), /native resolution failed/);
fail = false;
await Promise.all([hook({sessionID:'root'}),hook({sessionID:'root'})]);
assert.deepEqual(changes,[{sessionID:'root',model}]);
// Later prompts and another session in this TUI preserve native choices.
await hook({sessionID:'root'});
await hook({sessionID:'next'});
assert.equal(changes.length,1);
await cleanup();assert.equal(disposed,true);
hook = undefined;
await setup({...context,options:{}});
assert.equal(hook,undefined);
"#).unwrap();
    let output = Command::new("node")
        .arg(root.path().join("driver.mjs"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn tui_plugin_stub_tracks_exact_routes_coalesces_reports_and_cleans_up() {
    use std::process::Command;
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping OpenCode TUI stub: node is not on PATH");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let source = include_str!("tui.mjs")
        .replace(
            "import { Plugin } from \"@opencode/plugin/tui\";",
            "const Plugin = { define: value => value };",
        )
        .replace(
            "import { spawn } from \"node:child_process\";",
            "const spawn = (...args) => globalThis.fakeSpawn(...args);",
        );
    std::fs::write(root.path().join("plugin.mjs"), source).unwrap();
    std::fs::write(root.path().join("driver.mjs"),r#"
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { setup } from './plugin.mjs';
let poll;
let child;
let killed = false;
let cleared = false;
let now = 0;
let timeout;
const attempts = [];
const accepted = [];
globalThis.performance = {now:()=>now};
globalThis.setInterval = callback => { poll = callback; return 1; };
globalThis.clearInterval = () => { cleared = true; };
globalThis.setTimeout = callback => { timeout = callback; return 2; };
globalThis.clearTimeout = () => { timeout = undefined; };
globalThis.fakeSpawn = (executable, args, options) => {
  assert.equal(executable, 'reporter');
  assert.ok(args.includes('--acknowledge'));
  assert.equal(options.windowsHide, true);
  child = new EventEmitter(); child.kill = () => { killed = true; child.emit('close',null); };
  child.stdin = new EventEmitter(); child.stdin.end = text => attempts.push({args,payload:JSON.parse(text)});
  return child;
};
const ack = () => { accepted.push(attempts.at(-1).payload);child.emit('close',0); };
const tick = () => { now += 1000;poll(); };
process.env.RUNNER_HOOK_EXECUTABLE = 'reporter';
process.env.RUNNER_HOOK_GENERATION = 'generation';
let route = {type:'session', sessionID:'ses_first'};
const sessions = new Map([['ses_first', {time:{}}],['ses_next',{time:{created:1}}],['ses_child',{parentID:'ses_next',time:{created:1}}]]);
const context = { ui:{router:{current:()=>route}},data:{session:{get:id=>sessions.get(id)}} };
const cleanup = setup(context);ack();
assert.equal(accepted.at(-1).session_id,'ses_first');
assert.equal(accepted.at(-1).state,undefined);
assert.equal(accepted.at(-1).outcome,undefined);
// Unchanged route does not emit heartbeats or status snapshots.
const idleCount = attempts.length;
for (let n=0;n<6;n++) tick();
assert.equal(attempts.length,idleCount);
route = {type:'session',sessionID:'ses_next'};tick();
child.emit('close',1); // Lost identity report must retry current snapshot.
tick();ack();
assert.equal(accepted.at(-1).session_id,'ses_next');
assert.equal(accepted.at(-1).state,undefined);
route = {type:'session',sessionID:'ses_child'};tick();ack();
assert.equal(accepted.at(-1).session_id,'');
const beforeUnknown = attempts.length;
route = {type:'session',sessionID:'ses_unknown'};tick();assert.equal(attempts.length,beforeUnknown); // Still empty key -> no duplicate report.
sessions.set('ses_unknown',{time:{created:1,idle:42},outcome:'succeeded'});tick();ack();
assert.equal(accepted.at(-1).session_id,'ses_unknown');
assert.equal(accepted.at(-1).outcome,undefined);
route = {type:'home'};tick();ack();assert.equal(accepted.at(-1).session_id,'');
route = {type:'session',sessionID:'ses_first'};tick();
timeout();assert.ok(killed);tick();ack();assert.equal(accepted.at(-1).session_id,'ses_first');
// Three admission failures emit explicit bridge loss, then shut down telemetry.
route = {type:'home'};tick();child.emit('close',1);tick();child.emit('close',1);tick();child.emit('close',1);tick();
assert.ok(attempts.at(-1).args.includes('--bridge-unavailable'));ack();assert.ok(cleared);
cleanup();const total=attempts.length;tick();assert.equal(attempts.length,total);
"#).unwrap();
    let output = Command::new("node")
        .arg(root.path().join("driver.mjs"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}
