use std::collections::HashSet;

use chrono::Utc;
use runner_core::protocol::onboarding::{
    CreateStarterCrewArgs, OnboardingStatus, StarterCrewProposal, StarterCrewSlotProposal,
};
use runner_core::protocol::permissions::PermissionMode;
use runner_core::protocol::role::{default_permission_mode, CreateRoleInput};
use runner_core::protocol::runtime::{
    RuntimeExecutableStatus, RuntimeRowState, RuntimeSignInStatus,
};
use rusqlite::TransactionBehavior;
use ulid::Ulid as UlidGen;

use crate::error::Result;
use crate::model::{Crew, Runtime};
use crate::ops::crew::{create as create_crew, CreateCrewInput};
use crate::ops::role::create as create_role;
use crate::repo;
use crate::AppCore;

const STARTER_CREW_BASE_NAME: &str = "Pair coding";

pub fn status(core: &AppCore) -> Result<OnboardingStatus> {
    let conn = core.db.get()?;
    let completed_at = crate::db::onboarding_completed_at_conn(&conn)?;
    let has_crews = repo::crew::count(&conn)? > 0;
    Ok(OnboardingStatus {
        completed: completed_at.is_some(),
        completed_at,
        has_crews,
    })
}

pub fn complete(core: &AppCore) -> Result<OnboardingStatus> {
    if crate::db::mark_onboarding_completed(&core.db)? {
        core.events.emit("onboarding/changed", &());
    }
    status(core)
}

pub(crate) fn mark_completed_on_activity(core: &AppCore) -> Result<()> {
    if crate::db::mark_onboarding_completed(&core.db)? {
        core.events.emit("onboarding/changed", &());
    }
    Ok(())
}

pub fn starter_crew_proposal(
    core: &AppCore,
    args: CreateStarterCrewArgs,
) -> Result<StarterCrewProposal> {
    // Query runtime status before checking out a DB connection so the
    // connection pool (max_size = 1 in unit tests) never deadlocks.
    let status_list = crate::ops::runtime::runtime_status_list(core)?;
    let conn = core.db.get()?;
    let existing_crews = repo::crew::list_with_role_count(&conn)?
        .into_iter()
        .map(|(crew, _)| crew.name)
        .collect::<Vec<_>>();
    Ok(build_proposal(
        &status_list.runtimes,
        &args,
        &existing_crews,
    ))
}

pub fn create_starter_crew(core: &AppCore, args: CreateStarterCrewArgs) -> Result<Option<Crew>> {
    let status_list = crate::ops::runtime::runtime_status_list(core)?;
    let Some((coder, reviewer)) = pick_starter_pair(&status_list.runtimes, &args) else {
        return Ok(None);
    };

    let crew = {
        let mut conn = core.db.get()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_crews = repo::crew::list_with_role_count(&tx)?
            .into_iter()
            .map(|(crew, _)| crew.name)
            .collect::<Vec<_>>();
        let crew_name = next_unique_crew_name(&existing_crews);

        let mut used_handles = repo::role::list(&tx)?
            .into_iter()
            .map(|role| role.handle)
            .collect::<HashSet<_>>();
        let coder_handle = next_unique_role_handle(&mut used_handles, "coder");
        let reviewer_handle = next_unique_role_handle(&mut used_handles, "reviewer");

        let coder_role = create_role(
            &tx,
            CreateRoleInput {
                handle: coder_handle,
                display_name: "Coder".to_string(),
                runtime: coder.name,
                command: coder.command.clone(),
                args: Vec::new(),
                working_dir: None,
                system_prompt: Some(crate::db::SEED_CODER_PROMPT.trim_end().to_string()),
                env: Default::default(),
                model: None,
                effort: None,
                codex_speed: None,
                permission_mode: starter_permission_mode(coder.name),
            },
        )?;
        let reviewer_role = create_role(
            &tx,
            CreateRoleInput {
                handle: reviewer_handle,
                display_name: "Reviewer".to_string(),
                runtime: reviewer.name,
                command: reviewer.command.clone(),
                args: Vec::new(),
                working_dir: None,
                system_prompt: Some(crate::db::SEED_REVIEWER_PROMPT.trim_end().to_string()),
                env: Default::default(),
                model: None,
                effort: None,
                codex_speed: None,
                permission_mode: starter_permission_mode(reviewer.name),
            },
        )?;

        let crew = create_crew(
            &tx,
            CreateCrewInput {
                name: crew_name,
                system_prompt_addendum: Some(crate::db::SEED_CREW_ADDENDUM.trim_end().to_string()),
            },
        )?;

        let added_at = Utc::now();
        for (position, (slot_handle, role_id)) in
            [("coder", coder_role.id), ("reviewer", reviewer_role.id)]
                .into_iter()
                .enumerate()
        {
            repo::slot::insert(
                &tx,
                &repo::slot::SlotRow {
                    id: UlidGen::new().to_string(),
                    crew_id: crew.id.clone(),
                    role_id,
                    slot_handle: slot_handle.to_string(),
                    position: position as i64,
                    lead: false,
                    runtime_override: None,
                    model_override: None,
                    effort_override: None,
                    codex_speed_override: None,
                    added_at,
                },
            )?;
        }

        tx.commit()?;
        crew
    };

    core.events.emit("role/changed", &());
    core.events.emit("crew/changed", &());
    core.events.emit("slot/changed", &());
    core.events.emit("onboarding/changed", &());
    Ok(Some(crew))
}

fn eligible_ranked_runtimes<'a>(
    statuses: &'a [RuntimeExecutableStatus],
    args: &CreateStarterCrewArgs,
) -> Vec<&'a RuntimeExecutableStatus> {
    let mut eligible = statuses
        .iter()
        .filter(|entry| {
            matches!(
                entry.state,
                RuntimeRowState::Detected | RuntimeRowState::Override
            ) && !entry.below_minimum
                && args
                    .enabled_runtimes
                    .as_ref()
                    .is_none_or(|enabled| enabled.contains(&entry.name))
        })
        .collect::<Vec<_>>();
    eligible.sort_by_key(|entry| {
        (
            entry.sign_in == RuntimeSignInStatus::SignInNeeded,
            Some(entry.name) != args.default_runtime,
            entry.sign_in != RuntimeSignInStatus::SignedIn,
            entry.name.setup_rank(),
        )
    });
    eligible
}

fn pick_starter_pair<'a>(
    statuses: &'a [RuntimeExecutableStatus],
    args: &CreateStarterCrewArgs,
) -> Option<(&'a RuntimeExecutableStatus, &'a RuntimeExecutableStatus)> {
    let ranked = eligible_ranked_runtimes(statuses, args);
    let coder = *ranked.first()?;
    let reviewer = ranked
        .iter()
        .copied()
        .find(|entry| entry.name != coder.name)
        .unwrap_or(coder);
    Some((coder, reviewer))
}

fn build_proposal(
    statuses: &[RuntimeExecutableStatus],
    args: &CreateStarterCrewArgs,
    existing_crews: &[String],
) -> StarterCrewProposal {
    let name = next_unique_crew_name(existing_crews);
    let slots = match pick_starter_pair(statuses, args) {
        Some((coder, reviewer)) => vec![
            StarterCrewSlotProposal {
                handle: "coder".to_string(),
                display_name: "Coder".to_string(),
                runtime: coder.name,
            },
            StarterCrewSlotProposal {
                handle: "reviewer".to_string(),
                display_name: "Reviewer".to_string(),
                runtime: reviewer.name,
            },
        ],
        None => Vec::new(),
    };
    StarterCrewProposal { name, slots }
}

fn starter_permission_mode(runtime: Runtime) -> PermissionMode {
    if runtime == Runtime::ClaudeCode {
        PermissionMode::AcceptEdits
    } else {
        default_permission_mode()
    }
}

fn next_unique_crew_name(existing_names: &[String]) -> String {
    let taken = |candidate: &str| {
        existing_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(candidate))
    };
    if !taken(STARTER_CREW_BASE_NAME) {
        return STARTER_CREW_BASE_NAME.to_string();
    }
    for suffix in 2.. {
        let candidate = format!("{STARTER_CREW_BASE_NAME} {suffix}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn next_unique_role_handle(used: &mut HashSet<String>, base: &str) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_core::protocol::runtime::{RuntimeLaunchOptions, RuntimeRowState};
    use std::path::Path;

    fn status_entry(
        runtime: Runtime,
        state: RuntimeRowState,
        sign_in: RuntimeSignInStatus,
        below_minimum: bool,
    ) -> RuntimeExecutableStatus {
        let command = runtime.command().unwrap_or(runtime.key());
        RuntimeExecutableStatus {
            name: runtime,
            launch_options: RuntimeLaunchOptions::default(),
            display_name: runtime.display_name().to_string(),
            command: command.to_string(),
            default_model: None,
            default_effort: None,
            detected_path: (state == RuntimeRowState::Detected)
                .then(|| format!("/usr/local/bin/{command}")),
            override_path: None,
            effective_command: matches!(
                state,
                RuntimeRowState::Detected | RuntimeRowState::Override
            )
            .then(|| command.to_string()),
            effective_source: None,
            state,
            invalid_reason: None,
            installed_version: Some("9.9.9".to_string()),
            available_version: None,
            min_version: None,
            below_minimum,
            sign_in,
        }
    }

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

    #[test]
    fn proposal_prefers_signed_in_and_default_and_two_distinct_runtimes() {
        let statuses = vec![
            status_entry(
                Runtime::ClaudeCode,
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                false,
            ),
            status_entry(
                Runtime::Codex,
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignInNeeded,
                false,
            ),
            status_entry(
                Runtime::Pi,
                RuntimeRowState::Detected,
                RuntimeSignInStatus::Unknown,
                false,
            ),
        ];

        // Even when Codex is the default runtime, SignInNeeded drops it behind SignedIn and Unknown.
        let proposal = build_proposal(
            &statuses,
            &CreateStarterCrewArgs {
                default_runtime: Some(Runtime::Codex),
                enabled_runtimes: None,
            },
            &[],
        );
        assert_eq!(proposal.name, "Pair coding");
        assert_eq!(proposal.slots[0].runtime, Runtime::ClaudeCode);
        assert_eq!(proposal.slots[1].runtime, Runtime::Pi);

        // When Codex is Unknown (e.g., stored credentials unverified) and default, it leads over Unknown Pi while SignedIn still outranks non-default.
        let statuses_default = vec![
            status_entry(
                Runtime::ClaudeCode,
                RuntimeRowState::Detected,
                RuntimeSignInStatus::Unknown,
                false,
            ),
            status_entry(
                Runtime::Codex,
                RuntimeRowState::Detected,
                RuntimeSignInStatus::Unknown,
                false,
            ),
        ];
        let proposal_default = build_proposal(
            &statuses_default,
            &CreateStarterCrewArgs {
                default_runtime: Some(Runtime::Codex),
                enabled_runtimes: None,
            },
            &[],
        );
        assert_eq!(proposal_default.slots[0].runtime, Runtime::Codex);
        assert_eq!(proposal_default.slots[1].runtime, Runtime::ClaudeCode);
    }

    #[test]
    fn proposal_falls_back_to_same_runtime_and_handles_empty_or_disabled_set() {
        let single = vec![status_entry(
            Runtime::Codex,
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            false,
        )];
        let proposal = build_proposal(&single, &CreateStarterCrewArgs::default(), &[]);
        assert_eq!(proposal.slots.len(), 2);
        assert_eq!(proposal.slots[0].runtime, Runtime::Codex);
        assert_eq!(proposal.slots[1].runtime, Runtime::Codex);

        let below_min = vec![status_entry(
            Runtime::Codex,
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            true,
        )];
        let empty = build_proposal(&below_min, &CreateStarterCrewArgs::default(), &[]);
        assert!(empty.slots.is_empty());

        let disabled = build_proposal(
            &single,
            &CreateStarterCrewArgs {
                default_runtime: None,
                enabled_runtimes: Some(vec![Runtime::ClaudeCode]),
            },
            &[],
        );
        assert!(disabled.slots.is_empty());
    }

    #[test]
    fn create_starter_crew_creates_roles_slots_and_avoids_name_collisions() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_binary(tmp.path(), Runtime::ClaudeCode.command().unwrap());
        write_fake_binary(tmp.path(), Runtime::Codex.command().unwrap());

        let core = crate::test_support::test_core();
        {
            let mut env = core.runtime_shell_env.write().unwrap();
            env.path = Some(tmp.path().display().to_string());
            env.vars.insert(
                crate::runtime_status::TEST_ISOLATE_PATH_ENV.into(),
                "1".into(),
            );
            env.vars
                .insert("HOME".into(), tmp.path().display().to_string());
            env.vars
                .insert("OPENAI_API_KEY".into(), "sk-test-token".into());
        }
        core.runtime_discovery.write().unwrap().checking = false;

        let mut events = core.events.subscribe();
        let initial = status(&core).unwrap();
        assert!(!initial.completed);
        assert!(!initial.has_crews);

        let crew = create_starter_crew(&core, CreateStarterCrewArgs::default())
            .unwrap()
            .expect("starter crew should be created");
        assert_eq!(crew.name, "Pair coding");
        assert_eq!(
            crew.system_prompt_addendum.as_deref(),
            Some(crate::db::SEED_CREW_ADDENDUM.trim_end())
        );

        let conn = core.db.get().unwrap();
        let slots = crate::ops::slot::list(&conn, &crew.id).unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].slot.slot_handle, "coder");
        assert_eq!(slots[1].slot.slot_handle, "reviewer");
        let codex_role = crate::ops::role::get_by_handle(&conn, "coder").unwrap();
        let claude_role = crate::ops::role::get_by_handle(&conn, "reviewer").unwrap();
        // Codex is SignedIn via OPENAI_API_KEY, so it takes @coder while Claude Code takes @reviewer.
        assert_eq!(codex_role.runtime, Runtime::Codex.key());
        assert_eq!(claude_role.runtime, Runtime::ClaudeCode.key());
        assert!(
            claude_role.args.contains(&"acceptEdits".to_string()),
            "claude starter role should default to AcceptEdits, got {:?}",
            claude_role.args
        );
        drop(conn);

        // Creating a second starter crew increments the crew name and role handles cleanly.
        let crew2 = create_starter_crew(&core, CreateStarterCrewArgs::default())
            .unwrap()
            .expect("second starter crew should avoid name and handle collisions");
        assert_eq!(crew2.name, "Pair coding 2");
        let conn = core.db.get().unwrap();
        assert_eq!(
            crate::ops::role::get_by_handle(&conn, "coder-2")
                .unwrap()
                .handle,
            "coder-2"
        );
        assert_eq!(
            crate::ops::role::get_by_handle(&conn, "reviewer-2")
                .unwrap()
                .handle,
            "reviewer-2"
        );
        drop(conn);

        let after_create = status(&core).unwrap();
        assert!(after_create.has_crews);
        assert!(!after_create.completed);

        let completed = complete(&core).unwrap();
        assert!(completed.completed);
        let first_ts = completed.completed_at.clone().unwrap();
        let completed_again = complete(&core).unwrap();
        assert_eq!(
            completed_again.completed_at.as_deref(),
            Some(first_ts.as_str())
        );

        let mut seen = Vec::new();
        while let Ok(evt) = events.try_recv() {
            seen.push(evt.name);
        }
        assert!(seen.contains(&"role/changed"));
        assert!(seen.contains(&"crew/changed"));
        assert_eq!(
            seen.iter()
                .filter(|name| **name == "onboarding/changed")
                .count(),
            3 // 2 starter crews + 1 first `complete` call (second `complete` is idempotent no-op)
        );
    }

    #[test]
    fn create_starter_crew_returns_none_when_no_runtime_is_eligible() {
        let tmp = tempfile::tempdir().unwrap();
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

        assert!(create_starter_crew(&core, CreateStarterCrewArgs::default())
            .unwrap()
            .is_none());
        let s = status(&core).unwrap();
        assert!(!s.has_crews);
        assert!(!s.completed);
    }
}
