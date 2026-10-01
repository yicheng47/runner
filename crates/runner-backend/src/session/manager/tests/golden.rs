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
                crate::session::copilot_status::install_plugin(root).unwrap();
                crate::session::pi_status::install_extension(root).unwrap();
                crate::session::agy_status::install_hooks(root).unwrap();
            }
            insert_role_row(&pool.get().unwrap(), &configured);
            let fake = fake_runtime();
            let mgr = mgr_with_fake(None, Arc::clone(&fake));
            let mut normalizer = Normalizer::default();
            let snapshot = with_conversation_home(root, || {
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
                mgr.kill(&spawned.id).unwrap();
                snapshot
            });
            rows.push(json!({"runtime": key, "shape": shape, "spawn": snapshot}));
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
            configured.args = crate::runtimes::for_key(key)
                .permissions()
                .apply(&configured.args, mode);
            insert_role_row(&pool.get().unwrap(), &configured);
            let fake = fake_runtime();
            let mgr = mgr_with_fake(None, Arc::clone(&fake));
            let snapshot = with_conversation_home(data.path(), || {
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
                let snapshot = Normalizer::default().spawn(&fake, data.path());
                mgr.kill(&spawned.id).unwrap();
                snapshot
            });
            rows.push(json!({"runtime": key, "shape": "direct-permission", "mode": mode, "spawn": snapshot}));
        }
        for mode in MissionPermissionMode::ALL {
            for lead in [false, true] {
                let data = tempfile::tempdir().unwrap();
                let root = data.path();
                let pool = pool_with_schema();
                let configured = configured_role(key, root);
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
                let snapshot = with_conversation_home(root, || {
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
                    let snapshot = Normalizer::default().spawn(&fake, root);
                    mgr.kill(&spawned.id).unwrap();
                    snapshot
                });
                rows.push(json!({"runtime": key, "shape": if lead {"mission-lead"} else {"mission-worker"}, "mode": mode, "spawn": snapshot}));
            }
        }
    }
    crate::golden::assert_golden("spawns", json!(rows));
}

#[test]
fn fork_goldens() {
    let mut rows = Vec::new();
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
        let snapshot = with_conversation_home(root, || {
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
            mgr.kill(&spawned.id).unwrap();
            snapshot
        });
        rows.push(json!({"runtime": key, "spawn": snapshot}));
    }
    crate::golden::assert_golden("forks", json!(rows));
}
