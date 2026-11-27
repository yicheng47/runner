use std::collections::{HashMap, HashSet};

use chrono::Utc;
use runner_core::protocol::permissions::PermissionMode;
use runner_core::protocol::role::CreateRoleInput;
use runner_core::protocol::runtime::RuntimeRowState;
use runner_core::protocol::template::{
    ExportedTemplate, TemplateImportInput, TemplateImportResult, TemplateKind, TemplatePreview,
};
use runner_core::template::{
    allocate_role_handle, build_preview, parse_template, resolve_crew_import, resolve_role_import,
    suggest_crew_name, Template,
};
use rusqlite::{Connection, TransactionBehavior};
use ulid::Ulid as UlidGen;

use crate::error::{Error, Result};
use crate::model::{Crew, Role, Runtime, Slot, SlotWithRole};
use crate::ops::role::{create as create_role, validate_handle};
use crate::{repo, AppCore};

pub fn export_crew(core: &AppCore, id: &str) -> Result<ExportedTemplate> {
    let (crew, slots) = {
        let mut conn = core.db.get()?;
        read_crew_export_snapshot(&mut conn, id, || {})?
    };
    runner_core::template::export_crew(&crew, &slots).map_err(|err| Error::msg(err.to_string()))
}

fn read_crew_export_snapshot(
    conn: &mut Connection,
    id: &str,
    after_crew_read: impl FnOnce(),
) -> Result<(Crew, Vec<SlotWithRole>)> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let crew = crate::ops::crew::get(&tx, id)?;
    after_crew_read();
    let slots = crate::ops::slot::list(&tx, id)?;
    Ok((crew, slots))
}

pub fn export_role(core: &AppCore, id: &str) -> Result<ExportedTemplate> {
    let conn = core.db.get()?;
    let role = crate::ops::role::get(&conn, id)?;
    runner_core::template::export_role(&role).map_err(|err| Error::msg(err.to_string()))
}

pub fn preview(core: &AppCore, toml: &str) -> Result<TemplatePreview> {
    let template = match parse_template(toml) {
        Ok(template) => template,
        Err(error) => return Ok(TemplatePreview::Invalid { error }),
    };

    // Query runtime status before checking out a DB connection so single-connection
    // test pools never deadlock.
    let installed = installed_runtimes(core)?;

    let (existing_crews, existing_handles) = {
        let mut conn = core.db.get()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let existing_crews = repo::crew::list_with_role_count(&tx)?
            .into_iter()
            .map(|(crew, _)| crew.name)
            .collect::<Vec<_>>();
        let existing_handles = repo::role::list(&tx)?
            .into_iter()
            .map(|role| role.handle)
            .collect::<HashSet<_>>();
        (existing_crews, existing_handles)
    };

    Ok(TemplatePreview::Valid(Box::new(build_preview(
        template,
        &existing_crews,
        &existing_handles,
        &installed,
    ))))
}

pub fn import(core: &AppCore, input: TemplateImportInput) -> Result<TemplateImportResult> {
    let template = parse_template(&input.toml).map_err(|err| Error::msg(err.to_string()))?;

    // Query runtime status before checking out a DB connection.
    let installed = installed_runtimes(core)?;

    let result = {
        let mut conn = core.db.get()?;
        import_with_conn(&mut conn, template, &input, &installed)?
    };

    core.events.emit("role/changed", &());
    if result.kind == TemplateKind::Crew {
        core.events.emit("crew/changed", &());
        core.events.emit("slot/changed", &());
    }
    Ok(result)
}

pub(crate) fn import_with_conn(
    conn: &mut Connection,
    template: Template,
    input: &TemplateImportInput,
    installed_runtimes: &[Runtime],
) -> Result<TemplateImportResult> {
    let installed_set: HashSet<Runtime> = installed_runtimes.iter().copied().collect();

    match template {
        Template::Crew(crew_tmpl) => {
            if input.role_handle.is_some() || input.role_display_name.is_some() {
                return Err(Error::msg(
                    "role_handle and role_display_name are only valid for standalone role templates",
                ));
            }
            let resolved = resolve_crew_import(&crew_tmpl, &input.runtime_overrides)
                .map_err(|err| Error::msg(err.to_string()))?;

            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let crew_name = match input.crew_name.as_deref() {
                Some(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        return Err(Error::msg("crew name must not be empty"));
                    }
                    trimmed.to_owned()
                }
                None => {
                    let existing_crews = repo::crew::list_with_role_count(&tx)?
                        .into_iter()
                        .map(|(crew, _)| crew.name)
                        .collect::<Vec<_>>();
                    suggest_crew_name(&crew_tmpl.crew.name, &existing_crews)
                }
            };

            let mut used_handles = repo::role::list(&tx)?
                .into_iter()
                .map(|role| role.handle)
                .collect::<HashSet<_>>();

            let mut created_roles: Vec<Role> = Vec::with_capacity(resolved.roles.len());
            let mut role_id_by_template_handle: HashMap<String, String> =
                HashMap::with_capacity(resolved.roles.len());

            for role_tmpl in &resolved.roles {
                let handle = allocate_role_handle(&role_tmpl.handle, &mut used_handles);
                let command = role_tmpl.runtime.command().ok_or_else(|| {
                    Error::msg(format!("unknown runtime '{}'", role_tmpl.runtime))
                })?;
                let created = create_role(
                    &tx,
                    CreateRoleInput {
                        handle,
                        display_name: role_tmpl.display_name.clone(),
                        runtime: role_tmpl.runtime,
                        command: command.to_owned(),
                        args: Vec::new(),
                        working_dir: None,
                        system_prompt: role_tmpl.system_prompt.clone(),
                        env: HashMap::new(),
                        model: role_tmpl.model.clone(),
                        effort: role_tmpl.effort.clone(),
                        codex_speed: role_tmpl.codex_speed,
                        permission_mode: PermissionMode::Default,
                    },
                )?;
                role_id_by_template_handle.insert(role_tmpl.handle.clone(), created.id.clone());
                created_roles.push(created);
            }

            let now = Utc::now();
            let crew_id = UlidGen::new().to_string();
            let conventions = crew_tmpl
                .crew
                .conventions
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .cloned();
            repo::crew::insert(
                &tx,
                &repo::crew::CrewRow {
                    id: crew_id.clone(),
                    name: crew_name,
                    system_prompt_addendum: conventions,
                    created_at: now,
                    updated_at: now,
                },
            )?;
            let crew: Crew = crate::ops::crew::get(&tx, &crew_id)?;

            let added_at = Utc::now();
            let mut uninstalled_runtimes = Vec::new();
            for (position, slot) in resolved.slots.iter().enumerate() {
                let role_id = role_id_by_template_handle
                    .get(&slot.template_role_handle)
                    .ok_or_else(|| {
                        Error::msg(format!("missing role for slot @{}", slot.slot_handle))
                    })?
                    .clone();
                repo::slot::insert(
                    &tx,
                    &repo::slot::SlotRow {
                        id: UlidGen::new().to_string(),
                        crew_id: crew.id.clone(),
                        role_id,
                        slot_handle: slot.slot_handle.clone(),
                        position: position as i64,
                        lead: false,
                        runtime_override: slot.runtime_override.map(|rt| rt.key().to_owned()),
                        model_override: slot.model_override.clone(),
                        effort_override: slot.effort_override.clone(),
                        codex_speed_override: slot.codex_speed_override,
                        added_at,
                    },
                )?;
                if !installed_set.contains(&slot.effective_runtime)
                    && !uninstalled_runtimes.contains(&slot.effective_runtime)
                {
                    uninstalled_runtimes.push(slot.effective_runtime);
                }
            }

            let slots: Vec<Slot> = repo::slot::list_for_crew(&tx, &crew.id)?;
            tx.commit()?;

            Ok(TemplateImportResult {
                kind: TemplateKind::Crew,
                crew: Some(crew),
                roles: created_roles,
                slots,
                uninstalled_runtimes,
            })
        }
        Template::Role(role_tmpl) => {
            if input.crew_name.is_some() {
                return Err(Error::msg("crew_name is only valid for crew templates"));
            }
            let resolved = resolve_role_import(&role_tmpl, &input.runtime_overrides)
                .map_err(|err| Error::msg(err.to_string()))?;

            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut used_handles = repo::role::list(&tx)?
                .into_iter()
                .map(|role| role.handle)
                .collect::<HashSet<_>>();

            let handle = match input.role_handle.as_deref() {
                Some(raw) => {
                    let trimmed = raw.trim().strip_prefix('@').unwrap_or(raw.trim());
                    validate_handle(trimmed)?;
                    if used_handles.contains(trimmed) {
                        return Err(Error::msg(format!(
                            "role handle @{trimmed} is already taken"
                        )));
                    }
                    trimmed.to_owned()
                }
                None => allocate_role_handle(&resolved.role.handle, &mut used_handles),
            };

            let display_name = match input.role_display_name.as_deref() {
                Some(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        return Err(Error::msg("display_name must not be empty"));
                    }
                    trimmed.to_owned()
                }
                None => resolved.role.display_name.clone(),
            };

            let command = resolved.role.runtime.command().ok_or_else(|| {
                Error::msg(format!("unknown runtime '{}'", resolved.role.runtime))
            })?;
            let created = create_role(
                &tx,
                CreateRoleInput {
                    handle,
                    display_name,
                    runtime: resolved.role.runtime,
                    command: command.to_owned(),
                    args: Vec::new(),
                    working_dir: None,
                    system_prompt: resolved.role.system_prompt.clone(),
                    env: HashMap::new(),
                    model: resolved.role.model.clone(),
                    effort: resolved.role.effort.clone(),
                    codex_speed: resolved.role.codex_speed,
                    permission_mode: PermissionMode::Default,
                },
            )?;
            tx.commit()?;

            let uninstalled_runtimes = if installed_set.contains(&resolved.role.runtime) {
                Vec::new()
            } else {
                vec![resolved.role.runtime]
            };

            Ok(TemplateImportResult {
                kind: TemplateKind::Role,
                crew: None,
                roles: vec![created],
                slots: Vec::new(),
                uninstalled_runtimes,
            })
        }
    }
}

fn installed_runtimes(core: &AppCore) -> Result<Vec<Runtime>> {
    let status = crate::ops::runtime::runtime_status_list_base(core)?;
    let mut runtimes = status
        .runtimes
        .into_iter()
        .filter(|entry| {
            matches!(
                entry.state,
                RuntimeRowState::Detected | RuntimeRowState::Override
            ) && !entry.below_minimum
                && crate::runtimes::adapter(entry.name).catalog().is_some()
        })
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    runtimes.sort_by_key(|rt| rt.setup_rank());
    Ok(runtimes)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use runner_core::protocol::CodexSpeed;

    use super::*;

    fn write_fake_binary(dir: &Path, name: &str) {
        #[cfg(windows)]
        let file = dir.join(format!("{name}.cmd"));
        #[cfg(not(windows))]
        let file = dir.join(name);
        std::fs::write(&file, "#!/bin/sh\necho 99.0.0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&file).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&file, perms).unwrap();
        }
    }

    fn core_with_installed(runtimes: &[Runtime]) -> (tempfile::TempDir, AppCore) {
        let tmp = tempfile::tempdir().unwrap();
        for rt in runtimes {
            write_fake_binary(tmp.path(), rt.command().unwrap());
        }
        let core = crate::test_support::test_core();
        {
            let mut env = core.runtime_shell_env.write().unwrap();
            env.path = Some(tmp.path().display().to_string());
            env.vars.insert(
                crate::runtime_status::TEST_ISOLATE_PATH_ENV.into(),
                "1".into(),
            );
        }
        core.runtime_discovery.write().unwrap().checking = false;
        (tmp, core)
    }

    const SAMPLE_CREW_TOML: &str = r#"
version = 1
kind = "crew"

[meta]
description = "Three-slot delivery loop."
author = "Runner"

[crew]
name = "Codex Trio"
conventions = """
Keep changes focused.
Test before handing off."""

[[slots]]
handle = "coder"
role = "impl-codex"

[[slots]]
handle = "reviewer"
role = "review-codex"
runtime = "pi"
model = "deepseek-chat"
effort = "high"

[[roles]]
handle = "impl-codex"
display_name = "Implementer (Codex)"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
codex_speed = "fast"
system_prompt = "Write clean code."

[[roles]]
handle = "review-codex"
display_name = "Reviewer (Codex)"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
system_prompt = "Critique only."
"#;

    #[test]
    fn import_always_creates_fresh_roles_suffixes_handles_and_crew_name_and_emits_events() {
        let (_tmp, core) = core_with_installed(&[Runtime::Codex, Runtime::ClaudeCode]);
        let mut events = core.events.subscribe();

        // First import creates original handles and crew name.
        let first = import(
            &core,
            TemplateImportInput {
                toml: SAMPLE_CREW_TOML.into(),
                ..Default::default()
            },
        )
        .unwrap();
        let crew1 = first.crew.unwrap();
        assert_eq!(crew1.name, "Codex Trio");
        assert_eq!(first.roles.len(), 2);
        assert_eq!(first.roles[0].handle, "impl-codex");
        assert_eq!(first.roles[1].handle, "review-codex");
        // Roles created with PermissionMode::Default (empty args for Codex).
        assert!(first.roles[0].args.is_empty());
        assert_eq!(first.roles[0].command, "codex");
        // pi is not installed on this test machine.
        assert_eq!(first.uninstalled_runtimes, vec![Runtime::Pi]);

        let mut seen = Vec::new();
        while let Ok(evt) = events.try_recv() {
            seen.push(evt.name);
        }
        assert!(seen.contains(&"role/changed"));
        assert!(seen.contains(&"crew/changed"));
        assert!(seen.contains(&"slot/changed"));

        // Second import of identical template never reuses roles: auto-suffixes both handles and crew name.
        let preview_second = preview(&core, SAMPLE_CREW_TOML).unwrap();
        let TemplatePreview::Valid(plan) = preview_second else {
            panic!("expected valid preview");
        };
        assert_eq!(plan.suggested_crew_name.as_deref(), Some("Codex Trio (2)"));
        assert_eq!(plan.planned_roles[0].planned_handle, "impl-codex-2");
        assert!(plan.planned_roles[0].auto_suffixed);
        assert_eq!(plan.planned_roles[1].planned_handle, "review-codex-2");
        assert!(plan.planned_slots[0].runtime_installed);
        assert!(!plan.planned_slots[1].runtime_installed);
        assert!(plan.planned_slots[1].has_slot_override);

        let second = import(
            &core,
            TemplateImportInput {
                toml: SAMPLE_CREW_TOML.into(),
                ..Default::default()
            },
        )
        .unwrap();
        let crew2 = second.crew.unwrap();
        assert_eq!(crew2.name, "Codex Trio (2)");
        assert_eq!(second.roles[0].handle, "impl-codex-2");
        assert_eq!(second.roles[1].handle, "review-codex-2");
        assert_ne!(second.roles[0].id, first.roles[0].id);
        assert_ne!(second.roles[1].id, first.roles[1].id);
    }

    #[test]
    fn import_runtime_reassignment_clears_model_effort_and_codex_speed() {
        let (_tmp, core) = core_with_installed(&[Runtime::ClaudeCode]);

        let res = import(
            &core,
            TemplateImportInput {
                toml: SAMPLE_CREW_TOML.into(),
                runtime_overrides: BTreeMap::from([
                    ("coder".into(), Runtime::ClaudeCode),
                    ("reviewer".into(), Runtime::ClaudeCode),
                ]),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(res.uninstalled_runtimes.is_empty());
        assert_eq!(res.roles[0].runtime, "claude-code");
        assert_eq!(res.roles[0].command, "claude");
        assert_eq!(res.roles[0].model, None);
        assert_eq!(res.roles[0].effort, None);
        assert_eq!(res.roles[0].codex_speed, None);

        assert_eq!(res.roles[1].runtime, "claude-code");
        assert_eq!(res.roles[1].model, None);
        assert_eq!(res.roles[1].effort, None);
        assert_eq!(res.roles[1].codex_speed, None);

        assert_eq!(res.slots[1].runtime_override, None);
        assert_eq!(res.slots[1].model_override, None);
        assert_eq!(res.slots[1].effort_override, None);
        assert_eq!(res.slots[1].codex_speed_override, None);
    }

    #[test]
    fn import_handles_32_char_and_competing_template_handles() {
        let (_tmp, core) = core_with_installed(&[Runtime::Codex]);
        let long_base = "a".repeat(32);
        let base_30 = "a".repeat(30);
        // Pre-occupy the 32-char handle and its `-2` suffix locally.
        {
            let conn = core.db.get().unwrap();
            for handle in [&long_base, &format!("{base_30}-2")] {
                create_role(
                    &conn,
                    CreateRoleInput {
                        handle: handle.clone(),
                        display_name: "Existing".into(),
                        runtime: Runtime::Codex,
                        command: "codex".into(),
                        args: vec![],
                        working_dir: None,
                        system_prompt: None,
                        env: HashMap::new(),
                        model: None,
                        effort: None,
                        codex_speed: None,
                        permission_mode: PermissionMode::Default,
                    },
                )
                .unwrap();
            }
        }
        // Two template roles whose suffixed names compete: `a*32` wants `a*30-3`, and `a*30-3` itself is also in the template.
        let competing = format!("{base_30}-3");
        let toml = format!(
            r#"
version = 1
kind = "crew"
[crew]
name = "Long Handles"
[[slots]]
handle = "first"
role = "{long_base}"
[[slots]]
handle = "second"
role = "{competing}"
[[roles]]
handle = "{long_base}"
display_name = "Long"
runtime = "codex"
[[roles]]
handle = "{competing}"
display_name = "Competing"
runtime = "codex"
"#
        );

        let res = import(
            &core,
            TemplateImportInput {
                toml,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(res.roles[0].handle, format!("{base_30}-3"));
        assert_eq!(res.roles[1].handle, format!("{base_30}-4"));
        assert!(res.roles[0].handle.len() <= 32);
        assert!(res.roles[1].handle.len() <= 32);
    }

    #[test]
    fn standalone_role_import_and_export_round_trip_and_handle_collision_rolls_back() {
        let (_tmp, core) = core_with_installed(&[Runtime::ClaudeCode]);
        let role_toml = r#"
version = 1
kind = "role"
[role]
handle = "review-codex"
display_name = "Reviewer"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
codex_speed = "fast"
system_prompt = "Inspect every line."
"#;

        // Reassign uninstalled codex -> claude-code; clears model, effort, codex_speed.
        let imported = import(
            &core,
            TemplateImportInput {
                toml: role_toml.into(),
                role_display_name: Some("Reviewer (Claude)".into()),
                role_handle: Some("my-reviewer".into()),
                runtime_overrides: BTreeMap::from([("review-codex".into(), Runtime::ClaudeCode)]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(imported.roles.len(), 1);
        let role = &imported.roles[0];
        assert_eq!(role.handle, "my-reviewer");
        assert_eq!(role.display_name, "Reviewer (Claude)");
        assert_eq!(role.runtime, "claude-code");
        assert_eq!(role.model, None);
        assert_eq!(role.effort, None);
        assert_eq!(role.codex_speed, None);
        assert_eq!(role.system_prompt.as_deref(), Some("Inspect every line."));

        // Explicit colliding handle fails cleanly and leaves DB untouched.
        let err = import(
            &core,
            TemplateImportInput {
                toml: role_toml.into(),
                role_handle: Some("my-reviewer".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("already taken"), "{err}");

        // Export back and verify round-trip.
        let exported = export_role(&core, &role.id).unwrap();
        assert_eq!(exported.filename, "my-reviewer.role.toml");
        let parsed = parse_template(&exported.toml).unwrap();
        let Template::Role(parsed_role) = parsed else {
            panic!("expected role");
        };
        assert_eq!(parsed_role.role.handle, "my-reviewer");
        assert_eq!(parsed_role.role.runtime, Runtime::ClaudeCode);
    }

    #[test]
    fn failed_crew_import_rolls_back_all_created_rows() {
        let (_tmp, core) = core_with_installed(&[Runtime::Codex]);
        let err = import(
            &core,
            TemplateImportInput {
                toml: SAMPLE_CREW_TOML.into(),
                crew_name: Some("   ".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("empty"), "{err}");

        // Also test a failure AFTER role insertion inside the same transaction via a trigger.
        {
            let conn = core.db.get().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_slot_insert BEFORE INSERT ON slots
                 BEGIN SELECT RAISE(ABORT, 'simulated slot failure'); END;",
            )
            .unwrap();
        }
        let err = import(
            &core,
            TemplateImportInput {
                toml: SAMPLE_CREW_TOML.into(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("simulated slot failure"), "{err}");

        let conn = core.db.get().unwrap();
        assert!(
            repo::role::list(&conn).unwrap().is_empty(),
            "roles rolled back"
        );
        assert_eq!(repo::crew::count(&conn).unwrap(), 0, "crew rolled back");
    }

    #[test]
    fn export_crew_round_trips_from_sqlite_with_overrides() {
        let (_tmp, core) = core_with_installed(&[Runtime::Codex, Runtime::Pi]);
        let imported = import(
            &core,
            TemplateImportInput {
                toml: SAMPLE_CREW_TOML.into(),
                ..Default::default()
            },
        )
        .unwrap();
        let crew = imported.crew.unwrap();
        let exported = export_crew(&core, &crew.id).unwrap();
        assert_eq!(exported.filename, "codex-trio.crew.toml");

        let parsed = parse_template(&exported.toml).unwrap();
        let Template::Crew(parsed) = parsed else {
            panic!("expected crew");
        };
        assert_eq!(parsed.crew.name, "Codex Trio");
        assert_eq!(parsed.slots.len(), 2);
        assert_eq!(parsed.slots[1].runtime, Some(Runtime::Pi));
        assert_eq!(parsed.slots[1].model.as_deref(), Some("deepseek-chat"));
        assert_eq!(parsed.roles[0].codex_speed, Some(CodexSpeed::Fast));
    }

    #[test]
    fn shared_role_mixed_runtime_choices_preserve_effective_launch_settings() {
        use crate::session::manager::resolve_runtime_override;

        let (_tmp, core) = core_with_installed(&[Runtime::Codex, Runtime::ClaudeCode]);

        // Case A: first slot switches from codex -> claude-code; second slot stays codex.
        // Role takes first slot's chosen runtime (claude-code); second slot gets runtime_override
        // with explicit tuning so its resolved launch settings still match the template.
        let shared_toml = r#"
version = 1
kind = "crew"

[crew]
name = "Shared A"

[[slots]]
handle = "first"
role = "player"

[[slots]]
handle = "second"
role = "player"

[[roles]]
handle = "player"
display_name = "Player"
runtime = "codex"
model = "gpt-5"
effort = "high"
codex_speed = "fast"
system_prompt = "Play."
"#;
        let res_a = import(
            &core,
            TemplateImportInput {
                toml: shared_toml.into(),
                runtime_overrides: BTreeMap::from([("first".into(), Runtime::ClaudeCode)]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(res_a.roles.len(), 1);
        let role_a = &res_a.roles[0];
        assert_eq!(role_a.runtime, "claude-code");
        assert_eq!(role_a.model, None);
        assert_eq!(role_a.effort, None);
        assert_eq!(role_a.codex_speed, None);

        let resolve_slot = |role: &Role, slot: &Slot| {
            let mut eff = resolve_runtime_override(
                role,
                slot.runtime_override.as_deref(),
                slot.model_override.as_deref(),
                slot.effort_override.as_deref(),
            )
            .unwrap()
            .unwrap_or_else(|| role.clone());
            if eff.runtime == "codex" {
                eff.codex_speed = slot.codex_speed_override.or(eff.codex_speed);
            } else {
                eff.codex_speed = None;
            }
            eff
        };

        let first_a = resolve_slot(role_a, &res_a.slots[0]);
        assert_eq!(first_a.runtime, "claude-code");
        assert_eq!(first_a.model, None);
        assert_eq!(first_a.effort, None);
        assert_eq!(first_a.codex_speed, None);

        let second_a = resolve_slot(role_a, &res_a.slots[1]);
        assert_eq!(second_a.runtime, "codex");
        assert_eq!(second_a.model.as_deref(), Some("gpt-5"));
        assert_eq!(second_a.effort.as_deref(), Some("high"));
        assert_eq!(second_a.codex_speed, Some(CodexSpeed::Fast));

        // Case B: slot 0 had explicit runtime = "pi" and switches back to role's original runtime
        // ("codex"), while slot 1 stays untouched on inherited codex. Slot 0 must launch with
        // defaults (NOT inherit role's gpt-5/high/fast), while slot 1 preserves gpt-5/high/fast.
        let switched_back_toml = r#"
version = 1
kind = "crew"

[crew]
name = "Shared B"

[[slots]]
handle = "switched"
role = "player"
runtime = "pi"
model = "deepseek-chat"
effort = "low"

[[slots]]
handle = "untouched"
role = "player"

[[roles]]
handle = "player"
display_name = "Player"
runtime = "codex"
model = "gpt-5"
effort = "high"
codex_speed = "fast"
system_prompt = "Play."
"#;
        let res_b = import(
            &core,
            TemplateImportInput {
                toml: switched_back_toml.into(),
                runtime_overrides: BTreeMap::from([("switched".into(), Runtime::Codex)]),
                ..Default::default()
            },
        )
        .unwrap();
        let role_b = &res_b.roles[0];
        assert_eq!(role_b.runtime, "codex");
        assert_eq!(role_b.model, None);
        assert_eq!(role_b.effort, None);
        assert_eq!(role_b.codex_speed, None);

        let switched_b = resolve_slot(role_b, &res_b.slots[0]);
        assert_eq!(switched_b.runtime, "codex");
        assert_eq!(switched_b.model, None);
        assert_eq!(switched_b.effort, None);
        assert_eq!(switched_b.codex_speed, None);

        let untouched_b = resolve_slot(role_b, &res_b.slots[1]);
        assert_eq!(untouched_b.runtime, "codex");
        assert_eq!(untouched_b.model.as_deref(), Some("gpt-5"));
        assert_eq!(untouched_b.effort.as_deref(), Some("high"));
        assert_eq!(untouched_b.codex_speed, Some(CodexSpeed::Fast));

        // Case C: untouched slot A has an explicit matching runtime pin (`runtime = "codex"`),
        // while sibling slot B (on unavailable `pi`) is remapped to `claude-code`. Slot A must
        // retain its explicit `runtime_override = Some("codex")` both in SQLite and on re-export.
        let matching_pin_toml = r#"
version = 1
kind = "crew"

[crew]
name = "Shared Pin"

[[slots]]
handle = "pinned"
role = "player"
runtime = "codex"
model = "gpt-5.5"

[[slots]]
handle = "remapped"
role = "player"
runtime = "pi"
model = "deepseek-chat"

[[roles]]
handle = "player"
display_name = "Player"
runtime = "codex"
model = "gpt-5"
effort = "high"
codex_speed = "fast"
system_prompt = "Play."
"#;
        let res_c = import(
            &core,
            TemplateImportInput {
                toml: matching_pin_toml.into(),
                runtime_overrides: BTreeMap::from([("remapped".into(), Runtime::ClaudeCode)]),
                ..Default::default()
            },
        )
        .unwrap();
        let crew_c = res_c.crew.as_ref().unwrap();
        let role_c = &res_c.roles[0];
        assert_eq!(role_c.runtime, "codex");
        assert_eq!(role_c.model.as_deref(), Some("gpt-5"));
        assert_eq!(res_c.slots[0].runtime_override.as_deref(), Some("codex"));
        assert_eq!(res_c.slots[0].model_override.as_deref(), Some("gpt-5.5"));
        assert_eq!(
            res_c.slots[1].runtime_override.as_deref(),
            Some("claude-code")
        );
        assert_eq!(res_c.slots[1].model_override, None);

        let pinned_eff = resolve_slot(role_c, &res_c.slots[0]);
        assert_eq!(pinned_eff.runtime, "codex");
        assert_eq!(pinned_eff.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(pinned_eff.effort.as_deref(), Some("high"));
        assert_eq!(pinned_eff.codex_speed, Some(CodexSpeed::Fast));

        let remapped_eff = resolve_slot(role_c, &res_c.slots[1]);
        assert_eq!(remapped_eff.runtime, "claude-code");
        assert_eq!(remapped_eff.model, None);
        assert_eq!(remapped_eff.effort, None);
        assert_eq!(remapped_eff.codex_speed, None);

        // Re-exporting preserves the explicit runtime pin on `pinned` and the override on `remapped`.
        let exported_c = export_crew(&core, &crew_c.id).unwrap();
        let Template::Crew(re_c) = parse_template(&exported_c.toml).unwrap() else {
            panic!("expected crew");
        };
        assert_eq!(re_c.slots[0].runtime, Some(Runtime::Codex));
        assert_eq!(re_c.slots[0].model.as_deref(), Some("gpt-5.5"));
        assert_eq!(re_c.slots[1].runtime, Some(Runtime::ClaudeCode));
        assert_eq!(re_c.slots[1].model, None);

        // Re-exporting Case A also preserves the synthesized pin + tuning on the originally inherited slot.
        let exported_a = export_crew(&core, &res_a.crew.as_ref().unwrap().id).unwrap();
        let Template::Crew(re_a) = parse_template(&exported_a.toml).unwrap() else {
            panic!("expected crew");
        };
        assert_eq!(re_a.roles[0].runtime, Runtime::ClaudeCode);
        assert_eq!(re_a.slots[0].runtime, None);
        assert_eq!(re_a.slots[1].runtime, Some(Runtime::Codex));
        assert_eq!(re_a.slots[1].model.as_deref(), Some("gpt-5"));
        assert_eq!(re_a.slots[1].effort.as_deref(), Some("high"));
        assert_eq!(re_a.slots[1].codex_speed, Some(CodexSpeed::Fast));
    }

    #[test]
    fn shared_role_import_reply_does_not_duplicate_large_prompts_across_slots() {
        let (_tmp, core) = core_with_installed(&[Runtime::Codex]);
        let big_prompt = "x".repeat(runner_core::MAX_SYSTEM_PROMPT_BYTES);
        let mut toml = String::from(
            "version = 1\nkind = \"crew\"\n[crew]\nname = \"Many Slots\"\n\n[[roles]]\nhandle = \"shared\"\ndisplay_name = \"Shared\"\nruntime = \"codex\"\nsystem_prompt = \"\"\"\n",
        );
        toml.push_str(&big_prompt);
        toml.push_str("\"\"\"\n");
        for i in 0..100 {
            toml.push_str(&format!(
                "\n[[slots]]\nhandle = \"slot-{i}\"\nrole = \"shared\"\n"
            ));
        }
        assert!(toml.len() <= runner_core::template::MAX_TEMPLATE_BYTES);

        let result = import(
            &core,
            TemplateImportInput {
                toml,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.roles.len(), 1);
        assert_eq!(result.slots.len(), 100);
        for slot in &result.slots {
            assert_eq!(slot.role_id, result.roles[0].id);
        }
        let encoded = serde_json::to_vec(&result).unwrap();
        // Reply carries the 16 KiB prompt once, not 100x (~1.6 MiB).
        assert!(
            encoded.len() < 128 * 1024,
            "encoded reply unexpectedly large: {} bytes",
            encoded.len()
        );
    }

    #[test]
    fn conventions_preserve_leading_indentation_and_trailing_newline_through_db_round_trip() {
        let (_tmp, core) = core_with_installed(&[Runtime::Codex]);
        let conventions = "    indented block\n- item\n  - nested\n";
        let toml = format!(
            "version = 1\nkind = \"crew\"\n[crew]\nname = \"Indented\"\nconventions = \"\"\"\n{conventions}\"\"\"\n[[slots]]\nhandle = \"coder\"\nrole = \"coder\"\n[[roles]]\nhandle = \"coder\"\ndisplay_name = \"Coder\"\nruntime = \"codex\"\n"
        );
        let imported = import(
            &core,
            TemplateImportInput {
                toml,
                ..Default::default()
            },
        )
        .unwrap();
        let crew = imported.crew.unwrap();
        assert_eq!(crew.system_prompt_addendum.as_deref(), Some(conventions));

        let exported = export_crew(&core, &crew.id).unwrap();
        let parsed = parse_template(&exported.toml).unwrap();
        let Template::Crew(parsed) = parsed else {
            panic!("expected crew");
        };
        assert_eq!(parsed.crew.conventions.as_deref(), Some(conventions));
    }

    #[test]
    fn export_crew_snapshot_holds_consistent_wal_view_across_concurrent_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = crate::db::open_pool(&tmp.path().join("runner.db")).unwrap();
        let crew_id = {
            let mut conn = pool.get().unwrap();
            let template = parse_template(SAMPLE_CREW_TOML).unwrap();
            let imported = import_with_conn(
                &mut conn,
                template,
                &TemplateImportInput::default(),
                &[Runtime::Codex, Runtime::Pi],
            )
            .unwrap();
            imported.crew.unwrap().id
        };

        let mut reader = pool.get().unwrap();
        let (crew, slots) = read_crew_export_snapshot(&mut reader, &crew_id, || {
            // Delete the crew on a second WAL connection after crew::get succeeds;
            // the reader's deferred transaction must still observe the full roster.
            let mut writer = pool.get().unwrap();
            crate::ops::crew::delete(&mut writer, &crew_id).unwrap();
        })
        .unwrap();
        assert_eq!(crew.name, "Codex Trio");
        assert_eq!(slots.len(), 2);

        let exported = runner_core::template::export_crew(&crew, &slots).unwrap();
        let Template::Crew(parsed) = parse_template(&exported.toml).unwrap() else {
            panic!("expected crew");
        };
        assert_eq!(parsed.slots.len(), 2);
        assert_eq!(parsed.roles.len(), 2);
    }
}
