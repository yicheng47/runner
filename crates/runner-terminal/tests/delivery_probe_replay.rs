use std::path::Path;

use runner_terminal::delivery_probe::{InputProbe, ProbeState};
use runner_terminal::fixtures::{decode_chunk, Fixture, FixtureEvent, FixtureInput};
use runner_terminal::input_state::InputEvent;
use runner_terminal::mappings::InputKind;
use runner_terminal::replay::{feed, new_term, row_cells, Terminal};

#[test]
fn recorded_composers_confirm_even_with_transcript_echoes() {
    for name in [
        "input-claude-control-submit",
        "input-codex-fresh-submit",
        "input-codex-filled-submit",
        "input-codex-queued-submit",
        "input-zsh-submit",
    ] {
        let fixture = Fixture::load(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures")
                .join(format!("{name}.ndjson")),
        )
        .unwrap();
        let mut term = new_term(fixture.header.cols, fixture.header.rows);
        let rows = |term: &Terminal| {
            (0..term.rows())
                .map(|row| row_cells(term, row as i32))
                .collect::<Vec<_>>()
        };
        let mut before = None;
        let mut typed = String::new();
        let mut probe = None;
        for event in fixture.events {
            match event {
                FixtureEvent::Data { data, .. } => feed(&mut term, &decode_chunk(&data).unwrap()),
                FixtureEvent::Input {
                    input:
                        FixtureInput::Event(InputEvent::Key {
                            kind: InputKind::Content { text },
                        }),
                    ..
                } if probe.is_none() => {
                    before.get_or_insert_with(|| rows(&term));
                    typed.push_str(&text);
                }
                FixtureEvent::Input {
                    input:
                        FixtureInput::Event(InputEvent::Key {
                            kind: InputKind::Submit,
                        }),
                    ..
                } if probe.is_none() => {
                    let mut input = InputProbe::new(before.take().unwrap(), &typed);
                    input.locate(&rows(&term));
                    assert_eq!(
                        input.state(&rows(&term)),
                        ProbeState::Present,
                        "{name}: before Enter"
                    );
                    probe = Some(input);
                }
                _ => {}
            }
        }
        assert_eq!(
            probe.unwrap().state(&rows(&term)),
            ProbeState::Cleared,
            "{name}: after Enter"
        );
    }
}
