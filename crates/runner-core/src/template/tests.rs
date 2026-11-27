use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::Utc;

use crate::protocol::{CodexSpeed, Crew, Role, Slot, SlotWithRole};
use crate::{Runtime, MAX_SYSTEM_PROMPT_BYTES};

use super::*;

fn sample_role(handle: &str, runtime: &str) -> Role {
    let now = Utc::now();
    Role {
        id: format!("role-{handle}"),
        handle: handle.to_owned(),
        display_name: format!("Role {handle}"),
        runtime: runtime.to_owned(),
        command: "custom-secret-cmd".to_owned(),
        args: vec!["--dangerously-skip-permissions".into()],
        working_dir: Some("/Users/secret/project".into()),
        system_prompt: Some("Line 1\n\nLine 2 with \"quotes\" and \\backslash\\.".into()),
        env: HashMap::from([("API_KEY".into(), "super-secret-value".into())]),
        model: Some("gpt-6.2-astra".into()),
        effort: Some("xhigh".into()),
        codex_speed: (runtime == "codex").then_some(CodexSpeed::Fast),
        created_at: now,
        updated_at: now,
    }
}

fn sample_crew_and_slots() -> (Crew, Vec<SlotWithRole>) {
    let now = Utc::now();
    let crew = Crew {
        id: "crew-01".into(),
        name: "Codex Trio".into(),
        system_prompt_addendum: Some("Step 1: implement.\nStep 2: review.".into()),
        created_at: now,
        updated_at: now,
    };
    let coder_role = sample_role("impl-codex", "codex");
    let reviewer_role = sample_role("review-claude", "claude-code");
    let slots = vec![
        SlotWithRole {
            slot: Slot {
                id: "slot-2".into(),
                crew_id: crew.id.clone(),
                role_id: reviewer_role.id.clone(),
                slot_handle: "reviewer".into(),
                position: 1,
                lead: false,
                runtime_override: Some("codex".into()),
                model_override: Some("gpt-5.5".into()),
                effort_override: Some("high".into()),
                codex_speed_override: Some(CodexSpeed::Standard),
                added_at: now,
            },
            role: reviewer_role,
        },
        SlotWithRole {
            slot: Slot {
                id: "slot-1".into(),
                crew_id: crew.id.clone(),
                role_id: coder_role.id.clone(),
                slot_handle: "coder".into(),
                position: 0,
                lead: true,
                runtime_override: None,
                model_override: None,
                effort_override: None,
                codex_speed_override: None,
                added_at: now,
            },
            role: coder_role,
        },
    ];
    (crew, slots)
}

#[test]
fn crew_export_round_trips_and_omits_machine_local_fields() {
    let (crew, slots) = sample_crew_and_slots();
    let exported = export_crew(&crew, &slots).unwrap();
    assert_eq!(exported.filename, "codex-trio.crew.toml");

    // Excluded fields and secret values must never appear in exported TOML.
    for forbidden in [
        "crew-01",
        "role-impl-codex",
        "slot-1",
        "created_at",
        "updated_at",
        "added_at",
        "working_dir",
        "/Users/secret/project",
        "API_KEY",
        "super-secret-value",
        "custom-secret-cmd",
        "dangerously-skip-permissions",
        "permission_mode",
        "lead =",
    ] {
        assert!(
            !exported.toml.contains(forbidden),
            "exported TOML leaked {forbidden:?}:\n{}",
            exported.toml
        );
    }

    let parsed = parse_template(&exported.toml).unwrap();
    let Template::Crew(parsed_crew) = parsed else {
        panic!("expected crew template");
    };
    assert_eq!(parsed_crew.crew.name, "Codex Trio");
    assert_eq!(
        parsed_crew.crew.conventions.as_deref(),
        Some("Step 1: implement.\nStep 2: review.")
    );
    // Slots sorted by position (coder = 0, reviewer = 1), roles in first-referenced slot order.
    assert_eq!(parsed_crew.slots.len(), 2);
    assert_eq!(parsed_crew.slots[0].handle, "coder");
    assert_eq!(parsed_crew.slots[0].role, "impl-codex");
    assert_eq!(parsed_crew.slots[1].handle, "reviewer");
    assert_eq!(parsed_crew.slots[1].role, "review-claude");
    assert_eq!(parsed_crew.slots[1].runtime, Some(Runtime::Codex));
    assert_eq!(parsed_crew.slots[1].model.as_deref(), Some("gpt-5.5"));
    assert_eq!(parsed_crew.slots[1].effort.as_deref(), Some("high"));
    assert_eq!(parsed_crew.slots[1].codex_speed, Some(CodexSpeed::Standard));

    assert_eq!(parsed_crew.roles.len(), 2);
    assert_eq!(parsed_crew.roles[0].handle, "impl-codex");
    assert_eq!(parsed_crew.roles[0].codex_speed, Some(CodexSpeed::Fast));
    assert_eq!(parsed_crew.roles[1].handle, "review-claude");
    assert_eq!(parsed_crew.roles[1].codex_speed, None);
    assert_eq!(
        parsed_crew.roles[0].system_prompt.as_deref(),
        Some("Line 1\n\nLine 2 with \"quotes\" and \\backslash\\.")
    );
}

#[test]
fn crew_golden_toml_snapshot() {
    let template = Template::Crew(CrewTemplate {
        meta: TemplateMeta {
            description: Some("Two-agent coding loop.".into()),
            author: Some("Runner".into()),
            tags: vec!["coding".into(), "review".into()],
            homepage: Some("https://runnersh.dev/crews".into()),
        },
        crew: CrewSection {
            name: "Pair Coding".into(),
            conventions: Some("First line.\nSecond line.".into()),
        },
        slots: vec![
            SlotTemplate {
                handle: "coder".into(),
                role: "impl".into(),
                runtime: None,
                model: None,
                effort: None,
                codex_speed: None,
            },
            SlotTemplate {
                handle: "reviewer".into(),
                role: "review".into(),
                runtime: Some(Runtime::Codex),
                model: Some("gpt-6.2-astra".into()),
                effort: Some("xhigh".into()),
                codex_speed: Some(CodexSpeed::Fast),
            },
        ],
        roles: vec![
            RoleTemplate {
                handle: "impl".into(),
                display_name: "Implementer".into(),
                runtime: Runtime::ClaudeCode,
                model: Some("opus".into()),
                effort: None,
                codex_speed: None,
                system_prompt: Some("Write code.\nRun tests.".into()),
            },
            RoleTemplate {
                handle: "review".into(),
                display_name: "Reviewer".into(),
                runtime: Runtime::ClaudeCode,
                model: None,
                effort: None,
                codex_speed: None,
                system_prompt: Some("Review only.".into()),
            },
        ],
    });

    let emitted = format_template(&template).unwrap();
    let expected = r#"version = 1
kind = "crew"

[meta]
description = "Two-agent coding loop."
author = "Runner"
tags = ["coding", "review"]
homepage = "https://runnersh.dev/crews"

[crew]
name = "Pair Coding"
conventions = """
First line.
Second line."""

[[slots]]
handle = "coder"
role = "impl"

[[slots]]
handle = "reviewer"
role = "review"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
codex_speed = "fast"

[[roles]]
handle = "impl"
display_name = "Implementer"
runtime = "claude-code"
model = "opus"
system_prompt = """
Write code.
Run tests."""

[[roles]]
handle = "review"
display_name = "Reviewer"
runtime = "claude-code"
system_prompt = "Review only."
"#;
    assert_eq!(emitted, expected);
    assert_eq!(parse_template(&emitted).unwrap(), template);
}

#[test]
fn role_export_and_tricky_multiline_prompts_round_trip() {
    let mut role = sample_role("review-codex", "codex");
    role.system_prompt = Some(
        "Intro with \"\"\"triple quotes\"\"\"\nAnd trailing quote\"\nBackslash \\ and \t tab\n\u{200B}zero-width\nEnds with two quotes\"\""
            .into(),
    );
    let exported = export_role(&role).unwrap();
    assert_eq!(exported.filename, "review-codex.role.toml");

    let parsed = parse_template(&exported.toml).unwrap();
    let Template::Role(parsed_role) = parsed else {
        panic!("expected role template");
    };
    assert_eq!(parsed_role.role.system_prompt, role.system_prompt);
}

#[test]
fn empty_crew_round_trips() {
    let now = Utc::now();
    let crew = Crew {
        id: "c1".into(),
        name: "Empty Crew".into(),
        system_prompt_addendum: None,
        created_at: now,
        updated_at: now,
    };
    let exported = export_crew(&crew, &[]).unwrap();
    let parsed = parse_template(&exported.toml).unwrap();
    let Template::Crew(parsed) = parsed else {
        panic!("expected crew");
    };
    assert_eq!(parsed.crew.name, "Empty Crew");
    assert!(parsed.slots.is_empty());
    assert!(parsed.roles.is_empty());
}

#[test]
fn unknown_meta_keys_are_ignored() {
    let toml = r#"
version = 1
kind = "role"

[meta]
description = "Test"
future_field = 42
nested = { anything = true }

[role]
handle = "coder"
display_name = "Coder"
runtime = "codex"
"#;
    let parsed = parse_template(toml).unwrap();
    assert_eq!(parsed.meta().description.as_deref(), Some("Test"));
}

#[test]
fn validation_rejects_version_mismatches_and_bad_sections() {
    let err = parse_template("version = 2\nkind = \"role\"\n[role]\nhandle = \"a\"\ndisplay_name = \"A\"\nruntime = \"codex\"\n")
        .unwrap_err();
    assert_eq!(
        err.message,
        "Unsupported template version 2; update Runner to import this file."
    );
    assert_eq!(err.line, Some(1));

    let err = parse_template(
        "version = 1\nkind = \"crew\"\n[crew]\nname = \"X\"\n[role]\nhandle = \"a\"\ndisplay_name = \"A\"\nruntime = \"codex\"\n",
    )
    .unwrap_err();
    assert!(err.message.contains("[role]"), "{err}");
    assert_eq!(err.line, Some(5));

    let err = parse_template(
        "version = 1\nkind = \"role\"\n[crew]\nname = \"X\"\n[role]\nhandle = \"a\"\ndisplay_name = \"A\"\nruntime = \"codex\"\n",
    )
    .unwrap_err();
    assert!(err.message.contains("[crew]"), "{err}");
    assert_eq!(err.line, Some(3));
}

#[test]
fn validation_rejects_unknown_and_excluded_keys_with_lines() {
    for key in [
        "id = \"x\"",
        "created_at = \"2026-01-01T00:00:00Z\"",
        "updated_at = \"2026-01-01T00:00:00Z\"",
        "working_dir = \"/tmp\"",
        "env = { SECRET = \"1\" }",
        "command = \"sh\"",
        "args = [\"-c\"]",
        "lead = true",
        "permission_mode = \"bypass\"",
        "effrot = \"high\"",
    ] {
        let toml = format!(
            "version = 1\nkind = \"role\"\n\n[role]\nhandle = \"helper\"\ndisplay_name = \"Helper\"\nruntime = \"codex\"\n{key}\n"
        );
        let err = parse_template(&toml).unwrap_err();
        assert!(err.message.contains("@helper"), "for {key}: {err}");
        assert_eq!(err.line, Some(8), "for {key}: {err}");
    }
}

#[test]
fn validation_rejects_shell_unknown_runtime_codex_speed_and_limits() {
    let err = parse_template(
        "version = 1\nkind = \"role\"\n[role]\nhandle = \"helper\"\ndisplay_name = \"Helper\"\nruntime = \"shell\"\n",
    )
    .unwrap_err();
    assert!(err.message.contains("shell"), "{err}");
    assert!(err.message.contains("@helper"), "{err}");

    let err = parse_template(
        "version = 1\nkind = \"role\"\n[role]\nhandle = \"helper\"\ndisplay_name = \"Helper\"\nruntime = \"custom-bot\"\n",
    )
    .unwrap_err();
    assert_eq!(
        err.message,
        "Unknown runtime \"custom-bot\" on role @helper."
    );
    assert_eq!(err.line, Some(6));

    let err = parse_template(
        "version = 1\nkind = \"role\"\n[role]\nhandle = \"helper\"\ndisplay_name = \"Helper\"\nruntime = \"claude-code\"\ncodex_speed = \"fast\"\n",
    )
    .unwrap_err();
    assert!(err.message.contains("codex_speed"), "{err}");

    // Slot overriding a Codex role to Claude Code cannot carry codex_speed.
    let err = parse_template(
        r#"
version = 1
kind = "crew"
[crew]
name = "Bad Speed"
[[slots]]
handle = "coder"
role = "impl"
runtime = "claude-code"
codex_speed = "fast"
[[roles]]
handle = "impl"
display_name = "Impl"
runtime = "codex"
"#,
    )
    .unwrap_err();
    assert!(err.message.contains("codex_speed"), "{err}");
    assert!(err.message.contains("@coder"), "{err}");

    // Oversized system_prompt (> 16 KiB).
    let big_prompt = "a".repeat(MAX_SYSTEM_PROMPT_BYTES + 1);
    let toml = format!(
        "version = 1\nkind = \"role\"\n[role]\nhandle = \"big\"\ndisplay_name = \"Big\"\nruntime = \"codex\"\nsystem_prompt = \"{big_prompt}\"\n"
    );
    let err = parse_template(&toml).unwrap_err();
    assert!(err.message.contains("16 KiB"), "{err}");

    // Oversized file (> 256 KiB).
    let huge = " ".repeat(MAX_TEMPLATE_BYTES + 1);
    let err = parse_template(&huge).unwrap_err();
    assert!(err.message.contains("256 KiB"), "{err}");
}

#[test]
fn validation_rejects_dangling_unused_duplicate_and_invalid_handles() {
    // Dangling slot role reference.
    let err = parse_template(
        r#"
version = 1
kind = "crew"
[crew]
name = "Dangling"
[[slots]]
handle = "coder"
role = "missing"
"#,
    )
    .unwrap_err();
    assert!(err.message.contains("@missing"), "{err}");

    // Unused role.
    let err = parse_template(
        r#"
version = 1
kind = "crew"
[crew]
name = "Unused"
[[roles]]
handle = "orphan"
display_name = "Orphan"
runtime = "codex"
"#,
    )
    .unwrap_err();
    assert!(err.message.contains("@orphan"), "{err}");

    // Duplicate slot handle.
    let err = parse_template(
        r#"
version = 1
kind = "crew"
[crew]
name = "Dup"
[[slots]]
handle = "coder"
role = "impl"
[[slots]]
handle = "coder"
role = "impl"
[[roles]]
handle = "impl"
display_name = "Impl"
runtime = "codex"
"#,
    )
    .unwrap_err();
    assert!(err.message.contains("Duplicate slot handle"), "{err}");

    // Invalid legacy slot handle fails on export self-validation.
    let (crew, mut slots) = sample_crew_and_slots();
    slots[0].slot.slot_handle = "Bad Handle".into();
    let err = export_crew(&crew, &slots).unwrap_err();
    assert!(err.message.contains("Bad Handle"), "{err}");
}

#[test]
fn handle_allocator_handles_32_char_truncation_and_collisions() {
    let long = "a".repeat(32);
    let mut used = HashSet::from([long.clone()]);
    let second = allocate_role_handle(&long, &mut used);
    assert_eq!(second.len(), 32);
    assert!(second.ends_with("-2"));
    assert!(crate::slot_handle::validate_slot_handle(&second).is_none());

    let third = allocate_role_handle(&long, &mut used);
    assert_eq!(third.len(), 32);
    assert!(third.ends_with("-3"));

    assert_eq!(suggest_crew_name("Codex Trio", &[]), "Codex Trio");
    assert_eq!(
        suggest_crew_name("Codex Trio", &["codex trio".into()]),
        "Codex Trio (2)"
    );
    assert_eq!(
        suggest_crew_name(
            "Codex Trio",
            &["Codex Trio".into(), "Codex Trio (2)".into()]
        ),
        "Codex Trio (3)"
    );
}

#[test]
fn resolve_crew_import_handles_shared_role_divergent_runtimes() {
    let template = CrewTemplate {
        meta: TemplateMeta::default(),
        crew: CrewSection {
            name: "Game".into(),
            conventions: None,
        },
        slots: vec![
            SlotTemplate {
                handle: "player-1".into(),
                role: "player".into(),
                runtime: None,
                model: Some("deepseek-chat".into()),
                effort: Some("high".into()),
                codex_speed: None,
            },
            SlotTemplate {
                handle: "player-2".into(),
                role: "player".into(),
                runtime: None,
                model: None,
                effort: None,
                codex_speed: None,
            },
        ],
        roles: vec![RoleTemplate {
            handle: "player".into(),
            display_name: "Player".into(),
            runtime: Runtime::Codex,
            model: Some("gpt-6.2-astra".into()),
            effort: Some("xhigh".into()),
            codex_speed: Some(CodexSpeed::Fast),
            system_prompt: Some("Play well.".into()),
        }],
    };

    let overrides = BTreeMap::from([
        ("player-1".into(), Runtime::ClaudeCode),
        ("player-2".into(), Runtime::Pi),
    ]);
    let resolved = resolve_crew_import(&template, &overrides).unwrap();
    assert_eq!(resolved.roles.len(), 1);
    assert_eq!(resolved.roles[0].runtime, Runtime::ClaudeCode);
    assert_eq!(resolved.roles[0].model, None);
    assert_eq!(resolved.roles[0].effort, None);
    assert_eq!(resolved.roles[0].codex_speed, None);

    assert_eq!(resolved.slots[0].runtime_override, None);
    assert_eq!(resolved.slots[0].model_override, None);
    assert_eq!(resolved.slots[0].effort_override, None);
    assert_eq!(resolved.slots[1].runtime_override, Some(Runtime::Pi));
    assert_eq!(resolved.slots[1].model_override, None);
    assert_eq!(resolved.slots[1].effort_override, None);
}

#[test]
fn conventions_preserve_leading_indentation_and_trailing_newline() {
    let conventions = "    indented code\n- first\n  - nested\n";
    let template = Template::Crew(CrewTemplate {
        meta: TemplateMeta::default(),
        crew: CrewSection {
            name: "Indented".into(),
            conventions: Some(conventions.into()),
        },
        slots: vec![SlotTemplate {
            handle: "coder".into(),
            role: "coder".into(),
            runtime: None,
            model: None,
            effort: None,
            codex_speed: None,
        }],
        roles: vec![RoleTemplate {
            handle: "coder".into(),
            display_name: "Coder".into(),
            runtime: Runtime::Codex,
            model: None,
            effort: None,
            codex_speed: None,
            system_prompt: Some("Work.\n".into()),
        }],
    });
    let emitted = format_template(&template).unwrap();
    let parsed = parse_template(&emitted).unwrap();
    let Template::Crew(parsed_crew) = parsed else {
        panic!("expected crew");
    };
    assert_eq!(parsed_crew.crew.conventions.as_deref(), Some(conventions));
}

#[test]
fn read_template_bounded_stops_at_size_limit_and_rejects_non_utf8() {
    let ok_data = b"version = 1\nkind = \"role\"\n";
    assert_eq!(
        read_template_bounded(&ok_data[..]).unwrap(),
        "version = 1\nkind = \"role\"\n"
    );

    let oversized = vec![b' '; MAX_TEMPLATE_BYTES + 1024];
    let err = read_template_bounded(&oversized[..]).unwrap_err();
    assert!(err.message.contains("256 KiB"), "{err}");

    let bad_utf8 = [0xff, 0xfe, 0xfd];
    let err = read_template_bounded(&bad_utf8[..]).unwrap_err();
    assert!(err.message.contains("UTF-8"), "{err}");
}

#[test]
fn every_example_crew_template_is_valid_and_curated() {
    let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&examples_dir).unwrap() {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        let slug = entry.file_name().to_string_lossy().into_owned();
        let template_path = entry.path().join(format!("{slug}.crew.toml"));
        assert!(
            template_path.is_file(),
            "missing crew template at {}",
            template_path.display()
        );
        let raw = std::fs::read_to_string(&template_path).unwrap();
        assert!(
            !raw.contains("/Users/") && !raw.contains("C:\\Users\\"),
            "{slug} template must not contain personal filesystem paths"
        );
        let parsed = parse_template(&raw)
            .unwrap_or_else(|err| panic!("invalid {}: {err}", template_path.display()));
        let Template::Crew(crew) = parsed else {
            panic!("{} must be kind = \"crew\"", template_path.display());
        };
        assert!(!crew.slots.is_empty(), "{slug} has no slots");
        assert!(!crew.roles.is_empty(), "{slug} has no roles");
        assert!(
            crew.meta
                .description
                .as_deref()
                .is_some_and(|desc| !desc.trim().is_empty()),
            "{slug} must set [meta].description"
        );
        assert!(!crew.meta.tags.is_empty(), "{slug} must set [meta].tags");
        let doc = raw.parse::<toml_edit::DocumentMut>().unwrap();
        let category = doc
            .get("meta")
            .and_then(|item| item.as_table())
            .and_then(|table| table.get("category"))
            .and_then(|item| item.as_str())
            .unwrap_or_else(|| panic!("{slug} must set [meta].category"));
        assert!(
            matches!(category, "dev" | "social" | "video" | "fun"),
            "{slug} has unexpected [meta].category `{category}`"
        );
        if slug == "pair-coding" {
            for role in &crew.roles {
                assert!(
                    role.model.is_none() && role.effort.is_none(),
                    "{slug} starter role @{} must omit model and effort",
                    role.handle
                );
            }
            for slot in &crew.slots {
                assert!(
                    slot.model.is_none() && slot.effort.is_none(),
                    "{slug} starter slot @{} must omit model and effort",
                    slot.handle
                );
            }
        }
        let round_trip = format_template(&Template::Crew(crew.clone()))
            .unwrap_or_else(|err| panic!("failed to reformat {slug}: {err}"));
        assert_eq!(
            parse_template(&round_trip).unwrap(),
            Template::Crew(crew),
            "{slug} did not round-trip through format_template"
        );
        // Only goal.md and <slug>.crew.toml remain in each example folder.
        let mut files: Vec<String> = std::fs::read_dir(entry.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        let mut expected = vec![format!("{slug}.crew.toml"), "goal.md".to_owned()];
        expected.sort();
        assert_eq!(files, expected, "unexpected files in examples/{slug}");
        found.push(slug);
    }
    found.sort();
    assert_eq!(
        found,
        vec![
            "brainstorm-crew",
            "claude-duo",
            "neon-trio",
            "pair-coding",
            "release-video",
            "review-panel",
            "tic-tac-toe",
            "x-promotion",
        ]
    );
}
