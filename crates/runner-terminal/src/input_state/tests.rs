use super::*;
use crate::replay::{feed, new_term, row_to_string, Terminal};

#[test]
fn matcher_requires_a_prompt_prefix_and_prefers_the_empty_form() {
    let before = vec!["assistant ".into(), "❯ Try a task".into(), "$ ".into()];
    let after = vec!["assistant h".into(), "❯ h".into(), "$ h".into()];
    assert_eq!(
        locate_echo(&before, &after, "h", false, 40),
        Some(EchoMatch {
            row: 2,
            before_row: 2,
            prefix: "$ ".into(),
            empty_before: true,
            kind: EchoMatchKind::Exact,
        })
    );
}

#[test]
fn matcher_accepts_stable_right_side_composer_decorations() {
    for (before, after) in [
        (
            "> Try a task            <ret> send",
            "> h                     <ret> send",
        ),
        ("| > Ask Codex           |", "| > h                   |"),
    ] {
        assert!(locate_echo(&[before.into()], &[after.into()], "h", false, 40).is_some());
    }

    let before = vec!["  └ ".into(), "> Try a task            <ret> send".into()];
    let after = vec![
        "  └ hooked 12 files".into(),
        "> h                     <ret> send".into(),
    ];
    assert_eq!(locate_echo(&before, &after, "h", false, 40).unwrap().row, 1);
}

#[test]
fn a_transcript_echo_cannot_reuse_an_unchanged_prompt_on_another_row() {
    let before = vec!["old output".into(), "› Ask Codex".into()];
    let after = vec!["› hello".into(), "› Ask Codex".into()];
    assert!(locate_echo(&before, &after, "hello", false, 40).is_none());
}

#[test]
fn coalesced_native_input_holds_across_the_prompt_wrap_boundary() {
    use crate::replay::screen_snapshot;

    for (prefix, prompt_width) in [("› ", 2), ("界› ", 4)] {
        let available = 12 - prompt_width;
        for length in [available, available + 1, 12, 13] {
            let start = Instant::now();
            let mut term = new_term(12, 4);
            feed(&mut term, format!("{prefix}Ask").as_bytes());
            let mut tracker = InputTracker::new(start);
            tracker.initial_observation(start);
            let text = &"abcdefghijklm"[..length];
            for (ms, character) in text.chars().enumerate() {
                tracker.observe_input(
                    &InputEvent::Key {
                        kind: InputKind::Content {
                            text: character.to_string(),
                        },
                    },
                    start + Duration::from_millis(ms as u64),
                    &term.screen(),
                );
            }
            let split = text.len().min(available);
            feed(
                &mut term,
                format!("\x1b[H\x1b[J{prefix}{}", &text[..split]).as_bytes(),
            );
            if split < text.len() {
                feed(
                    &mut term,
                    format!("\r\n{}{}", " ".repeat(prompt_width), &text[split..]).as_bytes(),
                );
            }
            tracker.observe_output(start + Duration::from_millis(30), &term.screen());
            tracker.observe_output(start + Duration::from_secs(3), &term.screen());
            assert_eq!(
                tracker.current_reported_state().state,
                InputState::Drafting,
                "prefix={prefix:?} length={length}\n{}",
                screen_snapshot(&term)
            );
            feed(&mut term, format!("\x1b[H\x1b[J{prefix}Ask").as_bytes());
            assert_eq!(
                tracker
                    .observe_output(start + Duration::from_secs(4), &term.screen())
                    .unwrap()
                    .state,
                InputState::Idle
            );
        }
    }
}

#[test]
fn transcript_repaint_cannot_steal_the_composer_echo() {
    let start = Instant::now();
    let mut term = new_term(40, 5);
    feed(
        &mut term,
        b"\x1b[2;1H\xe2\x9d\xaf Try a task\x1b[4;1H  \xe2\x94\x94 ",
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "h".into() },
        },
        start,
        &term.screen(),
    );
    feed(
        &mut term,
        b"\x1b[2;1H\x1b[K\xe2\x9d\xaf h\x1b[4;1H\x1b[K  \xe2\x94\x94 hooked 12 files",
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    feed(&mut term, b"\x1b[2;1H\x1b[K\xe2\x9d\xaf Try a task");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(2), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn coalesced_composer_echo_and_clear_stays_idle() {
    let start = Instant::now();
    let mut term = new_term(40, 5);
    feed(
        &mut term,
        b"\x1b[2;1H\xe2\x9d\xaf Try a task\x1b[4;1H  \xe2\x94\x94 ",
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "h".into() },
        },
        start,
        &term.screen(),
    );
    let burst = [
        b"\x1b[2;1H\x1b[K\xe2\x9d\xaf h\x1b[4;1H\x1b[K  \xe2\x94\x94 hooked 12 files".as_slice(),
        b"\x1b[2;1H\x1b[K\xe2\x9d\xaf Try a task",
    ]
    .concat();
    feed(&mut term, &burst);
    assert!(tracker
        .observe_output(start + Duration::from_millis(2), &term.screen())
        .is_none());
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
}

#[test]
fn matcher_accepts_collapsed_multiline_paste_but_not_a_transcript_row() {
    assert_eq!(
        match_prompt_prefix(
            "❯ Try a task",
            "❯ [Pasted text #1 +19 lines]",
            "line one",
            true,
            40,
        ),
        Some(("❯ ".into(), EchoMatchKind::Weak))
    );
    assert_eq!(
        match_prompt_prefix("assistant ", "assistant [Pasted text]", "line", true, 40),
        None
    );
}

#[test]
fn typed_then_deleted_returns_to_idle_from_the_grid() {
    let start = Instant::now();
    let mut term = new_term(40, 4);
    feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf Try a task");
    let mut tracker = InputTracker::new(start);
    assert_eq!(tracker.initial_observation(start).state, InputState::Idle);
    assert!(tracker
        .observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "h".into() }
            },
            start,
            &term.screen(),
        )
        .is_none());
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
    let drafting = tracker
        .observe_output(start + Duration::from_millis(5), &term.screen())
        .unwrap();
    assert_eq!(drafting.state, InputState::Drafting);
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf Try a task");
    let idle = tracker
        .observe_output(start + Duration::from_millis(10), &term.screen())
        .unwrap();
    assert_eq!(idle.state, InputState::Idle);
}

#[test]
fn claude_styled_rotated_placeholder_returns_drafting_to_idle() {
    let start = Instant::now();
    let mut term = new_term(50, 4);
    feed(
        &mut term,
        b"\x1b[2;1H\xe2\x9d\xaf \x1b[2mTry refactoring this file\x1b[22m",
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "h".into() },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );

    feed(
        &mut term,
        b"\r\x1b[K\xe2\x9d\xaf \x1b[2mTry writing a test\x1b[22m",
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(2), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn restyled_live_draft_stays_drafting() {
    let start = Instant::now();
    let mut term = new_term(50, 4);
    feed(
        &mut term,
        b"\x1b[2;1H\xe2\x9d\xaf \x1b[2mTry refactoring this file\x1b[22m",
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "hello world".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello world");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );

    feed(
        &mut term,
        b"\r\x1b[K\xe2\x9d\xaf \x1b[2mhello world\x1b[22m",
    );
    assert!(tracker
        .observe_output(start + Duration::from_millis(2), &term.screen())
        .is_none());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
}

#[test]
fn codex_styled_rotated_placeholder_returns_drafting_to_idle() {
    let start = Instant::now();
    let mut term = new_term(50, 4);
    feed(
            &mut term,
            b"\x1b[2;1H\xe2\x94\x82 \xe2\x80\xba \x1b[38;5;8mAsk Codex\x1b[39m             \xe2\x94\x82",
        );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "h".into() },
        },
        start,
        &term.screen(),
    );
    feed(
        &mut term,
        b"\r\x1b[K\xe2\x94\x82 \xe2\x80\xba h                     \xe2\x94\x82",
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );

    feed(
        &mut term,
        b"\r\x1b[K\xe2\x94\x82 \xe2\x80\xba \x1b[38;5;8mReview this repository\x1b[39m\xe2\x94\x82",
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(2), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn clearing_a_draft_after_a_rollback_redraw_releases_the_hold() {
    for kind in [InputKind::Edit, InputKind::Cancel] {
        let start = Instant::now();
        let mut term = new_term(60, 12);
        feed(
            &mut term,
            "\x1b[4;1H› \x1b[90mAsk Codex to do anything\x1b[39m\x1b[4;3H".as_bytes(),
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "fix it".into(),
                },
            },
            start,
            &term.screen(),
        );
        feed(&mut term, "\x1b[4;1H\x1b[K› fix it".as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term.screen())
                .unwrap()
                .state,
            InputState::Drafting
        );

        // Codex redraws the trimmed transcript; a user row now sits at the tracked row.
        feed(
                &mut term,
                "\x1b[H\x1b[2J› first question\r\n\r\n• answer one\r\n› second question\r\n\r\n• answer two\x1b[10;1H› fix it".as_bytes(),
            );
        tracker.observe_output(start + Duration::from_millis(2), &term.screen());
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

        tracker.observe_input(
            &InputEvent::Key { kind: kind.clone() },
            start,
            &term.screen(),
        );
        if kind == InputKind::Edit {
            feed(&mut term, "\x1b[10;1H\x1b[K› fix".as_bytes());
            tracker.observe_output(start + Duration::from_millis(3), &term.screen());
            assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
            tracker.observe_input(
                &InputEvent::Key { kind: kind.clone() },
                start,
                &term.screen(),
            );
        }
        feed(
            &mut term,
            "\x1b[10;1H\x1b[K› \x1b[90mAsk Codex to do anything\x1b[39m\x1b[10;3H".as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(4), &term.screen());
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Idle,
            "{kind:?}"
        );
    }
}

#[test]
fn explicit_reset_forces_a_draft_idle_once() {
    let start = Instant::now();
    let mut term = new_term(40, 4);
    feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf Try a task");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "h".into() },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );

    let guard = tracker.reset_guard();
    let reset = tracker
        .reset_if_unchanged(guard, start + Duration::from_millis(2))
        .unwrap();
    assert_eq!(reset.state, InputState::Idle);
    assert!(!reset.composing);
    assert!(tracker
        .reset_if_unchanged(guard, start + Duration::from_millis(3))
        .is_none());
}

#[test]
fn explicit_reset_does_not_clear_input_observed_after_the_guard() {
    let start = Instant::now();
    let mut term = new_term(40, 4);
    feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf Try a task");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "h".into() },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    let guard = tracker.reset_guard();
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "i".into() },
        },
        start + Duration::from_millis(2),
        &term.screen(),
    );

    assert!(tracker
        .reset_if_unchanged(guard, start + Duration::from_millis(3))
        .is_none());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
}

#[test]
fn antigravity_submitted_history_does_not_hold_the_empty_composer() {
    use crate::fixtures::{decode_chunk, Fixture, FixtureEvent};

    for injected in [false, true] {
        let start = Instant::now();
        let mut term = new_term(110, 36);
        let border = "─".repeat(110);
        feed(
            &mut term,
            format!("\x1b[?1049h\x1b[8;1H{border}\r\n> \r\n{border}").as_bytes(),
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        let text = "Reply with exactly OK.";
        if injected {
            tracker.track_delivery_input(text, start, &term.screen());
        } else {
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Content { text: text.into() },
                },
                start,
                &term.screen(),
            );
        }
        feed(&mut term, format!("\x1b[9;1H> {text}").as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term.screen())
                .unwrap()
                .state,
            InputState::Drafting
        );
        if injected {
            tracker.track_delivery_submit(start + Duration::from_millis(2), &term.screen());
        } else {
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(2),
                &term.screen(),
            );
        }

        let fixture = Fixture::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/agy-first-turn.ndjson"),
        )
        .unwrap();
        for event in fixture.events {
            if let FixtureEvent::Data { data, .. } = event {
                feed(&mut term, &decode_chunk(&data).unwrap());
            }
        }
        assert_eq!(row_to_string(&term, 8), "> Reply with exactly OK.");
        assert_eq!(row_to_string(&term, 13), ">");
        tracker.observe_output(start + Duration::from_millis(3), &term.screen());
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Idle,
            "injected={injected}"
        );
    }
}

#[test]
fn framed_composer_relocation_keeps_a_live_draft_held() {
    let start = Instant::now();
    let mut term = new_term(40, 12);
    let border = "─".repeat(40);
    feed(
        &mut term,
        format!("\x1b[2;1H{border}\r\n> \r\n{border}").as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\x1b[3;1H> draft");
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    feed(
        &mut term,
        format!("\x1b[4;1H\x1b[K\x1b[7;1H{border}\r\n> draft\r\n{border}\x1b[8;8H").as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(2), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    feed(&mut term, b"\x1b[8;1H> \x1b[K");
    tracker.observe_output(start + Duration::from_millis(3), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
}

#[test]
fn framed_composer_keeps_multiline_drafts_held_and_releases_after_resize() {
    let start = Instant::now();
    let mut term = new_term(40, 12);
    let border = "─".repeat(40);
    feed(
        &mut term,
        format!("\x1b[2;1H{border}\r\n> \r\n{border}").as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Paste {
            text: "first\nsecond".into(),
        },
        start,
        &term.screen(),
    );
    feed(
        &mut term,
        format!("\x1b[3;1H> first\r\nsecond\r\n{border}").as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

    term.resize(30, 12);
    let border = "─".repeat(30);
    feed(
        &mut term,
        format!("\x1b[H\x1b[J> first\r\nsecond\x1b[7;1H{border}\r\n> first\r\nsecond\r\n{border}")
            .as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(2), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    feed(
        &mut term,
        format!("\x1b[8;1H> \x1b[K\r\n{border}\r\n\x1b[K\x1b[8;3H").as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(3), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
}

#[test]
fn an_unrelated_frame_cannot_release_a_multiline_or_hidden_draft() {
    for hidden in [false, true] {
        let start = Instant::now();
        let mut term = new_term(40, 12);
        let border = "─".repeat(40);
        feed(
            &mut term,
            format!("\x1b[2;1H{border}\r\n> Ask\r\n{border}").as_bytes(),
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Paste {
                text: "first\nsecond".into(),
            },
            start,
            &term.screen(),
        );
        feed(
            &mut term,
            format!("\x1b[3;1H> first\r\nsecond\r\n{border}").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(1), &term.screen());
        if hidden {
            feed(&mut term, b"\x1b[3;1Hmenu\x1b[K\r\nchoice\x1b[K");
        }
        feed(
            &mut term,
            format!("\x1b[8;1H{border}\r\n> Ask\r\n{border}\x1b[4;7H").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(2), &term.screen());
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Drafting,
            "hidden={hidden}"
        );
    }
}

#[test]
fn ambiguous_framed_prompts_cannot_release_a_live_draft() {
    let start = Instant::now();
    let mut term = new_term(40, 12);
    let border = "─".repeat(40);
    feed(
        &mut term,
        format!("\x1b[2;1H{border}\r\n> \r\n{border}").as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\x1b[3;1H> draft");
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    feed(
        &mut term,
        format!("\x1b[7;1H{border}\r\n> \r\n{border}").as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(2), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
}

#[test]
fn claude_framed_composer_releases_after_output_moves_it() {
    let start = Instant::now();
    let mut term = new_term(40, 12);
    let border = "─".repeat(40);
    feed(
        &mut term,
        format!("\x1b[2;1H{border}\r\n❯ \r\n{border}").as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, "\x1b[3;1H❯ draft".as_bytes());
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    feed(
            &mut term,
            format!(
                "\x1b[2;1H● working\x1b[K\r\n\x1b[K\r\n\x1b[K\x1b[5;1H{border}\r\n❯ draft\r\n{border}\x1b[6;8H"
            )
            .as_bytes(),
        );
    tracker.observe_output(start + Duration::from_millis(2), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    feed(&mut term, "\x1b[6;1H❯ \x1b[K\x1b[6;3H".as_bytes());
    tracker.observe_output(start + Duration::from_millis(3), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
}

#[test]
fn a_reshaped_frame_still_releases_the_cleared_composer_under_the_cursor() {
    let start = Instant::now();
    let mut term = new_term(40, 12);
    let border = "─".repeat(40);
    feed(
        &mut term,
        format!("\x1b[2;1H{border}\r\n> \r\n{border}").as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\x1b[3;1H> draft");
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    feed(
            &mut term,
            format!(
                "\x1b[2;1H\x1b[K\r\n\x1b[K\r\n\x1b[K\x1b[7;1H── plan ──\x1b[K\r\n> \x1b[K\r\n{border}\x1b[8;3H"
            )
            .as_bytes(),
        );
    tracker.observe_output(start + Duration::from_millis(2), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
}

#[test]
fn submitted_accepts_a_rotated_empty_form() {
    let start = Instant::now();
    let mut term = new_term(40, 4);
    feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf First placeholder");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "hello".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello");
    tracker.observe_output(start + Duration::from_millis(5), &term.screen());
    let submitted = tracker
        .observe_input(
            &InputEvent::Key {
                kind: InputKind::Submit,
            },
            start + Duration::from_millis(6),
            &term.screen(),
        )
        .unwrap();
    assert_eq!(submitted.state, InputState::Submitted);
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf Different placeholder");
    let idle = tracker
        .observe_output(start + Duration::from_millis(9), &term.screen())
        .unwrap();
    assert_eq!(idle.state, InputState::Idle);
}

#[test]
fn delivery_submit_before_echo_accepts_a_rotated_empty_form_without_local_input() {
    let start = Instant::now();
    let mut term = new_term(80, 4);
    feed(&mut term, "\x1b[2;1H❯ First placeholder".as_bytes());
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.track_delivery_input("[inbox] task", start, &term.screen());
    tracker.track_delivery_submit(start, &term.screen());
    feed(&mut term, "\r\x1b[K❯ [inbox] task".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(5), &term.screen())
            .unwrap()
            .state,
        InputState::Unsent
    );
    // An ignored Enter with no later output leaves the notice where the guard can resubmit it.
    assert!(tracker.unsent_notice(&term.screen()));
    feed(&mut term, "\r\x1b[K❯ Different placeholder".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(9), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
    assert_eq!(tracker.reset_guard(), 0);
}

#[test]
fn local_content_after_a_generated_submit_remains_a_draft() {
    let start = Instant::now();
    let mut term = new_term(80, 4);
    feed(&mut term, "\x1b[2;1H❯ First placeholder".as_bytes());
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.track_delivery_input("[inbox] task", start, &term.screen());
    tracker.track_delivery_submit(start, &term.screen());
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: " draft".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, "\r\x1b[K❯ [inbox] task draft".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(5), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
}

#[test]
fn an_ignored_delivery_enter_leaves_the_routers_notice_until_a_person_types() {
    let text = "[inbox] 1 new messages — run `runner msg read` to view.";
    for person in [false, true] {
        let start = Instant::now();
        let mut term = new_term(80, 4);
        feed(&mut term, "\x1b[2;1H› Ask Codex".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        if person {
            tracker.observe_input(
                &InputEvent::Paste { text: text.into() },
                start,
                &term.screen(),
            );
        } else {
            tracker.track_delivery_input(text, start, &term.screen());
        }
        feed(&mut term, format!("\r\x1b[K› {text}").as_bytes());
        tracker.observe_output(start + Duration::from_millis(5), &term.screen());
        if person {
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(300),
                &term.screen(),
            );
        } else {
            tracker.track_delivery_submit(start + Duration::from_millis(300), &term.screen());
        }
        feed(&mut term, format!("\r\x1b[K› {text}").as_bytes());
        tracker.observe_output(start + Duration::from_millis(400), &term.screen());
        tracker.observe_output(start + Duration::from_secs(26 * 60), &term.screen());
        assert_eq!(
            tracker.current_reported_state().state,
            if person {
                InputState::Drafting
            } else {
                InputState::Unsent
            },
            "person={person}"
        );
        assert_eq!(
            tracker.unsent_notice(&term.screen()),
            !person,
            "person={person}"
        );
        if person {
            continue;
        }
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "!".into() },
            },
            start + Duration::from_secs(26 * 60 + 1),
            &term.screen(),
        );
        feed(&mut term, format!("\r\x1b[K› {text}!").as_bytes());
        let observation = tracker
            .observe_output(start + Duration::from_secs(26 * 60 + 2), &term.screen())
            .unwrap();
        assert_eq!(observation.state, InputState::Drafting);
        assert!(!tracker.unsent_notice(&term.screen()));
    }
}

const NOTICE: &str = "[inbox] 1 new messages — run `runner msg read` to view.";

/// The router types its notice into the composer at row 4 and presses Enter.
fn submitted_notice(cols: u16, echo: &str) -> (Instant, Terminal, InputTracker) {
    let start = Instant::now();
    let mut term = new_term(cols, 8);
    feed(&mut term, b"\x1b[5;1H> Try a task");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.track_delivery_input(NOTICE, start, &term.screen());
    feed(&mut term, echo.as_bytes());
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    tracker.track_delivery_submit(start + Duration::from_millis(80), &term.screen());
    (start, term, tracker)
}

#[test]
fn a_confirmed_notice_is_not_unsent_where_a_history_row_repeats_it() {
    let echo = format!("\x1b[5;1H\x1b[K> {NOTICE}");
    let (start, mut term, mut tracker) = submitted_notice(80, &echo);
    // The prompt stays in history at the composer row; the cursor is parked below the output.
    let history = format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[8;1H\x1b[K");
    feed(&mut term, history.as_bytes());
    tracker.observe_output(start + Duration::from_millis(100), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Unsent);

    let revision = tracker.reset_guard();
    assert_eq!(
        tracker
            .confirm_delivery(revision, start + Duration::from_millis(1600))
            .unwrap()
            .state,
        InputState::Idle
    );
    feed(&mut term, history.as_bytes());
    tracker.observe_output(start + Duration::from_secs(60), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    assert!(!tracker.unsent_notice(&term.screen()));
}

#[test]
fn person_input_after_the_routers_revision_survives_confirmation_and_recheck() {
    for input in [
        InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        },
        InputEvent::Paste {
            text: "draft".into(),
        },
    ] {
        let echo = format!("\x1b[5;1H\x1b[K> {NOTICE}");
        let (start, mut term, mut tracker) = submitted_notice(80, &echo);
        feed(
            &mut term,
            format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[8;1H\x1b[K")
                .as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(100), &term.screen());
        let revision = tracker.reset_guard();
        // A person types before the router's confirmation and before the echo.
        tracker.observe_input(&input, start + Duration::from_millis(200), &term.screen());
        assert!(tracker
            .confirm_delivery(revision, start + Duration::from_millis(1600))
            .is_none());
        feed(&mut term, b"\x1b[7;1H\x1b[K> draft");
        tracker.observe_output(start + Duration::from_millis(1700), &term.screen());
        tracker.recheck_delivery(start + Duration::from_secs(60), &term.screen());
        assert_ne!(
            tracker.current_reported_state().state,
            InputState::Idle,
            "{input:?}"
        );
    }
}

#[test]
fn a_recheck_keeps_only_a_notice_under_the_input_cursor() {
    let echo = format!("\x1b[5;1H\x1b[K> {NOTICE}");
    let wrapped =
        "\x1b[5;1H\x1b[K> [inbox] 1 new messages — run\r\n\x1b[K  `runner msg read` to view.";
    for (case, cols, echo, after, unsent) in [
        ("in the composer", 80, echo.as_str(), echo.clone(), true),
        ("wrapped", 40, wrapped, wrapped.to_owned(), true),
        (
            "history above the live prompt",
            80,
            echo.as_str(),
            format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[7;1H"),
            false,
        ),
        (
            "cursor on output below the notice",
            80,
            echo.as_str(),
            format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[6;5H"),
            false,
        ),
        (
            "cursor parked below the live prompt",
            80,
            echo.as_str(),
            format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[8;1H"),
            false,
        ),
        (
            "composer lost",
            80,
            echo.as_str(),
            "\x1b[2J\x1b[5;1H⏺ Replied\x1b[7;1H> \x1b[8;1H".to_owned(),
            false,
        ),
    ] {
        let (start, mut term, mut tracker) = submitted_notice(cols, echo);
        feed(&mut term, after.as_bytes());
        tracker.observe_output(start + Duration::from_millis(100), &term.screen());
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Unsent,
            "{case}"
        );
        tracker.recheck_delivery(start + Duration::from_secs(60), &term.screen());
        assert_eq!(
            tracker.current_reported_state().state,
            if unsent {
                InputState::Unsent
            } else {
                InputState::Idle
            },
            "{case}"
        );
        assert_eq!(tracker.unsent_notice(&term.screen()), unsent, "{case}");
    }
}

const LINKED: &str = "[runner] Mission m1 is now linked to this chat.";

/// #1225: the router's line is submitted, and Claude Code redraws it as a transcript row at
/// the composer's row while the turn runs, with the input cursor parked below the frame.
fn linked_line_in_transcript() -> (Instant, Terminal, InputTracker) {
    let start = Instant::now();
    let mut term = new_term(60, 10);
    feed(
        &mut term,
        "\x1b[1;1H✻ Welcome\x1b[3;1H❯ \x1b[3;3H".as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.track_delivery_input(LINKED, start, &term.screen());
    feed(&mut term, format!("\x1b[3;1H\x1b[K❯ {LINKED}").as_bytes());
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    tracker.track_delivery_submit(start + Duration::from_millis(80), &term.screen());
    feed(
        &mut term,
        format!("\x1b[3;1H\x1b[K❯ {LINKED}\x1b[5;1H✻ Contemplating…\x1b[7;1H❯ \x1b[10;1H")
            .as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(89), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Unsent);
    (start, term, tracker)
}

/// The turn ends and the transcript is redrawn: the tracked row now holds an earlier prompt,
/// and the input cursor rests at the empty composer.
const TURN_END: &str = "\x1b[2J\x1b[1;1H⏺ Started.\x1b[3;1H❯ start the mission\x1b[4;1H⏺ Mission started.\x1b[5;1H❯ [runner] Mission m1 is now linked to this chat.\x1b[6;1H⏺ Linked.\x1b[8;1H❯ \x1b[8;3H";

#[test]
fn an_ended_composition_does_not_hold_a_submitted_line_as_unsent() {
    let (start, mut term, mut tracker) = linked_line_in_transcript();
    let revision = tracker.reset_guard();
    tracker.observe_input(
        &InputEvent::Composing { composing: false },
        start + Duration::from_millis(300),
        &term.screen(),
    );
    assert_eq!(tracker.reset_guard(), revision, "no composition to end");
    assert_eq!(
        tracker
            .confirm_delivery(revision, start + Duration::from_millis(1580))
            .map(|observation| observation.state),
        Some(InputState::Idle),
        "the router saw the turn start"
    );
    feed(&mut term, TURN_END.as_bytes());
    tracker.observe_output(start + Duration::from_secs(14), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);

    // A composition that did start still counts as input.
    tracker.observe_input(
        &InputEvent::Composing { composing: true },
        start,
        &term.screen(),
    );
    let composing = tracker.reset_guard();
    assert_ne!(composing, revision);
    tracker.observe_input(
        &InputEvent::Composing { composing: false },
        start,
        &term.screen(),
    );
    assert_ne!(tracker.reset_guard(), composing);
}

#[test]
fn a_key_during_the_turn_cannot_make_the_transcript_a_draft() {
    let (start, mut term, mut tracker) = linked_line_in_transcript();
    let revision = tracker.reset_guard();
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Navigate,
        },
        start + Duration::from_millis(300),
        &term.screen(),
    );
    assert!(tracker
        .confirm_delivery(revision, start + Duration::from_millis(1580))
        .is_none());
    feed(&mut term, TURN_END.as_bytes());
    tracker.observe_output(start + Duration::from_secs(14), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    assert!(!tracker.unsent_notice(&term.screen()));
}

#[test]
fn a_recheck_drops_a_draft_the_empty_editor_does_not_show() {
    let (start, mut term, mut tracker) = linked_line_in_transcript();
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Navigate,
        },
        start + Duration::from_millis(300),
        &term.screen(),
    );
    // The turn-end frame arrives before Claude Code returns the cursor to its composer.
    feed(&mut term, TURN_END.as_bytes());
    feed(&mut term, b"\x1b[10;1H");
    tracker.observe_output(start + Duration::from_secs(14), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    tracker.recheck_delivery(start + Duration::from_secs(15), &term.screen());
    assert_eq!(
        tracker.current_reported_state().state,
        InputState::Drafting,
        "no editor under the cursor yet"
    );
    feed(&mut term, b"\x1b[8;3H");
    tracker.recheck_delivery(start + Duration::from_secs(16), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
}

fn typed_before_echo(pasted: bool) -> InputEvent {
    if pasted {
        InputEvent::Paste {
            text: "draft".into(),
        }
    } else {
        InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        }
    }
}

#[test]
fn a_recheck_keeps_text_typed_before_its_echo() {
    for pasted in [false, true] {
        let (start, mut term, mut tracker) = linked_line_in_transcript();
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Navigate,
            },
            start + Duration::from_millis(300),
            &term.screen(),
        );
        feed(&mut term, TURN_END.as_bytes());
        feed(&mut term, b"\x1b[10;1H");
        tracker.observe_output(start + Duration::from_secs(14), &term.screen());
        feed(&mut term, b"\x1b[8;3H");
        let typed = start + Duration::from_secs(15);
        tracker.observe_input(&typed_before_echo(pasted), typed, &term.screen());
        tracker.recheck_delivery(typed + Duration::from_millis(10), &term.screen());
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Drafting,
            "pasted={pasted}: the echo is still on its way"
        );
        feed(&mut term, "\x1b[8;1H\x1b[K❯ draft".as_bytes());
        tracker.observe_output(typed + Duration::from_millis(30), &term.screen());
        tracker.recheck_delivery(typed + Duration::from_secs(5), &term.screen());
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Drafting,
            "pasted={pasted}"
        );
    }
}

#[test]
fn text_typed_before_its_echo_is_not_lost_to_a_frame_with_an_empty_editor() {
    // The frame either keeps the transcript row at the tracked row or ends the turn.
    for (changed_row, frame, echo) in [
        (false, "\x1b[7;3H", "\x1b[7;1H\x1b[K❯ draft"),
        (true, TURN_END, "\x1b[8;1H\x1b[K❯ draft"),
    ] {
        for pasted in [false, true] {
            let (start, mut term, mut tracker) = linked_line_in_transcript();
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Navigate,
                },
                start + Duration::from_millis(300),
                &term.screen(),
            );
            let typed = start + Duration::from_secs(14);
            tracker.observe_input(&typed_before_echo(pasted), typed, &term.screen());
            feed(&mut term, frame.as_bytes());
            tracker.observe_output(typed + Duration::from_millis(10), &term.screen());
            feed(&mut term, echo.as_bytes());
            tracker.observe_output(typed + Duration::from_millis(30), &term.screen());
            tracker.recheck_delivery(typed + Duration::from_secs(5), &term.screen());
            assert_ne!(
                tracker.current_reported_state().state,
                InputState::Idle,
                "changed_row={changed_row} pasted={pasted}"
            );
        }
    }
}

#[test]
fn the_tracker_remembers_holding_a_line_it_let_go_of_until_the_next_one() {
    let start = Instant::now();
    let mut term = new_term(60, 10);
    feed(&mut term, "\x1b[3;1H❯ \x1b[3;3H".as_bytes());
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.track_delivery_input(LINKED, start, &term.screen());
    feed(&mut term, format!("\x1b[3;1H\x1b[K❯ {LINKED}").as_bytes());
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    assert!(!tracker.delivery_held(), "not submitted yet");
    tracker.track_delivery_submit(start + Duration::from_millis(80), &term.screen());
    // The first frame after Enter already shows the empty composer below the transcript row.
    feed(
        &mut term,
        format!("\x1b[3;1H\x1b[K❯ {LINKED}\x1b[5;1H❯ \x1b[5;3H").as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(89), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    assert!(tracker.delivery_held());

    // A line whose echo was never located is not one the tracker held.
    tracker.track_delivery_input(
        "[inbox] task",
        start + Duration::from_secs(5),
        &term.screen(),
    );
    assert!(!tracker.delivery_held());
    tracker.track_delivery_submit(start + Duration::from_secs(5), &term.screen());
    tracker.observe_output(start + Duration::from_secs(6), &term.screen());
    assert!(!tracker.delivery_held());
}

#[test]
fn a_recheck_keeps_a_real_draft_before_and_after_a_turn_end() {
    let start = Instant::now();
    let mut term = new_term(60, 10);
    feed(&mut term, "\x1b[1;1H⏺ Done.\x1b[3;1H❯ \x1b[3;3H".as_bytes());
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "half a thought".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, "\x1b[3;1H\x1b[K❯ half a thought".as_bytes());
    tracker.observe_output(start + Duration::from_millis(5), &term.screen());
    tracker.recheck_delivery(start + Duration::from_secs(5), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    // A turn end redraws the transcript above the draft and returns the cursor to it.
    feed(
        &mut term,
        "\x1b[1;1H⏺ Done again.\x1b[K\x1b[3;1H\x1b[K❯ half a thought\x1b[3;17H".as_bytes(),
    );
    tracker.observe_output(start + Duration::from_secs(10), &term.screen());
    tracker.recheck_delivery(start + Duration::from_secs(11), &term.screen());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
}

#[test]
fn delivery_typed_over_person_text_keeps_it_a_draft() {
    // The trailing space defeats the echo match, so that person text is still probing.
    for typed in ["draft", "draft "] {
        let start = Instant::now();
        let mut term = new_term(80, 4);
        feed(&mut term, "\x1b[2;1H❯ Try a task".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: typed.into() },
            },
            start,
            &term.screen(),
        );
        feed(&mut term, format!("\r\x1b[K❯ {typed}").as_bytes());
        tracker.observe_output(start + Duration::from_millis(5), &term.screen());
        tracker.track_delivery_input(
            "[inbox] task",
            start + Duration::from_millis(6),
            &term.screen(),
        );
        feed(
            &mut term,
            format!("\r\x1b[K❯ {typed}[inbox] task").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(9), &term.screen());
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Drafting,
            "{typed:?}"
        );
    }
}

#[test]
fn hidden_composer_stays_drafting_until_the_grid_restores_it() {
    let start = Instant::now();
    let mut term = new_term(30, 3);
    feed(&mut term, b"\x1b[2;1H$ ");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "x".into() },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K$ x");
    let drafting = tracker
        .observe_output(start + Duration::from_millis(1), &term.screen())
        .unwrap();
    assert_eq!(drafting.state, InputState::Drafting);
    feed(&mut term, b"\r\x1b[Kmenu");
    let hidden = tracker
        .observe_output(start + Duration::from_millis(2), &term.screen())
        .unwrap();
    assert_eq!(hidden.state, InputState::Drafting);
    assert!(!hidden.composer_visible);
    feed(&mut term, b"\x1b[1;1H\x1b[K$ x");
    let visible = tracker
        .observe_output(start + Duration::from_millis(3), &term.screen())
        .unwrap();
    assert!(visible.composer_visible);
    feed(&mut term, b"\r\x1b[K$ ");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(4), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn resize_below_the_tracked_row_relocates_and_observes_the_clear() {
    let start = Instant::now();
    let mut term = new_term(30, 30);
    feed(&mut term, b"\x1b[25;1H$ ");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "x".into() },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K$ x");
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());

    term.resize(30, 10);
    feed(&mut term, b"\x1b[2J\x1b[8;1H$ x");
    assert!(tracker
        .observe_output(start + Duration::from_millis(2), &term.screen())
        .is_none());
    feed(&mut term, b"\r\x1b[K$ ");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(3), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn enter_over_a_hidden_composer_does_not_replace_the_submitted_form() {
    let start = Instant::now();
    let mut term = new_term(30, 3);
    feed(&mut term, b"\x1b[2;1H$ ");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "x".into() },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K$ x");
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    feed(&mut term, b"\r\x1b[Kmenu");
    tracker.observe_output(start + Duration::from_millis(2), &term.screen());
    assert_eq!(
        tracker
            .observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(3),
                &term.screen(),
            )
            .unwrap()
            .state,
        InputState::Submitted
    );
    feed(&mut term, b"\r\x1b[K$ x");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(4), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
}

#[test]
fn content_before_submit_redraw_rearms_drafting_without_an_idle_transition() {
    let start = Instant::now();
    let mut term = new_term(40, 4);
    feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf Try a task");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "hello".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello");
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    assert_eq!(
        tracker
            .observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(2),
                &term.screen(),
            )
            .unwrap()
            .state,
        InputState::Submitted
    );
    assert!(tracker
        .observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "n".into() },
            },
            start + Duration::from_millis(3),
            &term.screen(),
        )
        .is_none());
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf n");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(4), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
}

#[test]
fn submit_redraw_before_the_next_echo_keeps_the_reprobe_armed() {
    let start = Instant::now();
    let mut term = new_term(40, 4);
    feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf Try a task");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "hello".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello");
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Submit,
        },
        start + Duration::from_millis(2),
        &term.screen(),
    );
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "w".into() },
        },
        start + Duration::from_millis(3),
        &term.screen(),
    );

    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf Try a task");
    assert!(tracker
        .observe_output(start + Duration::from_millis(4), &term.screen())
        .is_none());
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf w");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(5), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
}

#[test]
fn submit_while_probing_holds_a_nonempty_first_echo() {
    let start = Instant::now();
    let mut term = new_term(30, 3);
    feed(&mut term, b"\x1b[2;1H$ ");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "x".into() },
        },
        start,
        &term.screen(),
    );
    assert!(tracker
        .observe_input(
            &InputEvent::Key {
                kind: InputKind::Submit,
            },
            start + Duration::from_millis(1),
            &term.screen(),
        )
        .is_none());
    feed(&mut term, b"\r\x1b[K$ x");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(2), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
}

#[test]
fn relocation_tracks_the_nearest_prompt_row_then_observes_its_clear() {
    let start = Instant::now();
    let mut term = new_term(30, 4);
    feed(&mut term, b"\x1b[2;1H$ ");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "x".into() },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K$ x");
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    feed(&mut term, b"\r\x1b[Kpopup\x1b[3;1H$ x");
    assert!(tracker
        .observe_output(start + Duration::from_millis(2), &term.screen())
        .is_none());
    feed(&mut term, b"\r\x1b[K$ ");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(3), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn composition_reports_drafting_and_masks_enter_submission() {
    let start = Instant::now();
    let term = new_term(30, 3);
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    let composing = tracker
        .observe_input(
            &InputEvent::Composing { composing: true },
            start + Duration::from_millis(1),
            &term.screen(),
        )
        .unwrap();
    assert_eq!(composing.state, InputState::Drafting);
    assert!(composing.composing);
    assert!(tracker
        .observe_input(
            &InputEvent::Key {
                kind: InputKind::Submit,
            },
            start + Duration::from_millis(2),
            &term.screen(),
        )
        .is_none());
    let idle = tracker
        .observe_input(
            &InputEvent::Composing { composing: false },
            start + Duration::from_millis(3),
            &term.screen(),
        )
        .unwrap();
    assert_eq!(idle.state, InputState::Idle);
}

#[test]
fn every_preedit_update_reports_activity_even_when_the_state_is_unchanged() {
    let start = Instant::now();
    let term = new_term(30, 3);
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    for seconds in [600, 601] {
        let observation = tracker
            .observe_input(
                &InputEvent::Composing { composing: true },
                start + Duration::from_secs(seconds),
                &term.screen(),
            )
            .unwrap();
        assert!(observation.composing);
        assert_eq!(observation.state, InputState::Drafting);
    }
}

#[test]
fn person_keys_keep_notice_ownership_until_the_composer_text_changes() {
    for kind in [
        InputKind::Navigate,
        InputKind::Edit,
        InputKind::Cancel,
        InputKind::Submit,
    ] {
        let start = Instant::now();
        let mut term = new_term(80, 4);
        feed(&mut term, "\x1b[2;1H› Ask Codex".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.track_delivery_input("[inbox] task", start, &term.screen());
        feed(&mut term, "\r\x1b[K› [inbox] task".as_bytes());
        tracker.observe_output(start, &term.screen());
        tracker.track_delivery_submit(start, &term.screen());
        tracker.observe_output(start, &term.screen());
        let revision = tracker.reset_guard();
        tracker.observe_input(&InputEvent::Key { kind }, start, &term.screen());
        feed(&mut term, b"\x1b[D");
        tracker.observe_output(start + Duration::from_secs(11 * 60), &term.screen());
        assert_eq!(tracker.current_reported_state().state, InputState::Unsent);
        assert!(tracker.unsent_notice(&term.screen()));
        assert_ne!(tracker.reset_guard(), revision);
        assert!(tracker.reset_if_unchanged(revision, start).is_none());
        feed(&mut term, "\r\x1b[K› recalled prompt".as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_secs(11 * 60 + 1), &term.screen())
                .unwrap()
                .state,
            InputState::Drafting
        );
        assert!(!tracker.unsent_notice(&term.screen()));
    }
}

#[test]
fn claude_dim_prompt_suggestion_clears_after_blank_composer_turn() {
    for suggestion in [
        "\x1b[2mRun the tests\x1b[22m",
        "\x1b[7mR\x1b[27m\x1b[2mun the tests\x1b[22m",
    ] {
        let start = Instant::now();
        let mut term = new_term(60, 6);
        feed(&mut term, "\x1b[4;1H❯\u{a0}\x1b[4;3H".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "draft".into(),
                },
            },
            start,
            &term.screen(),
        );
        feed(&mut term, "\x1b[4;1H\x1b[K❯\u{a0}draft\x1b[4;8H".as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(5), &term.screen())
                .unwrap()
                .state,
            InputState::Drafting
        );
        // Clearing the draft while Claude Code shows a dim prompt suggestion returns to Idle
        // even though the composer started blank and never learned a prior placeholder style.
        feed(
            &mut term,
            format!("\x1b[4;1H\x1b[K❯\u{a0}{suggestion}\x1b[4;3H").as_bytes(),
        );
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(10), &term.screen())
                .unwrap()
                .state,
            InputState::Idle,
            "suggestion={suggestion:?}"
        );
    }
}

#[test]
fn learned_placeholder_style_persists_across_turns() {
    let start = Instant::now();
    let mut term = new_term(60, 6);
    feed(
        &mut term,
        "\x1b[4;1H› \x1b[38;5;8mAsk Codex\x1b[39m\x1b[4;3H".as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "first".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, "\x1b[4;1H\x1b[K› first\x1b[4;8H".as_bytes());
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    // Turn 1 ends with a blank composer (no placeholder).
    feed(&mut term, "\x1b[4;1H\x1b[K› \x1b[4;3H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(2), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );

    // Turn 2 starts on the blank composer, then clears to a styled placeholder.
    let turn2 = start + Duration::from_secs(5);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "second".into(),
            },
        },
        turn2,
        &term.screen(),
    );
    feed(&mut term, "\x1b[4;1H\x1b[K› second\x1b[4;9H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(turn2 + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    feed(
        &mut term,
        "\x1b[4;1H\x1b[K› \x1b[38;5;8mReview this repository\x1b[39m\x1b[4;3H".as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(turn2 + Duration::from_millis(2), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn nbsp_and_ascii_space_prefixes_match_across_transcript_and_live_composer() {
    let start = Instant::now();
    let mut term = new_term(60, 10);
    // Blank Claude Code composer with NBSP; row_to_string trims it to "❯".
    feed(&mut term, "\x1b[3;1H❯\u{a0}\x1b[3;3H".as_bytes());
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "start review".into(),
            },
        },
        start,
        &term.screen(),
    );
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Submit,
        },
        start + Duration::from_millis(5),
        &term.screen(),
    );
    // Transcript row echoes with ASCII space while the cursor is parked away from the composer.
    feed(
        &mut term,
        "\x1b[3;1H\x1b[K❯ start review\x1b[9;1H".as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(10), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    // Turn ends: transcript copy stays on row 3, live composer at row 8 uses NBSP + dim suggestion.
    feed(
        &mut term,
        "\x1b[8;1H\x1b[K❯\u{a0}\x1b[2mCommit these changes\x1b[22m\x1b[8;3H".as_bytes(),
    );
    assert_eq!(
        tracker
            .recheck_delivery(start + Duration::from_secs(10), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn unframed_relocation_and_recheck_clear_hidden_draft_at_empty_composer() {
    // Part A: an unframed composer that moves and clears to a trimmed empty prompt ("❯")
    // relocates and transitions to Idle.
    let start = Instant::now();
    let mut term = new_term(60, 10);
    feed(&mut term, "\x1b[3;1H❯\u{a0}\x1b[3;3H".as_bytes());
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, "\x1b[3;1H\x1b[K❯\u{a0}draft\x1b[3;8H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    feed(
        &mut term,
        "\x1b[3;1H\x1b[K⏺ Done.\x1b[7;1H\x1b[K❯\u{a0}\x1b[7;3H".as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(2), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );

    // Part B: even when a composer was previously hidden (visible=false), recheck_delivery clears
    // it once the cursor rests on an empty prompt, and keeps it held while the cursor is away.
    let mut hidden_term = new_term(60, 10);
    feed(&mut hidden_term, "\x1b[3;1H❯\u{a0}\x1b[3;3H".as_bytes());
    let mut hidden = InputTracker::new(start);
    hidden.initial_observation(start);
    hidden.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        },
        start,
        &hidden_term.screen(),
    );
    feed(
        &mut hidden_term,
        "\x1b[3;1H\x1b[K❯\u{a0}draft\x1b[3;8H".as_bytes(),
    );
    hidden.observe_output(start + Duration::from_millis(1), &hidden_term.screen());
    feed(&mut hidden_term, "\x1b[3;1H\x1b[Kmenu\x1b[3;5H".as_bytes());
    let hidden_obs = hidden
        .observe_output(start + Duration::from_millis(2), &hidden_term.screen())
        .unwrap();
    assert_eq!(hidden_obs.state, InputState::Drafting);
    assert!(!hidden_obs.composer_visible);
    // Recheck while still on the menu keeps Drafting.
    assert!(hidden
        .recheck_delivery(start + Duration::from_secs(5), &hidden_term.screen())
        .is_none());
    assert_eq!(hidden.current_reported_state().state, InputState::Drafting);
    // Once the empty composer returns under the cursor, recheck_delivery clears it even without
    // an intervening observe_output restoring composer.visible.
    feed(
        &mut hidden_term,
        "\x1b[7;1H\x1b[K❯\u{a0}\x1b[7;3H".as_bytes(),
    );
    assert_eq!(
        hidden
            .recheck_delivery(start + Duration::from_secs(6), &hidden_term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn inverse_cursor_on_placeholder_learns_underlying_style_and_keeps_edited_drafts_held() {
    let start = Instant::now();
    let mut term = new_term(60, 6);
    // Initial placeholder has a fake inverse cursor on cell 0 and grey style on cell 1+.
    feed(
        &mut term,
        "\x1b[4;1H› \x1b[7mA\x1b[27m\x1b[38;5;8msk Codex\x1b[39m\x1b[4;3H".as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "abc".into() },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, "\x1b[4;1H\x1b[K› abc\x1b[4;6H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );

    // Editing a real draft with a fake inverse cursor on cell 0 must stay Drafting, both for
    // multi-char and single-char drafts.
    feed(
        &mut term,
        "\x1b[4;1H\x1b[K› \x1b[7mb\x1b[27mc\x1b[4;3H".as_bytes(),
    );
    assert!(tracker
        .observe_output(start + Duration::from_millis(2), &term.screen())
        .is_none());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

    feed(
        &mut term,
        "\x1b[4;1H\x1b[K› \x1b[7mb\x1b[27m\x1b[4;3H".as_bytes(),
    );
    assert!(tracker
        .observe_output(start + Duration::from_millis(3), &term.screen())
        .is_none());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

    // A rotated placeholder without a fake inverse cursor still matches the learned style.
    feed(
        &mut term,
        "\x1b[4;1H\x1b[K› \x1b[38;5;8mReview this repository\x1b[39m\x1b[4;3H".as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(4), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn dim_draft_theme_and_inline_dim_ghost_completion_stay_drafting() {
    // A theme where typed text itself is dim does not treat edited dim text as a placeholder.
    let start = Instant::now();
    let mut term = new_term(60, 6);
    feed(&mut term, "\x1b[4;1H❯\u{a0}\x1b[4;3H".as_bytes());
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(
        &mut term,
        "\x1b[4;1H\x1b[K❯\u{a0}\x1b[2mdraft\x1b[22m\x1b[4;8H".as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    feed(
        &mut term,
        "\x1b[4;1H\x1b[K❯\u{a0}\x1b[2mdraf\x1b[22m\x1b[4;7H".as_bytes(),
    );
    assert!(tracker
        .observe_output(start + Duration::from_millis(2), &term.screen())
        .is_none());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

    // A normal draft cell followed by a dim inline ghost completion stays Drafting.
    let mut ghost_term = new_term(60, 6);
    feed(&mut ghost_term, "\x1b[4;1H❯\u{a0}\x1b[4;3H".as_bytes());
    let mut ghost = InputTracker::new(start);
    ghost.initial_observation(start);
    ghost.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "hi".into() },
        },
        start,
        &ghost_term.screen(),
    );
    feed(
        &mut ghost_term,
        "\x1b[4;1H\x1b[K❯\u{a0}hi\x1b[4;5H".as_bytes(),
    );
    assert_eq!(
        ghost
            .observe_output(start + Duration::from_millis(1), &ghost_term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    feed(
        &mut ghost_term,
        "\x1b[4;1H\x1b[K❯\u{a0}h\x1b[2mello world\x1b[22m\x1b[4;4H".as_bytes(),
    );
    assert!(ghost
        .observe_output(start + Duration::from_millis(2), &ghost_term.screen())
        .is_none());
    assert_eq!(ghost.current_reported_state().state, InputState::Drafting);
}

#[test]
fn nonempty_prefix_requires_content_and_normalizes_trailing_whitespace() {
    assert!(row_matches_nonempty_prefix("❯\u{a0}hello", "❯ "));
    assert!(row_matches_nonempty_prefix("❯ hello", "❯\u{a0}"));
    assert!(!row_matches_nonempty_prefix("❯", "❯ "));
    assert!(!row_matches_nonempty_prefix("❯ ", "❯ "));
    assert!(!row_matches_nonempty_prefix("❯\u{a0}", "❯ "));
    assert!(!row_matches_nonempty_prefix("❯   ", "❯\u{a0}"));
    assert!(row_matches_prefix("❯", "❯\u{a0}"));
    assert!(row_matches_prefix("❯\u{a0}", "❯ "));
}

#[test]
fn unframed_dim_status_or_transcript_at_former_row_keeps_relocated_draft_held() {
    for (case, former_row, cursor_after) in [
        (
            "dim non-prompt status, cursor on draft",
            "\x1b[2mstatus moved\x1b[22m",
            "\x1b[6;8H",
        ),
        (
            "dim non-prompt status, cursor parked away",
            "\x1b[2mstatus moved\x1b[22m",
            "\x1b[9;1H",
        ),
        (
            "dim transcript prompt, cursor on draft",
            "❯ \x1b[2mold transcript prompt\x1b[22m",
            "\x1b[6;8H",
        ),
        (
            "dim transcript prompt, cursor on continuation",
            "❯ \x1b[2mold transcript prompt\x1b[22m",
            "\x1b[7;5H",
        ),
    ] {
        let start = Instant::now();
        let mut term = new_term(60, 10);
        feed(&mut term, "\x1b[3;1H❯ \x1b[3;3H".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "draft".into(),
                },
            },
            start,
            &term.screen(),
        );
        feed(&mut term, "\x1b[3;1H\x1b[K❯ draft\x1b[3;8H".as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term.screen())
                .unwrap()
                .state,
            InputState::Drafting,
            "{case}"
        );

        feed(
            &mut term,
            format!("\x1b[3;1H\x1b[K{former_row}\x1b[6;1H\x1b[K❯ draft{cursor_after}").as_bytes(),
        );
        assert!(
            tracker
                .observe_output(start + Duration::from_millis(2), &term.screen())
                .is_none(),
            "{case}: relocated draft must not drop to Idle"
        );
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Drafting,
            "{case}"
        );

        // Clearing the relocated composer under the cursor releases to Idle.
        feed(
            &mut term,
            "\x1b[6;1H\x1b[K❯ \x1b[2mRun tests\x1b[22m\x1b[6;3H".as_bytes(),
        );
        if tracker
            .observe_output(start + Duration::from_millis(3), &term.screen())
            .is_none()
        {
            tracker.recheck_delivery(start + Duration::from_secs(5), &term.screen());
        }
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Idle,
            "{case}"
        );
    }
}

#[test]
fn current_draft_style_overrides_cached_placeholder_style_across_turns() {
    let start = Instant::now();
    let mut term = new_term(60, 6);
    // Turn 1 learns DIM from a dim suggestion while normal text is non-dim.
    feed(
        &mut term,
        "\x1b[4;1H❯\u{a0}\x1b[2mTry something\x1b[22m\x1b[4;3H".as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "one".into() },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, "\x1b[4;1H\x1b[K❯\u{a0}one\x1b[4;6H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    feed(&mut term, "\x1b[4;1H\x1b[K❯\u{a0}\x1b[4;3H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(2), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );

    // Turn 2 starts blank, echoes a DIM-styled draft "two", then extends it to "twos".
    let turn2 = start + Duration::from_secs(5);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "two".into() },
        },
        turn2,
        &term.screen(),
    );
    feed(
        &mut term,
        "\x1b[4;1H\x1b[K❯\u{a0}\x1b[2mtwo\x1b[22m\x1b[4;6H".as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(turn2 + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "s".into() },
        },
        turn2 + Duration::from_millis(2),
        &term.screen(),
    );
    feed(
        &mut term,
        "\x1b[4;1H\x1b[K❯\u{a0}\x1b[2mtwos\x1b[22m\x1b[4;7H".as_bytes(),
    );
    assert!(tracker
        .observe_output(turn2 + Duration::from_millis(3), &term.screen())
        .is_none());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

    // Clearing to blank still returns to Idle.
    feed(&mut term, "\x1b[4;1H\x1b[K❯\u{a0}\x1b[4;3H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(turn2 + Duration::from_millis(4), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn two_chunk_clear_to_dim_suggestion_releases_when_cursor_returns_and_on_recheck() {
    for release_via_recheck in [false, true] {
        let start = Instant::now();
        let mut term = new_term(60, 8);
        feed(&mut term, "\x1b[4;1H❯\u{a0}\x1b[4;3H".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "draft".into(),
                },
            },
            start,
            &term.screen(),
        );
        feed(&mut term, "\x1b[4;1H\x1b[K❯\u{a0}draft\x1b[4;8H".as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term.screen())
                .unwrap()
                .state,
            InputState::Drafting,
            "release_via_recheck={release_via_recheck}"
        );

        // Chunk 1: paints dim suggestion on the editor row and a footer elsewhere, leaving the
        // cursor on the footer. Style-only clear defers without overwriting confirmed draft text.
        feed(
            &mut term,
            "\x1b[4;1H\x1b[K❯\u{a0}\x1b[2mRun tests\x1b[22m\x1b[6;1H\x1b[Kstatus footer".as_bytes(),
        );
        assert!(tracker
            .observe_output(start + Duration::from_millis(2), &term.screen())
            .is_none());
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Drafting,
            "release_via_recheck={release_via_recheck}"
        );

        // Chunk 2: only moves the cursor back to the start of the dim suggestion.
        feed(&mut term, b"\x1b[4;3H");
        if release_via_recheck {
            assert_eq!(
                tracker
                    .recheck_delivery(start + Duration::from_secs(30), &term.screen())
                    .unwrap()
                    .state,
                InputState::Idle
            );
        } else {
            assert_eq!(
                tracker
                    .observe_output(start + Duration::from_millis(3), &term.screen())
                    .unwrap()
                    .state,
                InputState::Idle
            );
            assert!(tracker
                .recheck_delivery(start + Duration::from_secs(30), &term.screen())
                .is_none());
        }
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    }
}

#[test]
fn transcript_repaint_with_parked_cursor_keeps_ignored_return_draft_held() {
    let start = Instant::now();
    let mut term = new_term(60, 8);
    feed(
        &mut term,
        "\x1b[2;1H❯ older prompt\x1b[6;1H❯ \x1b[6;3H".as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, "\x1b[6;1H\x1b[K❯ draft\x1b[6;8H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    assert_eq!(
        tracker
            .observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(2),
                &term.screen(),
            )
            .unwrap()
            .state,
        InputState::Submitted
    );
    // Ignored Return leaves row 5 unchanged ("❯ draft").
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(3), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );

    // A chunk repaints transcript row 1 and leaves the cursor at the end of "older prompt"
    // while row 5 still holds the intact draft.
    feed(&mut term, "\x1b[2;1H\x1b[K❯ older prompt".as_bytes());
    assert!(tracker
        .observe_output(start + Duration::from_millis(4), &term.screen())
        .is_none());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

    // Restoring the cursor to the intact draft keeps Drafting across observe_output and recheck.
    feed(&mut term, b"\x1b[6;8H");
    assert!(tracker
        .observe_output(start + Duration::from_millis(5), &term.screen())
        .is_none());
    assert!(tracker
        .recheck_delivery(start + Duration::from_secs(30), &term.screen())
        .is_none());
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

    // Clearing the draft on row 5 still releases to Idle.
    feed(&mut term, "\x1b[6;1H\x1b[K❯ \x1b[6;3H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_secs(31), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn recheck_diagnostic_dedup_survives_visibility_flicker_during_resize() {
    let start = Instant::now();
    let mut term = new_term(76, 10);
    feed(&mut term, "\x1b[6;1H❯\u{a0}\x1b[6;3H".as_bytes());
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "draft".into(),
            },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, "\x1b[6;1H\x1b[K❯\u{a0}draft\x1b[6;8H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );

    // First recheck emits a diagnostic.
    let first = tracker
        .record_recheck_draft_hold(start + Duration::from_secs(10), &term.screen())
        .expect("initial recheck diagnostic");
    assert!(first.visible);
    assert_eq!(first.tracked_row, Some(5));
    assert_eq!(first.cursor, Some((5, 7)));

    // Simulate a pane resize that briefly clears the composer row (visible: true -> false -> true)
    // within the same continuous Drafting hold.
    term.resize(69, 10);
    feed(&mut term, b"\x1b[6;1H\x1b[K\x1b[1;1H");
    let hidden = tracker
        .observe_output(start + Duration::from_secs(20), &term.screen())
        .unwrap();
    assert_eq!(hidden.state, InputState::Drafting);
    assert!(!hidden.composer_visible);
    feed(&mut term, "\x1b[6;1H\x1b[K❯\u{a0}draft\x1b[6;8H".as_bytes());
    let restored = tracker
        .observe_output(
            start + Duration::from_secs(20) + Duration::from_millis(10),
            &term.screen(),
        )
        .unwrap();
    assert_eq!(restored.state, InputState::Drafting);
    assert!(restored.composer_visible);

    // Unchanged diagnostic fields after the resize redraw must not log again.
    assert_eq!(
        tracker.record_recheck_draft_hold(start + Duration::from_secs(40), &term.screen()),
        None
    );

    // A genuine field change (e.g. cursor moves) still logs once.
    feed(&mut term, b"\x1b[6;4H");
    let moved = tracker
        .record_recheck_draft_hold(start + Duration::from_secs(41), &term.screen())
        .expect("cursor change logs a new diagnostic");
    assert_eq!(moved.cursor, Some((5, 3)));
    assert_eq!(
        tracker.record_recheck_draft_hold(start + Duration::from_secs(71), &term.screen()),
        None
    );

    // Leaving Drafting clears the cache so a later hold logs again.
    feed(&mut term, "\x1b[6;1H\x1b[K❯\u{a0}\x1b[6;3H".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_secs(72), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
    assert_eq!(tracker.last_recheck_diag, None);
}

#[test]
#[ignore = "manual M6.1 handoff measurement"]
fn measure_tracker_cost_on_a_200_by_60_grid() {
    const ITERATIONS: u32 = 20_000;
    let start = Instant::now();
    let mut term = new_term(200, 60);
    feed(&mut term, b"\x1b[59;1H\xe2\x9d\xaf Placeholder");
    let mut idle = InputTracker::new(start);
    idle.initial_observation(start);
    let measured_at = Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(idle.observe_output(Instant::now(), &term.screen()));
    }
    let idle_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;

    let mut drafting = InputTracker::new(start);
    drafting.initial_observation(start);
    drafting.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "x".into() },
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf x");
    drafting.observe_output(start + Duration::from_millis(1), &term.screen());
    let measured_at = Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(drafting.observe_output(Instant::now(), &term.screen()));
    }
    let drafting_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;

    let measured_at = Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(relocate_row(&term.screen(), "missing-prefix ", 30, None));
    }
    let relocation_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;

    let measured_at = Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(relocate_row(
            &term.screen(),
            "missing-prefix ",
            30,
            Some('─'),
        ));
    }
    let framed_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;
    eprintln!(
            "input tracker µs/chunk: idle={idle_us:.3} drafting={drafting_us:.3} relocation={relocation_us:.3} framed={framed_us:.3}"
        );
}
