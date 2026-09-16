use super::logic::crew_name_refresh;
use super::logic::crew_name_state;
use super::logic::move_item;
use super::logic::slot_command_summary;
use super::logic::suggest_slot_handle;
use super::logic::validate_slot_handle;
use super::logic::CrewNameRefresh;
use super::*;
use chrono::Utc;
use runner_backend::model::{Role, Slot};

fn slot_with_role(
    runtime_override: Option<&str>,
    model_override: Option<&str>,
    effort_override: Option<&str>,
) -> SlotWithRole {
    let now = Utc::now();
    SlotWithRole {
        slot: Slot {
            id: "slot".into(),
            crew_id: "crew".into(),
            role_id: "role".into(),
            slot_handle: "coder".into(),
            position: 0,
            lead: true,
            runtime_override: runtime_override.map(str::to_owned),
            model_override: model_override.map(str::to_owned),
            effort_override: effort_override.map(str::to_owned),
            added_at: now,
        },
        role: Role {
            id: "role".into(),
            handle: "coder".into(),
            display_name: "Coder".into(),
            runtime: "codex".into(),
            command: "codex".into(),
            args: vec!["--quiet".into()],
            working_dir: None,
            system_prompt: None,
            env: Default::default(),
            model: None,
            effort: None,
            created_at: now,
            updated_at: now,
        },
    }
}

#[test]
fn slot_handle_validation_matches_the_shipped_contract() {
    for valid in ["", "a", "0", "coder-2", "coder_2", &"a".repeat(32)] {
        assert_eq!(validate_slot_handle(valid), None, "{valid}");
    }
    for invalid in ["Coder", "-coder", "_coder", "coder!", &"a".repeat(33)] {
        assert!(validate_slot_handle(invalid).is_some(), "{invalid}");
    }
}

#[test]
fn slot_handle_suggestions_take_the_first_available_suffix() {
    let taken = HashSet::from([
        "coder".to_owned(),
        "coder-2".to_owned(),
        "coder-3".to_owned(),
    ]);

    assert_eq!(suggest_slot_handle("reviewer", &taken), "reviewer");
    assert_eq!(suggest_slot_handle("coder", &taken), "coder-4");
}

#[test]
fn crew_name_state_tracks_saved_reverted_and_empty_edits() {
    assert_eq!(crew_name_state("Crew", "Crew"), (false, false, false));
    assert_eq!(crew_name_state("New crew", "Crew"), (true, true, false));
    assert_eq!(crew_name_state(" Crew ", "Crew"), (true, false, false));
    assert_eq!(crew_name_state("  ", "Crew"), (true, false, true));
}

#[test]
fn crew_name_refresh_preserves_live_edits_without_orphaning_the_field() {
    assert_eq!(
        crew_name_refresh("Draft", "Crew", true),
        CrewNameRefresh::Preserve
    );
    assert_eq!(
        crew_name_refresh("Old", "New", false),
        CrewNameRefresh::Reset
    );
    assert_eq!(
        crew_name_refresh("Saved", "Saved", true),
        CrewNameRefresh::MarkClean
    );
}

#[test]
fn slot_command_summary_applies_runtime_and_model_effort_layers() {
    assert_eq!(
        slot_command_summary(&slot_with_role(None, None, None)),
        "codex --quiet"
    );
    assert_eq!(
        slot_command_summary(&slot_with_role(Some("codex"), Some("gpt-5"), Some("high"))),
        "codex --quiet (model gpt-5 · effort high)"
    );
    assert_eq!(
        slot_command_summary(&slot_with_role(Some("claude-code"), None, None)),
        "claude (runtime defaults)"
    );
    assert_eq!(
        slot_command_summary(&slot_with_role(
            Some("claude-code"),
            Some("opus"),
            Some("max")
        )),
        "claude (runtime defaults · model opus · effort max)"
    );
}

#[test]
fn move_item_reorders_slots_in_both_directions() {
    assert_eq!(move_item(&["a", "b", "c", "d"], 1, 3), ["a", "c", "d", "b"]);
    assert_eq!(move_item(&["a", "b", "c", "d"], 3, 1), ["a", "d", "b", "c"]);
}

#[test]
fn move_item_ignores_same_or_invalid_positions() {
    assert_eq!(move_item(&[1, 2, 3], 1, 1), [1, 2, 3]);
    assert_eq!(move_item(&[1, 2, 3], 8, 1), [1, 2, 3]);
    assert_eq!(move_item(&[1, 2, 3], 1, 8), [1, 2, 3]);
}
