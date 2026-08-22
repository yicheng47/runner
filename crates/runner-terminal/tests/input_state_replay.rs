use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use runner_terminal::fixtures::{decode_chunk, Fixture, FixtureEvent, FixtureInput};
use runner_terminal::input_state::{InputObservation, InputTracker};
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
                        tracker.observe_output(started + Duration::from_millis(ms), &term),
                    )
                }
                FixtureEvent::Input {
                    ms,
                    input: FixtureInput::Event(input),
                } => (
                    ms,
                    tracker.observe_input(&input, started + Duration::from_millis(ms), &term),
                ),
                FixtureEvent::Input {
                    input: FixtureInput::LegacyBytes(_),
                    ..
                }
                | FixtureEvent::Exit { .. } => continue,
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
