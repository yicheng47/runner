use super::*;
use crate::replay::{feed, new_term};

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
                    &term,
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
            tracker.observe_output(start + Duration::from_millis(30), &term);
            tracker.observe_output(start + Duration::from_secs(3), &term);
            assert_eq!(
                tracker.current_reported_state().state,
                InputState::Drafting,
                "prefix={prefix:?} length={length}\n{}",
                screen_snapshot(&term)
            );
            feed(&mut term, format!("\x1b[H\x1b[J{prefix}Ask").as_bytes());
            assert_eq!(
                tracker
                    .observe_output(start + Duration::from_secs(4), &term)
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
        &term,
    );
    feed(
        &mut term,
        b"\x1b[2;1H\x1b[K\xe2\x9d\xaf h\x1b[4;1H\x1b[K  \xe2\x94\x94 hooked 12 files",
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term)
            .unwrap()
            .state,
        InputState::Drafting
    );
    feed(&mut term, b"\x1b[2;1H\x1b[K\xe2\x9d\xaf Try a task");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(2), &term)
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
        &term,
    );
    let burst = [
        b"\x1b[2;1H\x1b[K\xe2\x9d\xaf h\x1b[4;1H\x1b[K  \xe2\x94\x94 hooked 12 files".as_slice(),
        b"\x1b[2;1H\x1b[K\xe2\x9d\xaf Try a task",
    ]
    .concat();
    feed(&mut term, &burst);
    assert!(tracker
        .observe_output(start + Duration::from_millis(2), &term)
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
            &term,
        )
        .is_none());
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
    let drafting = tracker
        .observe_output(start + Duration::from_millis(5), &term)
        .unwrap();
    assert_eq!(drafting.state, InputState::Drafting);
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf Try a task");
    let idle = tracker
        .observe_output(start + Duration::from_millis(10), &term)
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
        &term,
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term)
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
            .observe_output(start + Duration::from_millis(2), &term)
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
        &term,
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello world");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term)
            .unwrap()
            .state,
        InputState::Drafting
    );

    feed(
        &mut term,
        b"\r\x1b[K\xe2\x9d\xaf \x1b[2mhello world\x1b[22m",
    );
    assert!(tracker
        .observe_output(start + Duration::from_millis(2), &term)
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
        &term,
    );
    feed(
        &mut term,
        b"\r\x1b[K\xe2\x94\x82 \xe2\x80\xba h                     \xe2\x94\x82",
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term)
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
            .observe_output(start + Duration::from_millis(2), &term)
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
            &term,
        );
        feed(&mut term, "\x1b[4;1H\x1b[K› fix it".as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );

        // Codex redraws the trimmed transcript; a user row now sits at the tracked row.
        feed(
                &mut term,
                "\x1b[H\x1b[2J› first question\r\n\r\n• answer one\r\n› second question\r\n\r\n• answer two\x1b[10;1H› fix it".as_bytes(),
            );
        tracker.observe_output(start + Duration::from_millis(2), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

        tracker.observe_input(&InputEvent::Key { kind: kind.clone() }, start, &term);
        if kind == InputKind::Edit {
            feed(&mut term, "\x1b[10;1H\x1b[K› fix".as_bytes());
            tracker.observe_output(start + Duration::from_millis(3), &term);
            assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
            tracker.observe_input(&InputEvent::Key { kind: kind.clone() }, start, &term);
        }
        feed(
            &mut term,
            "\x1b[10;1H\x1b[K› \x1b[90mAsk Codex to do anything\x1b[39m\x1b[10;3H".as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(4), &term);
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
        &term,
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term)
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
        &term,
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
    tracker.observe_output(start + Duration::from_millis(1), &term);
    let guard = tracker.reset_guard();
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "i".into() },
        },
        start + Duration::from_millis(2),
        &term,
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
            tracker.track_delivery_input(text, start, &term);
        } else {
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Content { text: text.into() },
                },
                start,
                &term,
            );
        }
        feed(&mut term, format!("\x1b[9;1H> {text}").as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );
        if injected {
            tracker.track_delivery_submit(start + Duration::from_millis(2), &term);
        } else {
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(2),
                &term,
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
        tracker.observe_output(start + Duration::from_millis(3), &term);
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
        &term,
    );
    feed(&mut term, b"\x1b[3;1H> draft");
    tracker.observe_output(start + Duration::from_millis(1), &term);
    feed(
        &mut term,
        format!("\x1b[4;1H\x1b[K\x1b[7;1H{border}\r\n> draft\r\n{border}\x1b[8;8H").as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(2), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    feed(&mut term, b"\x1b[8;1H> \x1b[K");
    tracker.observe_output(start + Duration::from_millis(3), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
}

#[test]
fn framed_composer_keeps_multiline_drafts_held_and_releases_after_resize() {
    use alacritty_terminal::term::test::TermSize;

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
        &term,
    );
    feed(
        &mut term,
        format!("\x1b[3;1H> first\r\nsecond\r\n{border}").as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(1), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

    term.resize(TermSize::new(30, 12));
    let border = "─".repeat(30);
    feed(
        &mut term,
        format!("\x1b[H\x1b[J> first\r\nsecond\x1b[7;1H{border}\r\n> first\r\nsecond\r\n{border}")
            .as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(2), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    feed(
        &mut term,
        format!("\x1b[8;1H> \x1b[K\r\n{border}\r\n\x1b[K\x1b[8;3H").as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(3), &term);
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
            &term,
        );
        feed(
            &mut term,
            format!("\x1b[3;1H> first\r\nsecond\r\n{border}").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(1), &term);
        if hidden {
            feed(&mut term, b"\x1b[3;1Hmenu\x1b[K\r\nchoice\x1b[K");
        }
        feed(
            &mut term,
            format!("\x1b[8;1H{border}\r\n> Ask\r\n{border}\x1b[4;7H").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(2), &term);
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
        &term,
    );
    feed(&mut term, b"\x1b[3;1H> draft");
    tracker.observe_output(start + Duration::from_millis(1), &term);
    feed(
        &mut term,
        format!("\x1b[7;1H{border}\r\n> \r\n{border}").as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(2), &term);
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
        &term,
    );
    feed(&mut term, "\x1b[3;1H❯ draft".as_bytes());
    tracker.observe_output(start + Duration::from_millis(1), &term);
    feed(
            &mut term,
            format!(
                "\x1b[2;1H● working\x1b[K\r\n\x1b[K\r\n\x1b[K\x1b[5;1H{border}\r\n❯ draft\r\n{border}\x1b[6;8H"
            )
            .as_bytes(),
        );
    tracker.observe_output(start + Duration::from_millis(2), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    feed(&mut term, "\x1b[6;1H❯ \x1b[K\x1b[6;3H".as_bytes());
    tracker.observe_output(start + Duration::from_millis(3), &term);
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
        &term,
    );
    feed(&mut term, b"\x1b[3;1H> draft");
    tracker.observe_output(start + Duration::from_millis(1), &term);
    feed(
            &mut term,
            format!(
                "\x1b[2;1H\x1b[K\r\n\x1b[K\r\n\x1b[K\x1b[7;1H── plan ──\x1b[K\r\n> \x1b[K\r\n{border}\x1b[8;3H"
            )
            .as_bytes(),
        );
    tracker.observe_output(start + Duration::from_millis(2), &term);
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
        &term,
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello");
    tracker.observe_output(start + Duration::from_millis(5), &term);
    let submitted = tracker
        .observe_input(
            &InputEvent::Key {
                kind: InputKind::Submit,
            },
            start + Duration::from_millis(6),
            &term,
        )
        .unwrap();
    assert_eq!(submitted.state, InputState::Submitted);
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf Different placeholder");
    let idle = tracker
        .observe_output(start + Duration::from_millis(9), &term)
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
    tracker.track_delivery_input("[inbox] task", start, &term);
    tracker.track_delivery_submit(start, &term);
    feed(&mut term, "\r\x1b[K❯ [inbox] task".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(5), &term)
            .unwrap()
            .state,
        InputState::Unsent
    );
    // An ignored Enter with no later output leaves the notice where the guard can resubmit it.
    assert!(tracker.unsent_notice(&term));
    feed(&mut term, "\r\x1b[K❯ Different placeholder".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(9), &term)
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
    tracker.track_delivery_input("[inbox] task", start, &term);
    tracker.track_delivery_submit(start, &term);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: " draft".into(),
            },
        },
        start,
        &term,
    );
    feed(&mut term, "\r\x1b[K❯ [inbox] task draft".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(5), &term)
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
            tracker.observe_input(&InputEvent::Paste { text: text.into() }, start, &term);
        } else {
            tracker.track_delivery_input(text, start, &term);
        }
        feed(&mut term, format!("\r\x1b[K› {text}").as_bytes());
        tracker.observe_output(start + Duration::from_millis(5), &term);
        if person {
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(300),
                &term,
            );
        } else {
            tracker.track_delivery_submit(start + Duration::from_millis(300), &term);
        }
        feed(&mut term, format!("\r\x1b[K› {text}").as_bytes());
        tracker.observe_output(start + Duration::from_millis(400), &term);
        tracker.observe_output(start + Duration::from_secs(26 * 60), &term);
        assert_eq!(
            tracker.current_reported_state().state,
            if person {
                InputState::Drafting
            } else {
                InputState::Unsent
            },
            "person={person}"
        );
        assert_eq!(tracker.unsent_notice(&term), !person, "person={person}");
        if person {
            continue;
        }
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "!".into() },
            },
            start + Duration::from_secs(26 * 60 + 1),
            &term,
        );
        feed(&mut term, format!("\r\x1b[K› {text}!").as_bytes());
        let observation = tracker
            .observe_output(start + Duration::from_secs(26 * 60 + 2), &term)
            .unwrap();
        assert_eq!(observation.state, InputState::Drafting);
        assert!(!tracker.unsent_notice(&term));
    }
}

const NOTICE: &str = "[inbox] 1 new messages — run `runner msg read` to view.";

/// The router types its notice into the composer at row 4 and presses Enter.
fn submitted_notice(
    cols: u16,
    echo: &str,
) -> (
    Instant,
    Term<alacritty_terminal::event::VoidListener>,
    InputTracker,
) {
    let start = Instant::now();
    let mut term = new_term(cols, 8);
    feed(&mut term, b"\x1b[5;1H> Try a task");
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.track_delivery_input(NOTICE, start, &term);
    feed(&mut term, echo.as_bytes());
    tracker.observe_output(start + Duration::from_millis(1), &term);
    tracker.track_delivery_submit(start + Duration::from_millis(80), &term);
    (start, term, tracker)
}

#[test]
fn a_confirmed_notice_is_not_unsent_where_a_history_row_repeats_it() {
    let echo = format!("\x1b[5;1H\x1b[K> {NOTICE}");
    let (start, mut term, mut tracker) = submitted_notice(80, &echo);
    // The prompt stays in history at the composer row; the cursor is parked below the output.
    let history = format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[8;1H\x1b[K");
    feed(&mut term, history.as_bytes());
    tracker.observe_output(start + Duration::from_millis(100), &term);
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
    tracker.observe_output(start + Duration::from_secs(60), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    assert!(!tracker.unsent_notice(&term));
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
        tracker.observe_output(start + Duration::from_millis(100), &term);
        let revision = tracker.reset_guard();
        // A person types before the router's confirmation and before the echo.
        tracker.observe_input(&input, start + Duration::from_millis(200), &term);
        assert!(tracker
            .confirm_delivery(revision, start + Duration::from_millis(1600))
            .is_none());
        feed(&mut term, b"\x1b[7;1H\x1b[K> draft");
        tracker.observe_output(start + Duration::from_millis(1700), &term);
        tracker.recheck_delivery(start + Duration::from_secs(60), &term);
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
        tracker.observe_output(start + Duration::from_millis(100), &term);
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Unsent,
            "{case}"
        );
        tracker.recheck_delivery(start + Duration::from_secs(60), &term);
        assert_eq!(
            tracker.current_reported_state().state,
            if unsent {
                InputState::Unsent
            } else {
                InputState::Idle
            },
            "{case}"
        );
        assert_eq!(tracker.unsent_notice(&term), unsent, "{case}");
    }
}

const LINKED: &str = "[runner] Mission m1 is now linked to this chat.";

/// #1225: the router's line is submitted, and Claude Code redraws it as a transcript row at
/// the composer's row while the turn runs, with the input cursor parked below the frame.
fn linked_line_in_transcript() -> (
    Instant,
    Term<alacritty_terminal::event::VoidListener>,
    InputTracker,
) {
    let start = Instant::now();
    let mut term = new_term(60, 10);
    feed(
        &mut term,
        "\x1b[1;1H✻ Welcome\x1b[3;1H❯ \x1b[3;3H".as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.track_delivery_input(LINKED, start, &term);
    feed(&mut term, format!("\x1b[3;1H\x1b[K❯ {LINKED}").as_bytes());
    tracker.observe_output(start + Duration::from_millis(1), &term);
    tracker.track_delivery_submit(start + Duration::from_millis(80), &term);
    feed(
        &mut term,
        format!("\x1b[3;1H\x1b[K❯ {LINKED}\x1b[5;1H✻ Contemplating…\x1b[7;1H❯ \x1b[10;1H")
            .as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(89), &term);
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
        &term,
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
    tracker.observe_output(start + Duration::from_secs(14), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);

    // A composition that did start still counts as input.
    tracker.observe_input(&InputEvent::Composing { composing: true }, start, &term);
    let composing = tracker.reset_guard();
    assert_ne!(composing, revision);
    tracker.observe_input(&InputEvent::Composing { composing: false }, start, &term);
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
        &term,
    );
    assert!(tracker
        .confirm_delivery(revision, start + Duration::from_millis(1580))
        .is_none());
    feed(&mut term, TURN_END.as_bytes());
    tracker.observe_output(start + Duration::from_secs(14), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    assert!(!tracker.unsent_notice(&term));
}

#[test]
fn a_recheck_drops_a_draft_the_empty_editor_does_not_show() {
    let (start, mut term, mut tracker) = linked_line_in_transcript();
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Navigate,
        },
        start + Duration::from_millis(300),
        &term,
    );
    // The turn-end frame arrives before Claude Code returns the cursor to its composer.
    feed(&mut term, TURN_END.as_bytes());
    feed(&mut term, b"\x1b[10;1H");
    tracker.observe_output(start + Duration::from_secs(14), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    tracker.recheck_delivery(start + Duration::from_secs(15), &term);
    assert_eq!(
        tracker.current_reported_state().state,
        InputState::Drafting,
        "no editor under the cursor yet"
    );
    feed(&mut term, b"\x1b[8;3H");
    tracker.recheck_delivery(start + Duration::from_secs(16), &term);
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
            &term,
        );
        feed(&mut term, TURN_END.as_bytes());
        feed(&mut term, b"\x1b[10;1H");
        tracker.observe_output(start + Duration::from_secs(14), &term);
        feed(&mut term, b"\x1b[8;3H");
        let typed = start + Duration::from_secs(15);
        tracker.observe_input(&typed_before_echo(pasted), typed, &term);
        tracker.recheck_delivery(typed + Duration::from_millis(10), &term);
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Drafting,
            "pasted={pasted}: the echo is still on its way"
        );
        feed(&mut term, "\x1b[8;1H\x1b[K❯ draft".as_bytes());
        tracker.observe_output(typed + Duration::from_millis(30), &term);
        tracker.recheck_delivery(typed + Duration::from_secs(5), &term);
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
                &term,
            );
            let typed = start + Duration::from_secs(14);
            tracker.observe_input(&typed_before_echo(pasted), typed, &term);
            feed(&mut term, frame.as_bytes());
            tracker.observe_output(typed + Duration::from_millis(10), &term);
            feed(&mut term, echo.as_bytes());
            tracker.observe_output(typed + Duration::from_millis(30), &term);
            tracker.recheck_delivery(typed + Duration::from_secs(5), &term);
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
    tracker.track_delivery_input(LINKED, start, &term);
    feed(&mut term, format!("\x1b[3;1H\x1b[K❯ {LINKED}").as_bytes());
    tracker.observe_output(start + Duration::from_millis(1), &term);
    assert!(!tracker.delivery_held(), "not submitted yet");
    tracker.track_delivery_submit(start + Duration::from_millis(80), &term);
    // The first frame after Enter already shows the empty composer below the transcript row.
    feed(
        &mut term,
        format!("\x1b[3;1H\x1b[K❯ {LINKED}\x1b[5;1H❯ \x1b[5;3H").as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(89), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    assert!(tracker.delivery_held());

    // A line whose echo was never located is not one the tracker held.
    tracker.track_delivery_input("[inbox] task", start + Duration::from_secs(5), &term);
    assert!(!tracker.delivery_held());
    tracker.track_delivery_submit(start + Duration::from_secs(5), &term);
    tracker.observe_output(start + Duration::from_secs(6), &term);
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
        &term,
    );
    feed(&mut term, "\x1b[3;1H\x1b[K❯ half a thought".as_bytes());
    tracker.observe_output(start + Duration::from_millis(5), &term);
    tracker.recheck_delivery(start + Duration::from_secs(5), &term);
    assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    // A turn end redraws the transcript above the draft and returns the cursor to it.
    feed(
        &mut term,
        "\x1b[1;1H⏺ Done again.\x1b[K\x1b[3;1H\x1b[K❯ half a thought\x1b[3;17H".as_bytes(),
    );
    tracker.observe_output(start + Duration::from_secs(10), &term);
    tracker.recheck_delivery(start + Duration::from_secs(11), &term);
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
            &term,
        );
        feed(&mut term, format!("\r\x1b[K❯ {typed}").as_bytes());
        tracker.observe_output(start + Duration::from_millis(5), &term);
        tracker.track_delivery_input("[inbox] task", start + Duration::from_millis(6), &term);
        feed(
            &mut term,
            format!("\r\x1b[K❯ {typed}[inbox] task").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(9), &term);
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
        &term,
    );
    feed(&mut term, b"\r\x1b[K$ x");
    let drafting = tracker
        .observe_output(start + Duration::from_millis(1), &term)
        .unwrap();
    assert_eq!(drafting.state, InputState::Drafting);
    feed(&mut term, b"\r\x1b[Kmenu");
    let hidden = tracker
        .observe_output(start + Duration::from_millis(2), &term)
        .unwrap();
    assert_eq!(hidden.state, InputState::Drafting);
    assert!(!hidden.composer_visible);
    feed(&mut term, b"\x1b[1;1H\x1b[K$ x");
    let visible = tracker
        .observe_output(start + Duration::from_millis(3), &term)
        .unwrap();
    assert!(visible.composer_visible);
    feed(&mut term, b"\r\x1b[K$ ");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(4), &term)
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn resize_below_the_tracked_row_relocates_and_observes_the_clear() {
    use alacritty_terminal::term::test::TermSize;

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
        &term,
    );
    feed(&mut term, b"\r\x1b[K$ x");
    tracker.observe_output(start + Duration::from_millis(1), &term);

    term.resize(TermSize::new(30, 10));
    feed(&mut term, b"\x1b[2J\x1b[8;1H$ x");
    assert!(tracker
        .observe_output(start + Duration::from_millis(2), &term)
        .is_none());
    feed(&mut term, b"\r\x1b[K$ ");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(3), &term)
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
        &term,
    );
    feed(&mut term, b"\r\x1b[K$ x");
    tracker.observe_output(start + Duration::from_millis(1), &term);
    feed(&mut term, b"\r\x1b[Kmenu");
    tracker.observe_output(start + Duration::from_millis(2), &term);
    assert_eq!(
        tracker
            .observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(3),
                &term,
            )
            .unwrap()
            .state,
        InputState::Submitted
    );
    feed(&mut term, b"\r\x1b[K$ x");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(4), &term)
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
        &term,
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello");
    tracker.observe_output(start + Duration::from_millis(1), &term);
    assert_eq!(
        tracker
            .observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(2),
                &term,
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
            &term,
        )
        .is_none());
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf n");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(4), &term)
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
        &term,
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello");
    tracker.observe_output(start + Duration::from_millis(1), &term);
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Submit,
        },
        start + Duration::from_millis(2),
        &term,
    );
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "w".into() },
        },
        start + Duration::from_millis(3),
        &term,
    );

    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf Try a task");
    assert!(tracker
        .observe_output(start + Duration::from_millis(4), &term)
        .is_none());
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf w");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(5), &term)
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
        &term,
    );
    assert!(tracker
        .observe_input(
            &InputEvent::Key {
                kind: InputKind::Submit,
            },
            start + Duration::from_millis(1),
            &term,
        )
        .is_none());
    feed(&mut term, b"\r\x1b[K$ x");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(2), &term)
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
        &term,
    );
    feed(&mut term, b"\r\x1b[K$ x");
    tracker.observe_output(start + Duration::from_millis(1), &term);
    feed(&mut term, b"\r\x1b[Kpopup\x1b[3;1H$ x");
    assert!(tracker
        .observe_output(start + Duration::from_millis(2), &term)
        .is_none());
    feed(&mut term, b"\r\x1b[K$ ");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(3), &term)
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
            &term,
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
            &term,
        )
        .is_none());
    let idle = tracker
        .observe_input(
            &InputEvent::Composing { composing: false },
            start + Duration::from_millis(3),
            &term,
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
                &term,
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
        tracker.track_delivery_input("[inbox] task", start, &term);
        feed(&mut term, "\r\x1b[K› [inbox] task".as_bytes());
        tracker.observe_output(start, &term);
        tracker.track_delivery_submit(start, &term);
        tracker.observe_output(start, &term);
        let revision = tracker.reset_guard();
        tracker.observe_input(&InputEvent::Key { kind }, start, &term);
        feed(&mut term, b"\x1b[D");
        tracker.observe_output(start + Duration::from_secs(11 * 60), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Unsent);
        assert!(tracker.unsent_notice(&term));
        assert_ne!(tracker.reset_guard(), revision);
        assert!(tracker.reset_if_unchanged(revision, start).is_none());
        feed(&mut term, "\r\x1b[K› recalled prompt".as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_secs(11 * 60 + 1), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );
        assert!(!tracker.unsent_notice(&term));
    }
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
        std::hint::black_box(idle.observe_output(Instant::now(), &term));
    }
    let idle_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;

    let mut drafting = InputTracker::new(start);
    drafting.initial_observation(start);
    drafting.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content { text: "x".into() },
        },
        start,
        &term,
    );
    feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf x");
    drafting.observe_output(start + Duration::from_millis(1), &term);
    let measured_at = Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(drafting.observe_output(Instant::now(), &term));
    }
    let drafting_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;

    let measured_at = Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(relocate_row(&term, "missing-prefix ", 30, None));
    }
    let relocation_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;

    let measured_at = Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(relocate_row(&term, "missing-prefix ", 30, Some('─')));
    }
    let framed_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;
    eprintln!(
            "input tracker µs/chunk: idle={idle_us:.3} drafting={drafting_us:.3} relocation={relocation_us:.3} framed={framed_us:.3}"
        );
}
