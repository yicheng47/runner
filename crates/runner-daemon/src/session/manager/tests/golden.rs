use super::*;
use crate::model::CodexSpeed;
use crate::router::runtime::PermissionMode;
use crate::runtimes::with_conversation_home;
use serde_json::{json, Value};

#[derive(Default)]
struct Normalizer {
    ids: Vec<String>,
}

impl Normalizer {
    fn text(&mut self, text: &str, root: &Path) -> String {
        let canonical_root = root.canonicalize().unwrap();
        let mut roots = [root, canonical_root.as_path()];
        roots.sort_by_key(|path| std::cmp::Reverse(path.as_os_str().len()));
        let mut text = text.to_string();
        for root in roots {
            text = text.replace(root.to_string_lossy().as_ref(), "<TMP>");
        }
        if let Some(home) = runner_core::app_paths::home_dir() {
            text = text.replace(&home.to_string_lossy().to_string(), "<HOME>");
        }
        let mut out = String::new();
        let mut offset = 0;
        while offset < text.len() {
            let rest = &text[offset..];
            let id = rest
                .get(..36)
                .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                .or_else(|| {
                    rest.get(..26).filter(|id| {
                        ulid::Ulid::from_string(id).is_ok()
                            && id
                                .bytes()
                                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                    })
                });
            if let Some(id) = id {
                let index = self
                    .ids
                    .iter()
                    .position(|prior| prior == id)
                    .unwrap_or_else(|| {
                        self.ids.push(id.to_string());
                        self.ids.len() - 1
                    });
                out.push_str(&format!("<ID{}>", index + 1));
                offset += id.len();
            } else {
                let c = rest.chars().next().unwrap();
                out.push(c);
                offset += c.len_utf8();
            }
        }
        out
    }

    fn spawn(&mut self, fake: &FakeRuntime, root: &Path) -> Value {
        let spec = fake.last_spawn_spec().unwrap();
        let mut env_keys: Vec<_> = spec.env.keys().cloned().collect();
        env_keys.sort();
        json!({"command": self.text(&spec.command, root),
            "args": spec.args.iter().map(|arg| self.text(arg, root)).collect::<Vec<_>>(),
            "env_keys": env_keys})
    }

    fn effects(
        &mut self,
        fake: &FakeRuntime,
        mgr: &SessionManager,
        key: &str,
        root: &Path,
    ) -> Value {
        let spec = fake.last_spawn_spec().unwrap();
        assert_eq!(spec.agent_runtime, Runtime::parse(key), "runtime {key}");
        let adapter = spec
            .agent_runtime
            .map(crate::runtimes::adapter)
            .unwrap_or(&crate::runtimes::NoAgent);
        let env: BTreeMap<_, _> = spec
            .env
            .iter()
            .map(|(name, value)| (name.clone(), self.text(value, root)))
            .collect();
        let watcher = spec
            .agent_runtime
            .filter(|_| {
                adapter
                    .status_hooks()
                    .is_some_and(|hooks| hooks.supported(cfg!(windows)))
                    && spec
                        .env
                        .contains_key(runner_core::protocol::hook::GENERATION_ENV)
            })
            .map(Runtime::key);
        // The fixture shares its agent home and app-data root; capture paths represent the home.
        let capture = match adapter.key_capture_for_spawn(&spec) {
            crate::runtimes::KeyCapture::RolloutScan {
                sessions_root: Some(path),
            } => {
                json!({"mechanism": "rollout-scan", "sessions_root": self.text(&path.to_string_lossy(), root).replace("<TMP>", "<HOME>")})
            }
            crate::runtimes::KeyCapture::LogTail => json!({"mechanism": "log-tail"}),
            crate::runtimes::KeyCapture::Hook => json!({"mechanism": "hook"}),
            _ => json!({"mechanism": "none"}),
        };
        let routes = Arc::new(crate::session::hook_queue::HookRoutes::default());
        let interrupt = adapter
            .status_hooks()
            .and_then(|hooks| {
                let generation = spec.env.get(runner_core::protocol::hook::GENERATION_ENV)?;
                let receiver = routes.register(
                    spec.agent_runtime?,
                    spec.session_id.clone(),
                    generation.clone(),
                );
                hooks.start_receiver(&spec, receiver)
            })
            .map(|_| {
                matches!(
                    spec.agent_runtime.unwrap(),
                    Runtime::ClaudeCode | Runtime::Copilot | Runtime::Antigravity
                )
            });
        let rollout = mgr.codex_capture_context(&spec.session_id).map(|ctx| {
            json!({
                "sessions_root": self.text(&ctx.sessions_root.to_string_lossy(), root).replace("<TMP>", "<HOME>"),
                "prompt_marker": ctx.prompt_marker.map(|marker| self.text(&marker, root))
            })
        });
        json!({"env": env, "codex_pending_turn": spec.pending_turn,
            "watcher": watcher, "interrupt": interrupt, "key_capture": capture, "rollout": rollout})
    }
}

fn configured_role(key: &str, root: &Path) -> Role {
    let mut configured = role("golden-agent", &["--keep"]);
    configured.runtime = key.into();
    configured.working_dir = Some(root.to_string_lossy().into_owned());
    configured.system_prompt = Some("GOLDEN_PERSONA".into());
    configured.model = Some(
        if key == "antigravity" {
            "gemini-3.8-flash"
        } else {
            "fixture-model"
        }
        .into(),
    );
    configured.effort = Some("High".into());
    configured.env.insert(
        "CODEX_HOME".into(),
        root.join(".codex").to_string_lossy().into_owned(),
    );
    configured.env.insert(
        "COPILOT_HOME".into(),
        root.join(".copilot").to_string_lossy().into_owned(),
    );
    configured.env.insert(
        "PI_CODING_AGENT_DIR".into(),
        root.join(".pi/agent").to_string_lossy().into_owned(),
    );
    configured
}

fn history(root: &Path, cwd: &str, key: &str) {
    let claude_project: String = cwd
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect();
    for path in [
        root.join(".claude/projects")
            .join(claude_project)
            .join(format!("{key}.jsonl")),
        root.join(".copilot/session-state")
            .join(key)
            .join("events.jsonl"),
        root.join(".gemini/antigravity-cli/conversations")
            .join(format!("{key}.db")),
        root.join(".pi/agent/sessions")
            .join(crate::runtimes::pi::pi_project_slug(cwd))
            .join(format!("fixture_{key}.jsonl")),
    ] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
}

#[test]
fn spawn_goldens() {
    let mut rows = Vec::new();
    let mut effects_rows = Vec::new();
    for key in Runtime::ALL
        .map(Runtime::key)
        .into_iter()
        .chain(["unknown-runtime"])
    {
        for shape in ["fresh", "blank", "resume", "missing", "hooks-off", "speed"] {
            if shape == "hooks-off"
                && !matches!(
                    key,
                    "claude-code" | "codex" | "copilot" | "pi" | "antigravity"
                )
            {
                continue;
            }
            if shape == "speed" && key != "codex" {
                continue;
            }
            let data = tempfile::tempdir().unwrap();
            let root = data.path();
            let pool = pool_with_schema();
            let mut configured = configured_role(key, root);
            if key == "codex" {
                configured.env.insert(
                    "CODEX_HOME".into(),
                    root.join("custom-codex").to_string_lossy().into_owned(),
                );
            }
            if shape == "blank" {
                configured.system_prompt = None;
                configured.model = None;
                configured.effort = None;
            }
            if shape == "speed" {
                configured.codex_speed = Some(CodexSpeed::Fast);
            }
            if shape == "hooks-off" {
                if key == "claude-code" {
                    configured
                        .args
                        .extend(["--settings".into(), "custom.json".into()]);
                }
                if key == "codex" {
                    configured.args.extend(["--disable".into(), "hooks".into()]);
                }
            } else {
                crate::runtimes::copilot::copilot_status::install_plugin(root).unwrap();
                crate::runtimes::pi::pi_status::install_extension(root).unwrap();
                crate::runtimes::antigravity::agy_status::install_hooks(root).unwrap();
            }
            insert_role_row(&pool.get().unwrap(), &configured);
            let fake = fake_runtime();
            let mgr = mgr_with_fake(None, Arc::clone(&fake));
            let mut normalizer = Normalizer::default();
            let (snapshot, effects) = with_conversation_home(root, || {
                let spawned = mgr
                    .spawn_direct(
                        &configured,
                        None,
                        None,
                        None,
                        None,
                        configured.working_dir.as_deref(),
                        None,
                        None,
                        root,
                        Arc::clone(&pool),
                        capture(),
                        crate::router::prompt::compose_direct_first_turn(
                            configured.system_prompt.as_deref(),
                        ),
                    )
                    .unwrap();
                if matches!(shape, "resume" | "missing") {
                    mgr.kill(&spawned.id).unwrap();
                    let prior = uuid::Uuid::new_v4().to_string();
                    pool.get()
                        .unwrap()
                        .execute(
                            "UPDATE sessions SET agent_session_key = ?1 WHERE id = ?2",
                            params![prior, spawned.id],
                        )
                        .unwrap();
                    if shape == "resume" {
                        history(root, configured.working_dir.as_deref().unwrap(), &prior);
                    }
                    mgr.resume(&spawned.id, None, None, root, Arc::clone(&pool), capture())
                        .unwrap();
                }
                let snapshot = normalizer.spawn(&fake, root);
                let effects = normalizer.effects(&fake, &mgr, key, root);
                mgr.kill(&spawned.id).unwrap();
                (snapshot, effects)
            });
            rows.push(json!({"runtime": key, "shape": shape, "spawn": snapshot}));
            effects_rows.push(json!({"runtime": key, "shape": shape, "spawn": effects}));
        }
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ] {
            let data = tempfile::tempdir().unwrap();
            let pool = pool_with_schema();
            let mut configured = configured_role(key, data.path());
            if key == "codex" {
                configured.env.insert(
                    "CODEX_HOME".into(),
                    data.path()
                        .join("custom-codex")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            configured.args = crate::runtimes::for_key(key)
                .permissions()
                .apply(&configured.args, mode);
            insert_role_row(&pool.get().unwrap(), &configured);
            let fake = fake_runtime();
            let mgr = mgr_with_fake(None, Arc::clone(&fake));
            let (snapshot, effects) = with_conversation_home(data.path(), || {
                let spawned = mgr
                    .spawn_direct(
                        &configured,
                        None,
                        None,
                        None,
                        None,
                        configured.working_dir.as_deref(),
                        None,
                        None,
                        data.path(),
                        Arc::clone(&pool),
                        capture(),
                        Some("GOLDEN_PERSONA".into()),
                    )
                    .unwrap();
                let mut normalizer = Normalizer::default();
                let snapshot = normalizer.spawn(&fake, data.path());
                let effects = normalizer.effects(&fake, &mgr, key, data.path());
                mgr.kill(&spawned.id).unwrap();
                (snapshot, effects)
            });
            rows.push(json!({"runtime": key, "shape": "direct-permission", "mode": mode, "spawn": snapshot}));
            effects_rows.push(json!({"runtime": key, "shape": "direct-permission", "mode": mode, "spawn": effects}));
        }
        for mode in MissionPermissionMode::ALL {
            for lead in [false, true] {
                let data = tempfile::tempdir().unwrap();
                let root = data.path();
                let pool = pool_with_schema();
                let mut configured = configured_role(key, root);
                if key == "codex" {
                    configured.env.insert(
                        "CODEX_HOME".into(),
                        root.join("custom-codex").to_string_lossy().into_owned(),
                    );
                }
                let mut mission = mission();
                mission.cwd = configured.working_dir.clone();
                mission.crew_id = "c".into();
                let mut slot = slot_for(&configured);
                slot.id = insert_crew_role(&pool, &mission.id, &configured.id);
                slot.lead = lead;
                update_role_row(&pool.get().unwrap(), &configured);
                let fake = fake_runtime();
                let mgr = mgr_with_fake(None, Arc::clone(&fake));
                mgr.set_mission_permission_mode(mode);
                let (snapshot, effects) = with_conversation_home(root, || {
                    let (system_prompt, first_turn) = if lead {
                        crate::runtimes::for_key(key).prompt_channels().lead(
                            &crate::router::prompt::LaunchPromptInput {
                                lead: crate::router::prompt::LeadView {
                                    handle: "tester",
                                    display_name: "Tester",
                                    system_prompt: Some("GOLDEN_PERSONA"),
                                },
                                crew_name: "Golden crew",
                                mission_goal: "Golden goal",
                                roster: &[],
                                allowed_signals: &[],
                                crew_addendum: Some("TEAM"),
                            },
                        )
                    } else {
                        crate::runtimes::for_key(key).prompt_channels().split(
                            crate::router::prompt::SessionPromptKind::Worker,
                            Some(crate::router::prompt::compose_worker_first_turn(
                                Some("GOLDEN_PERSONA"),
                                Some("TEAM"),
                            )),
                        )
                    };
                    let spawned = mgr
                        .spawn_with_prompt_channels(
                            &mission,
                            &configured,
                            &slot,
                            root,
                            root.join("events.ndjson"),
                            Arc::clone(&pool),
                            capture(),
                            system_prompt,
                            first_turn,
                        )
                        .unwrap();
                    let mut normalizer = Normalizer::default();
                    let snapshot = normalizer.spawn(&fake, root);
                    let effects = normalizer.effects(&fake, &mgr, key, root);
                    mgr.kill(&spawned.id).unwrap();
                    (snapshot, effects)
                });
                rows.push(json!({"runtime": key, "shape": if lead {"mission-lead"} else {"mission-worker"}, "mode": mode, "spawn": snapshot}));
                effects_rows.push(json!({"runtime": key, "shape": if lead {"mission-lead"} else {"mission-worker"}, "mode": mode, "spawn": effects}));
            }
        }
    }
    crate::golden::assert_golden("spawns", json!(rows));
    crate::golden::assert_golden("spawn-effects", json!(effects_rows));
}

#[test]
fn fork_goldens() {
    let mut rows = Vec::new();
    let mut effects_rows = Vec::new();
    for key in ["claude-code", "codex", "pi"] {
        let data = tempfile::tempdir().unwrap();
        let root = data.path();
        let pool = pool_with_schema();
        let mut configured = configured_role(key, root);
        let source_key = uuid::Uuid::new_v4().to_string();
        let fork_key = uuid::Uuid::new_v4().to_string();
        let materializer = if key == "codex" {
            let fixture = super::fork::codex_fork_materializer(&source_key, &fork_key, true);
            configured.command = fixture.1.clone();
            configured.env.insert(
                "CODEX_HOME".into(),
                fixture.3.to_string_lossy().into_owned(),
            );
            Some(fixture)
        } else {
            None
        };
        insert_role_row(&pool.get().unwrap(), &configured);
        let mut source =
            crate::repo::session::SessionRowDb::new_running(ulid::Ulid::new().to_string());
        source.role_id = Some(configured.id.clone());
        source.cwd = configured.working_dir.clone();
        source.status = crate::model::SessionStatus::Stopped;
        source.agent_session_key = Some(source_key.clone());
        source.title = Some("Golden source".into());
        crate::repo::session::insert(&pool.get().unwrap(), &source).unwrap();
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let (snapshot, effects) = with_conversation_home(root, || {
            let spawned = mgr
                .spawn_fork(
                    &source.id,
                    Some("Golden fork".into()),
                    None,
                    None,
                    root,
                    Arc::clone(&pool),
                    capture(),
                )
                .unwrap();
            let mut normalizer = Normalizer::default();
            let mut snapshot = normalizer.spawn(&fake, root);
            if let Some((dir, _, capture_path, _)) = &materializer {
                snapshot["command"] = json!("<MATERIALIZER>/codex-fork-materializer");
                snapshot["headless"] = json!(normalizer
                    .text(&std::fs::read_to_string(capture_path).unwrap(), root)
                    .replace(&dir.path().to_string_lossy().to_string(), "<MATERIALIZER>"));
            }
            let mut effects = normalizer.effects(&fake, &mgr, key, root);
            if let Some((dir, _, _, _)) = &materializer {
                effects["env"]["CODEX_HOME"] = json!(effects["env"]["CODEX_HOME"]
                    .as_str()
                    .unwrap()
                    .replace(&dir.path().to_string_lossy().to_string(), "<MATERIALIZER>"));
                effects["key_capture"]["sessions_root"] = json!(effects["key_capture"]
                    ["sessions_root"]
                    .as_str()
                    .unwrap()
                    .replace(&dir.path().to_string_lossy().to_string(), "<MATERIALIZER>"));
            }
            mgr.kill(&spawned.id).unwrap();
            (snapshot, effects)
        });
        rows.push(json!({"runtime": key, "spawn": snapshot}));
        effects_rows.push(json!({"runtime": key, "spawn": effects}));
    }
    crate::golden::assert_golden("forks", json!(rows));
    crate::golden::assert_golden("fork-effects", json!(effects_rows));
}

fn files_at(root: &Path) -> Value {
    fn visit(dir: &Path, root: &Path, files: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, root, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    Normalizer::default()
                        .text(&std::fs::read_to_string(&path).unwrap(), root)
                        .replace(env!("CARGO_PKG_VERSION"), "<VERSION>"),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    json!(files)
}

#[test]
fn startup_files_golden() {
    let data = tempfile::tempdir().unwrap();
    let mgr = mgr_with_fake(None, fake_runtime());
    mgr.start_runtime_watchers(data.path(), pool_with_schema(), capture())
        .unwrap();
    crate::golden::assert_golden("startup-files", files_at(data.path()));
}

#[test]
fn trust_files_golden() {
    let mut rows = Vec::new();
    for key in Runtime::ALL
        .map(Runtime::key)
        .into_iter()
        .chain(["unknown-runtime"])
    {
        let data = tempfile::tempdir().unwrap();
        let project = data.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mgr = mgr_with_fake(None, fake_runtime());
        let copilot_home = data.path().join("custom-copilot");
        with_conversation_home(data.path(), || {
            let app_data = tempfile::tempdir().unwrap();
            let pool = pool_with_schema();
            let mut configured = configured_role(key, data.path());
            configured.working_dir = Some(project.to_string_lossy().into_owned());
            configured.env.insert(
                "COPILOT_HOME".into(),
                copilot_home.to_string_lossy().into_owned(),
            );
            insert_role_row(&pool.get().unwrap(), &configured);
            let spawned = mgr
                .spawn_direct(
                    &configured,
                    None,
                    None,
                    None,
                    None,
                    configured.working_dir.as_deref(),
                    None,
                    None,
                    app_data.path(),
                    pool,
                    capture(),
                    None,
                )
                .unwrap();
            mgr.kill(&spawned.id).unwrap();
            // Codex's mocked spawn seed is a no-op; exercise its production writer explicitly.
            if key == "codex" {
                crate::runtimes::codex::codex_trust::seed_project_trust_at(
                    &project,
                    &data.path().join(".codex/config.toml"),
                )
                .unwrap();
            }
        });
        rows.push(json!({"runtime": key, "files": files_at(data.path())}));
    }
    crate::golden::assert_golden("trust-files", json!(rows));
}

#[test]
fn spawn_capabilities_golden() {
    let data = tempfile::tempdir().unwrap();
    let mut rows = Vec::new();
    for key in Runtime::ALL
        .map(Runtime::key)
        .into_iter()
        .chain(["unknown-runtime"])
    {
        let mgr = mgr_with_fake(None, fake_runtime());
        mgr.enter_claude_launch_gate("golden", key);
        let markers: Vec<_> = [
            None,
            Some("TURN".into()),
            Some("x".repeat(crate::router::runtime::FIRST_TURN_ARGV_MAX_BYTES)),
        ]
        .into_iter()
        .map(|body| {
            let (body, marker) = SessionManager::codex_capture_prompt_marker(key, "golden", body);
            json!({"body_len": body.as_ref().map(String::len), "marker": marker})
        })
        .collect();
        rows.push(json!({"runtime": key, "hooks_unix": crate::runtimes::for_key(key).status_hooks().is_some_and(|hooks| hooks.supported(false)),
            "hooks_windows": crate::runtimes::for_key(key).status_hooks().is_some_and(|hooks| hooks.supported(true)),
            "launch_gate": mgr.claude_launch_gate.lock().unwrap().is_some(), "markers": markers,
            "env": Normalizer::default().text(&serde_json::to_string(&super::super::spawn::agent_env(BTreeMap::new(), &HashMap::new(), BTreeMap::new(), key)).unwrap(), data.path())}));
    }
    crate::golden::assert_golden("spawn-capabilities", json!(rows));
}

#[test]
fn catalog_speed_golden() {
    let mut rows = Vec::new();
    for runtime in Runtime::ALL
        .into_iter()
        .filter(|runtime| !runtime.is_shell())
    {
        let home = tempfile::tempdir().unwrap();
        let pool = pool_with_schema();
        let mut configured = configured_role(runtime.key(), home.path());
        let (mission, slot) = seed_mission_rows(&pool, &configured);
        for speed in [None, Some(CodexSpeed::Standard), Some(CodexSpeed::Fast)] {
            for slot_speed in [None, Some(CodexSpeed::Standard), Some(CodexSpeed::Fast)] {
                let fake = fake_runtime();
                let mgr = mgr_with_fake(None, Arc::clone(&fake));
                let mut conn = pool.get().unwrap();
                configured = crate::ops::role::update(
                    &conn,
                    &configured.id,
                    crate::ops::role::UpdateRoleInput {
                        codex_speed: Some(speed),
                        ..Default::default()
                    },
                )
                .unwrap();
                let updated = crate::ops::slot::update(
                    &mut conn,
                    &slot.id,
                    crate::ops::slot::UpdateSlotInput {
                        codex_speed_override: Some(slot_speed),
                        ..Default::default()
                    },
                )
                .unwrap();
                drop(conn);
                with_conversation_home(home.path(), || {
                    let spawned = mgr
                        .spawn_with_prompt_channels(
                            &mission,
                            &configured,
                            &updated.slot,
                            home.path(),
                            home.path().join("events.ndjson"),
                            Arc::clone(&pool),
                            capture(),
                            None,
                            None,
                        )
                        .unwrap();
                    let stored = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
                        .unwrap()
                        .unwrap();
                    let args: Vec<_> = fake
                        .last_spawn_spec()
                        .unwrap()
                        .args
                        .into_iter()
                        .filter(|arg| arg == "-c" || arg.starts_with("service_tier="))
                        .collect();
                    rows.push(json!({"runtime":runtime,"role_input":speed,"slot_input":slot_speed,"role_stored":crate::ops::role::get(&pool.get().unwrap(),&configured.id).unwrap().codex_speed,"slot_stored":updated.slot.codex_speed_override,"session_stored":stored.agent_speed,"speed_args":args}));
                    mgr.kill(&spawned.id).unwrap();
                });
            }
        }
    }
    crate::golden::assert_golden("catalog-speed", json!(rows));
}
