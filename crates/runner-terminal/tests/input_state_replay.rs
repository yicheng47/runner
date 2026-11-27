use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use runner_terminal::fixtures::{decode_chunk, Fixture, FixtureEvent, FixtureInput};
use runner_terminal::input_state::{InputEvent, InputObservation, InputState, InputTracker};
use runner_terminal::mappings::InputKind;
use runner_terminal::replay::{feed, new_term};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn transition(ms: u64, observation: InputObservation) -> String {
    format!(
        "{ms} {:?} composing={} visible={}",
        observation.state, observation.composing, observation.composer_visible
    )
}

#[test]
fn native_submit_before_echo_releases_at_a_new_empty_composer() {
    for (cols, empty) in [
        (40, "Ask Codex to do anything"),
        (120, "Ask Codex to do anything"),
        (120, "Review this repository"),
        (120, ""),
    ] {
        for pasted in [false, true] {
            for delayed_empty in [false, true] {
                submitted_history_releases_at_a_new_empty_composer(
                    false,
                    cols,
                    pasted,
                    delayed_empty,
                    empty,
                );
            }
        }
    }
}

#[test]
fn native_submit_after_echo_releases_at_a_new_empty_composer() {
    for (cols, empty) in [
        (40, "Ask Codex to do anything"),
        (120, "Ask Codex to do anything"),
        (120, "Review this repository"),
        (120, ""),
    ] {
        for pasted in [false, true] {
            for delayed_empty in [false, true] {
                submitted_history_releases_at_a_new_empty_composer(
                    true,
                    cols,
                    pasted,
                    delayed_empty,
                    empty,
                );
            }
        }
    }
}

fn submitted_history_releases_at_a_new_empty_composer(
    echo_before_submit: bool,
    cols: u16,
    pasted: bool,
    delayed_empty: bool,
    empty: &str,
) {
    let start = Instant::now();
    let mut term = new_term(cols, 10);
    feed(
        &mut term,
        "\x1b[2;1H› \x1b[90mAsk Codex to do anything\x1b[39m\x1b[2;3H".as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    let mut state = tracker.initial_observation(start).state;
    let text =
        "Run python3 /private/tmp/runner-940.fixture/qa_worker.py read once. Then ACK QA940.";
    if pasted {
        tracker.observe_input(
            &InputEvent::Paste { text: text.into() },
            start,
            &term.screen(),
        );
    } else {
        for character in text.chars() {
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Content {
                        text: character.to_string(),
                    },
                },
                start,
                &term.screen(),
            );
        }
    }
    if echo_before_submit {
        feed(&mut term, format!("\x1b[2;1H\x1b[K› {text}").as_bytes());
        state = tracker.observe_output(start, &term.screen()).unwrap().state;
        assert_eq!(state, InputState::Drafting);
    }
    if let Some(observation) = tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Submit,
        },
        start,
        &term.screen(),
    ) {
        state = observation.state;
    }
    if delayed_empty {
        feed(
            &mut term,
            format!("\x1b[2;1H\x1b[K› {text}\x1b[4;1HACK QA940").as_bytes(),
        );
        if let Some(observation) =
            tracker.observe_output(start + Duration::from_millis(1), &term.screen())
        {
            state = observation.state;
        }
        assert_eq!(state, InputState::Drafting);
    }
    feed(
        &mut term,
        format!(
            "\x1b[2;1H\x1b[K› {text}\x1b[4;1HACK QA940\x1b[7;1H› \x1b[90m{empty}\x1b[39m\x1b[7;3H"
        )
        .as_bytes(),
    );
    if let Some(observation) =
        tracker.observe_output(start + Duration::from_millis(30), &term.screen())
    {
        state = observation.state;
    }
    assert_eq!(state, InputState::Idle);
    assert!(tracker
        .observe_output(start + Duration::from_secs(30), &term.screen())
        .is_none());

    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Content {
                text: "unsent".into(),
            },
        },
        start + Duration::from_secs(31),
        &term.screen(),
    );
    feed(&mut term, "\x1b[7;1H\x1b[K› unsent".as_bytes());
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_secs(31), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    feed(&mut term, b"\x1b[3S\x1b[4;9H");
    assert!(tracker
        .observe_output(start + Duration::from_secs(60), &term.screen())
        .is_none());
    feed(
        &mut term,
        format!("\x1b[4;1H\x1b[K› \x1b[90m{empty}\x1b[39m\x1b[4;3H").as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_secs(61), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn a_rejected_native_probe_submit_holds_on_its_only_redraw() {
    for reprobe in [false, true] {
        let start = Instant::now();
        let mut term = new_term(80, 10);
        feed(&mut term, "\x1b[2;1H› Ask Codex".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        let submit = InputEvent::Key {
            kind: InputKind::Submit,
        };
        if reprobe {
            tracker.observe_input(
                &InputEvent::Paste { text: "old".into() },
                start,
                &term.screen(),
            );
            feed(&mut term, "\x1b[2;1H\x1b[K› old".as_bytes());
            tracker.observe_output(start, &term.screen());
            tracker.observe_input(&submit, start, &term.screen());
        }
        tracker.observe_input(
            &InputEvent::Paste {
                text: "line one\nline two".into(),
            },
            start,
            &term.screen(),
        );
        tracker.observe_input(&submit, start, &term.screen());
        feed(
            &mut term,
            "\x1b[2;1H\x1b[K› line one\x1b[3;1H  line two\x1b[4;3H".as_bytes(),
        );
        let observation = tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap();
        assert_eq!(observation.state, InputState::Drafting, "reprobe={reprobe}");
    }
}

#[test]
fn native_submit_during_reprobe_releases_at_a_new_empty_composer() {
    let start = Instant::now();
    let mut term = new_term(80, 10);
    feed(&mut term, "\x1b[2;1H› Ask Codex".as_bytes());
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Paste { text: "old".into() },
        start,
        &term.screen(),
    );
    feed(&mut term, "\x1b[2;1H\x1b[K› old".as_bytes());
    tracker.observe_output(start, &term.screen());
    let submit = InputEvent::Key {
        kind: InputKind::Submit,
    };
    tracker.observe_input(&submit, start, &term.screen());
    tracker.observe_input(
        &InputEvent::Paste { text: "new".into() },
        start,
        &term.screen(),
    );
    tracker.observe_input(&submit, start, &term.screen());
    feed(
        &mut term,
        "\x1b[2J\x1b[1;1H› old\x1b[4;1H› new\x1b[7;1H› Ask Codex\x1b[7;3H".as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn submit_before_echo_with_scrolled_transcript_and_pinned_composer_releases_1381() {
    let wrapped = "QA1377-WORK-TURN: Run python3 /tmp/runner-qa-1377/mission/working.py. \
        Handle any inbox notification during the turn, read and ACK it once as instructed, \
        then finish with WORK-DONE-QA1377. No other work.";
    for cols in [40, 78] {
        for empty in ["Ask Codex to do anything", ""] {
            for text in [wrapped, "hi"] {
                for pasted in [false, true] {
                    for submitted_row in [4, 6] {
                        let start = Instant::now();
                        let mut term = new_term(cols, 10);
                        feed(
                            &mut term,
                            format!(
                                "\x1b[2;1HWorked for 18s\
                                 \x1b[4;1H› [inbox] 1 new messages — run `runner msg read` to view.\
                                 \x1b[9;1H› \x1b[2m{empty}\x1b[22m\x1b[9;3H"
                            )
                            .as_bytes(),
                        );
                        let mut tracker = InputTracker::new(start);
                        let mut state = tracker.initial_observation(start).state;
                        if pasted {
                            tracker.observe_input(
                                &InputEvent::Paste { text: text.into() },
                                start,
                                &term.screen(),
                            );
                        } else {
                            for character in text.chars() {
                                tracker.observe_input(
                                    &InputEvent::Key {
                                        kind: InputKind::Content {
                                            text: character.to_string(),
                                        },
                                    },
                                    start,
                                    &term.screen(),
                                );
                            }
                        }
                        if let Some(observation) = tracker.observe_input(
                            &InputEvent::Key {
                                kind: InputKind::Submit,
                            },
                            start,
                            &term.screen(),
                        ) {
                            state = observation.state;
                        }
                        let head = text
                            .chars()
                            .take(usize::from(cols).saturating_sub(2))
                            .collect::<String>();
                        feed(
                            &mut term,
                            format!(
                                "\x1b[2J\x1b[1;1H› [inbox] 1 new messages — run `runner msg read` to view.\
                                 \x1b[{submitted_row};1H› {head}\
                                 \x1b[7;1H• Working (0s • esc to interrupt)\
                                 \x1b[9;1H› \x1b[2m{empty}\x1b[22m\x1b[9;3H"
                            )
                            .as_bytes(),
                        );
                        if let Some(observation) = tracker
                            .observe_output(start + Duration::from_millis(20), &term.screen())
                        {
                            state = observation.state;
                        }
                        assert_eq!(
                            state, InputState::Idle,
                            "cols={cols} empty={empty:?} text={text:?} pasted={pasted} row={submitted_row}"
                        );
                        assert!(tracker
                            .recheck_delivery(start + Duration::from_secs(30), &term.screen())
                            .is_none());

                        tracker.observe_input(
                            &InputEvent::Key {
                                kind: InputKind::Content {
                                    text: "draft".into(),
                                },
                            },
                            start + Duration::from_secs(31),
                            &term.screen(),
                        );
                        feed(&mut term, "\x1b[9;1H\x1b[K› draft\x1b[9;8H".as_bytes());
                        assert_eq!(
                            tracker
                                .observe_output(start + Duration::from_secs(31), &term.screen())
                                .unwrap()
                                .state,
                            InputState::Drafting
                        );
                        feed(
                            &mut term,
                            format!("\x1b[9;1H\x1b[K› \x1b[2m{empty}\x1b[22m\x1b[9;3H").as_bytes(),
                        );
                        assert_eq!(
                            tracker
                                .observe_output(start + Duration::from_secs(32), &term.screen())
                                .unwrap()
                                .state,
                            InputState::Idle
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn wrapped_draft_with_nearby_scrolled_transcript_keeps_cursor_composer_empty_form_1381() {
    let start = Instant::now();
    let mut term = new_term(40, 10);
    feed(
        &mut term,
        "\x1b[5;1H› earlier transcript prompt\
         \x1b[9;1H› \x1b[2mAsk Codex to do anything\x1b[22m\x1b[9;3H"
            .as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Paste {
            text: "first wrapped draft line that scrolls the screen\nand a continuation\nand third\nand fourth"
                .into(),
        },
        start,
        &term.screen(),
    );
    feed(
        &mut term,
        "\x1b[2J\x1b[2;1H› earlier transcript prompt\
         \x1b[6;1H› first wrapped draft line that scrolls\
         \x1b[7;1H  the screen\
         \x1b[8;1H  and a continuation\
         \x1b[9;1H  and fourth\x1b[9;13H"
            .as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(10), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Submit,
        },
        start + Duration::from_millis(20),
        &term.screen(),
    );
    feed(
        &mut term,
        "\x1b[2J\x1b[1;1H› earlier transcript prompt\
         \x1b[3;1H› first wrapped draft line that scrolls\
         \x1b[9;1H› \x1b[2mAsk Codex to do anything\x1b[22m\x1b[9;3H"
            .as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(30), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn delivery_submit_before_echo_with_pinned_composer_marks_delivery_held_and_releases_1381() {
    let start = Instant::now();
    let mut term = new_term(78, 10);
    feed(
        &mut term,
        "\x1b[3;1H› prior user prompt\
         \x1b[9;1H› \x1b[2mAsk Codex to do anything\x1b[22m\x1b[9;3H"
            .as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    let notice = "[inbox] 1 new messages — run `runner msg read` to view.";
    tracker.track_delivery_input(notice, start, &term.screen());
    tracker.track_delivery_submit(start + Duration::from_millis(10), &term.screen());
    feed(
        &mut term,
        format!(
            "\x1b[2J\x1b[1;1H› prior user prompt\
             \x1b[5;1H› {notice}\
             \x1b[9;1H› \x1b[2mAsk Codex to do anything\x1b[22m\x1b[9;3H"
        )
        .as_bytes(),
    );
    assert!(tracker
        .observe_output(start + Duration::from_millis(20), &term.screen())
        .is_none());
    assert_eq!(tracker.latest_observation().state, InputState::Idle);
    assert!(tracker.delivery_held());
}

#[test]
fn reprobe_submit_before_echo_with_pinned_composer_and_scrolled_history_releases_1381() {
    let start = Instant::now();
    let mut term = new_term(78, 10);
    feed(
        &mut term,
        "\x1b[3;1H› [inbox] 1 new messages — run `runner msg read` to view.\
         \x1b[9;1H› \x1b[2mAsk Codex to do anything\x1b[22m\x1b[9;3H"
            .as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    let submit = InputEvent::Key {
        kind: InputKind::Submit,
    };

    tracker.observe_input(
        &InputEvent::Paste {
            text: "first".into(),
        },
        start,
        &term.screen(),
    );
    feed(&mut term, "\x1b[9;1H\x1b[K› first\x1b[9;8H".as_bytes());
    tracker.observe_output(start + Duration::from_millis(5), &term.screen());
    tracker.observe_input(&submit, start + Duration::from_millis(10), &term.screen());

    // Before the first submit's empty composer renders, a second wrapped prompt is pasted and
    // submitted, and the next frame shows the pinned empty composer back on row 9.
    feed(
        &mut term,
        "\x1b[9;1H\x1b[K› \x1b[2mReview this repository\x1b[22m\x1b[9;3H".as_bytes(),
    );
    tracker.observe_input(
        &InputEvent::Paste {
            text: "second multiline prompt\nwith continuation".into(),
        },
        start + Duration::from_millis(15),
        &term.screen(),
    );
    tracker.observe_input(&submit, start + Duration::from_millis(20), &term.screen());
    feed(
        &mut term,
        "\x1b[2J\x1b[1;1H› [inbox] 1 new messages — run `runner msg read` to view.\
         \x1b[3;1H› first\
         \x1b[5;1H› second multiline prompt\
         \x1b[6;1H  with continuation\
         \x1b[9;1H› \x1b[2mReview this repository\x1b[22m\x1b[9;3H"
            .as_bytes(),
    );
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(30), &term.screen())
            .unwrap()
            .state,
        InputState::Idle
    );
}

#[test]
fn history_only_redraw_before_rejected_submit_echo_preserves_pending_probe_1381() {
    for history_frame in [
        "\x1b[2J\x1b[1;1H› prior user prompt\x1b[9;1H› \x1b[2mAsk Codex to do anything\x1b[22m\x1b[9;3H",
        "\x1b[2J\x1b[1;1H› prior user prompt\x1b[4;1H› unrelated streamed prompt\x1b[9;1H› \x1b[2mAsk Codex to do anything\x1b[22m\x1b[9;3H",
    ] {
        for reprobe in [false, true] {
            let start = Instant::now();
            let mut term = new_term(78, 10);
            feed(
                &mut term,
                "\x1b[3;1H› prior user prompt\
                 \x1b[9;1H› \x1b[2mAsk Codex to do anything\x1b[22m\x1b[9;3H"
                    .as_bytes(),
            );
            let mut tracker = InputTracker::new(start);
            tracker.initial_observation(start);
            let submit = InputEvent::Key {
                kind: InputKind::Submit,
            };

            if reprobe {
                tracker.observe_input(&InputEvent::Paste { text: "old".into() }, start, &term.screen());
                feed(&mut term, "\x1b[9;1H\x1b[K› old\x1b[9;6H".as_bytes());
                tracker.observe_output(start + Duration::from_millis(2), &term.screen());
                tracker.observe_input(&submit, start + Duration::from_millis(4), &term.screen());
                feed(
                    &mut term,
                    "\x1b[9;1H\x1b[K› \x1b[2mAsk Codex to do anything\x1b[22m\x1b[9;3H"
                        .as_bytes(),
                );
            }

            tracker.observe_input(
                &InputEvent::Paste {
                    text: "new line one\nnew line two".into(),
                },
                start + Duration::from_millis(5),
                &term.screen(),
            );
            tracker.observe_input(&submit, start + Duration::from_millis(6), &term.screen());

            // Only transcript history redraws before the pasted draft reaches the composer.
            feed(&mut term, history_frame.as_bytes());
            tracker.observe_output(start + Duration::from_millis(10), &term.screen());

            // The paste echoes next with Return rejected, leaving the draft in the composer.
            feed(
                &mut term,
                "\x1b[9;1H\x1b[K› new line one\x1b[10;1H  new line two\x1b[10;15H".as_bytes(),
            );
            tracker.observe_output(start + Duration::from_millis(20), &term.screen());
            tracker.recheck_delivery(start + Duration::from_secs(30), &term.screen());
            assert_eq!(
                tracker.latest_observation().state,
                InputState::Drafting,
                "reprobe={reprobe} history_frame={history_frame:?}"
            );
        }
    }
}

#[test]
fn collapsed_paste_submit_with_pinned_composer_ignores_scrolled_paste_and_releases_on_new_one_1381()
{
    let start = Instant::now();
    let mut term = new_term(78, 10);
    feed(
        &mut term,
        "\x1b[3;1H❯ [Pasted text #1 +19 lines]\
         \x1b[9;1H❯ \x1b[2mTry a task\x1b[22m\x1b[9;3H"
            .as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.track_delivery_input("line one\nline two", start, &term.screen());
    tracker.track_delivery_submit(start + Duration::from_millis(2), &term.screen());

    // Scrolling the prior collapsed paste alone does not satisfy the pending delivery probe.
    feed(
        &mut term,
        "\x1b[2J\x1b[1;1H❯ [Pasted text #1 +19 lines]\
         \x1b[9;1H❯ \x1b[2mTry a task\x1b[22m\x1b[9;3H"
            .as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(8), &term.screen());
    assert!(!tracker.delivery_held());

    // Rendering the new collapsed paste in transcript while restoring the pinned composer does.
    feed(
        &mut term,
        "\x1b[2J\x1b[1;1H❯ [Pasted text #1 +19 lines]\
         \x1b[5;1H❯ [Pasted text #2 +1 lines]\
         \x1b[9;1H❯ \x1b[2mTry a task\x1b[22m\x1b[9;3H"
            .as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(16), &term.screen());
    assert_eq!(tracker.latest_observation().state, InputState::Idle);
    assert!(tracker.delivery_held());
}

#[test]
fn a_rejected_multiline_submit_holds_with_the_cursor_on_a_blank_continuation() {
    let start = Instant::now();
    let mut term = new_term(80, 10);
    feed(&mut term, "\x1b[2;1H› Ask Codex".as_bytes());
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Paste {
            text: "line one\nline two".into(),
        },
        start,
        &term.screen(),
    );
    feed(
        &mut term,
        "\x1b[2;1H\x1b[K› line one\x1b[3;1H  line two".as_bytes(),
    );
    tracker.observe_output(start, &term.screen());
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Submit,
        },
        start,
        &term.screen(),
    );
    feed(&mut term, b"\x1b[4;3H");
    assert_eq!(
        tracker
            .observe_output(start + Duration::from_millis(1), &term.screen())
            .unwrap()
            .state,
        InputState::Drafting
    );
    assert!(tracker
        .observe_output(start + Duration::from_secs(30), &term.screen())
        .is_none());
}

#[test]
fn a_composition_return_cannot_clear_a_draft_using_another_empty_prompt() {
    let start = Instant::now();
    let mut term = new_term(80, 10);
    feed(
        &mut term,
        "\x1b[2;1H› Ask Codex\x1b[7;1H› Ask Codex\x1b[2;3H".as_bytes(),
    );
    let mut tracker = InputTracker::new(start);
    tracker.initial_observation(start);
    tracker.observe_input(
        &InputEvent::Paste {
            text: "line one\nline two".into(),
        },
        start,
        &term.screen(),
    );
    tracker.observe_input(
        &InputEvent::Composing { composing: true },
        start,
        &term.screen(),
    );
    tracker.observe_input(
        &InputEvent::Key {
            kind: InputKind::Submit,
        },
        start,
        &term.screen(),
    );
    feed(
        &mut term,
        "\x1b[2;1H\x1b[K› line one\x1b[3;1H  line two\x1b[7;3H".as_bytes(),
    );
    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
    let observation = tracker
        .observe_input(
            &InputEvent::Composing { composing: false },
            start + Duration::from_millis(2),
            &term.screen(),
        )
        .unwrap();
    assert_eq!(observation.state, InputState::Drafting);
    assert!(tracker
        .observe_output(start + Duration::from_secs(30), &term.screen())
        .is_none());
}

#[test]
fn recorded_input_transitions_follow_the_grid() {
    let mut paths = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("input-") && name.ends_with(".ndjson"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "input fixture corpus is empty");

    for path in paths {
        let fixture = Fixture::load(&path).unwrap_or_else(|error| {
            panic!("load {}: {error:#}", path.display());
        });
        let started = Instant::now();
        let mut term = new_term(fixture.header.cols, fixture.header.rows);
        let mut tracker = InputTracker::new(started);
        let mut actual = vec![transition(0, tracker.initial_observation(started))];
        for event in fixture.events {
            let (ms, observation) = match event {
                FixtureEvent::Data { ms, data } => {
                    feed(&mut term, &decode_chunk(&data).unwrap());
                    (
                        ms,
                        tracker.observe_output(started + Duration::from_millis(ms), &term.screen()),
                    )
                }
                FixtureEvent::Input {
                    ms,
                    input: FixtureInput::Event(input),
                } => (
                    ms,
                    tracker.observe_input(
                        &input,
                        started + Duration::from_millis(ms),
                        &term.screen(),
                    ),
                ),
                FixtureEvent::Input {
                    input: FixtureInput::LegacyBytes(_),
                    ..
                }
                | FixtureEvent::Exit { .. } => continue,
                FixtureEvent::Resize { cols, rows, .. } => {
                    term.resize(cols, rows);
                    continue;
                }
            };
            if let Some(observation) = observation {
                actual.push(transition(ms, observation));
            }
        }
        let actual = format!("{}\n", actual.join("\n"));
        let expected_path = path.with_extension("expected.txt");
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("missing {}", expected_path.display()));
        assert_eq!(actual, expected, "transition drift in {}", path.display());
    }
}
