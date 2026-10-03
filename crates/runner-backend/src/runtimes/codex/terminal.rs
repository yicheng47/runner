use crate::runtimes::{TerminalAdapter, TerminalEvent, TerminalInput};
use crate::session::runtime::SessionActivityState;
use crate::session::state::agent::AgentEvent;

pub(crate) struct CodexTerminal {
    startup: Option<CodexStartup>,
    title: CodexTitleHint,
    hook_owned: bool,
}

impl CodexTerminal {
    pub(crate) fn new(pending_turn: bool) -> Self {
        Self {
            startup: Some(CodexStartup {
                pending_turn,
                input_pending: false,
                submitted_input: false,
                hooks_available: true,
                ready: false,
                readiness: Default::default(),
            }),
            title: Default::default(),
            hook_owned: false,
        }
    }
}

impl TerminalAdapter for CodexTerminal {
    fn on_output(&mut self, bytes: &[u8]) -> Option<TerminalEvent> {
        let title_activity = (!self.hook_owned)
            .then(|| self.title.observe(bytes))
            .flatten();
        if let Some(startup) = self.startup.as_mut() {
            startup.ready |= startup.readiness.observe(bytes)[0];
            if !startup.submitted_input && (startup.pending_turn || startup.ready) {
                let state = if startup.pending_turn {
                    SessionActivityState::Busy
                } else {
                    SessionActivityState::Idle
                };
                self.title.reset();
                return Some(TerminalEvent::Ready(state));
            }
        }
        title_activity.map(TerminalEvent::Title)
    }
    fn on_input(&mut self, bytes: &[u8]) -> TerminalInput {
        let mut input = TerminalInput::default();
        if bytes == b"\r" {
            let title_was_idle = self.title.classifier.activity == Some(SessionActivityState::Idle);
            self.title.reset();
            if title_was_idle {
                input.state = Some(SessionActivityState::Busy);
                input.refresh = true;
                input.announce = true;
            }
        }
        if let Some(startup) = self.startup.as_mut() {
            if bytes == b"\r" {
                // A local submission can be a native command, not a model turn.
                if startup.input_pending && !startup.pending_turn {
                    startup.submitted_input = true;
                    input.state = Some(SessionActivityState::Busy);
                    input.refresh = true;
                }
                startup.input_pending = false;
                input.announce = true;
                if (startup.pending_turn || startup.submitted_input) && !startup.hooks_available {
                    self.startup = None;
                    input.state = Some(SessionActivityState::Busy);
                    input.refresh = true;
                }
            } else {
                use crate::session::manager::{classify_local_input, LocalInputClass};
                let input = if matches!(bytes, b"\x1b[A" | b"\x1b[B" | b"\x1bOA" | b"\x1bOB") {
                    Some(LocalInputClass::SetPending)
                } else {
                    classify_local_input(bytes)
                };
                match input {
                    Some(LocalInputClass::SetPending) => startup.input_pending = true,
                    Some(LocalInputClass::ClearPending) => startup.input_pending = false,
                    _ => {}
                }
            }
        }
        input
    }
    fn held_activity(&self) -> Option<SessionActivityState> {
        self.startup
            .as_ref()
            .filter(|startup| !startup.submitted_input && (startup.pending_turn || startup.ready))
            .map(|startup| {
                if startup.pending_turn {
                    SessionActivityState::Busy
                } else {
                    SessionActivityState::Idle
                }
            })
    }
    fn hooks_unavailable(&mut self) {
        self.hook_owned = false;
        self.title.reset();
        if let Some(startup) = self.startup.as_mut() {
            startup.hooks_available = false;
            if startup.pending_turn || startup.submitted_input {
                self.startup = None;
            }
        }
    }
    fn accept_event(&mut self, event: &AgentEvent) -> bool {
        if self
            .startup
            .as_ref()
            .is_some_and(|startup| startup.pending_turn || startup.submitted_input)
            && event.startup_ready()
        {
            return false;
        }
        self.hook_owned = true;
        self.title.reset();
        self.startup = None;
        true
    }
}

struct CodexStartup {
    pending_turn: bool,
    input_pending: bool,
    submitted_input: bool,
    hooks_available: bool,
    ready: bool,
    readiness: crate::session::runtime::TuiReadiness,
}

const CODEX_TITLE_SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

#[derive(Default)]
struct CodexTitleHint {
    parser: vte::Parser,
    classifier: CodexTitleClassifier,
}

#[derive(Default)]
struct CodexTitleClassifier {
    confirmed: bool,
    candidate: Option<(char, String)>,
    activity: Option<SessionActivityState>,
    working_payload: Option<String>,
    idle_title: Option<String>,
    rename_suffix: Option<String>,
    rename_completed: bool,
}

enum TitleEvent {
    Set(String),
    Reset,
}

#[derive(Default)]
struct TitleEvents(Vec<TitleEvent>);

impl vte::Perform for TitleEvents {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if !matches!(params.first(), Some(selector) if *selector == b"0" || *selector == b"2") {
            return;
        }
        let mut raw = Vec::new();
        for (index, part) in params.iter().skip(1).enumerate() {
            if index > 0 {
                raw.push(b';');
            }
            raw.extend_from_slice(part);
        }
        match String::from_utf8(raw) {
            Ok(title) if !title.is_empty() => self.0.push(TitleEvent::Set(title)),
            _ => self.0.push(TitleEvent::Reset),
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if !ignore && intermediates.is_empty() && byte == b'c' {
            self.0.push(TitleEvent::Reset);
        }
    }
}

impl CodexTitleHint {
    fn observe(&mut self, bytes: &[u8]) -> Option<SessionActivityState> {
        let mut events = TitleEvents::default();
        self.parser.advance(&mut events, bytes);
        for event in events.0 {
            match event {
                TitleEvent::Set(title) => self.classifier.observe(&title),
                TitleEvent::Reset => self.classifier.clear_authority(),
            }
        }
        self.classifier.activity
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

impl CodexTitleClassifier {
    fn observe(&mut self, title: &str) {
        if let Some((frame, payload)) = codex_working_title(title) {
            let confirmed = self.confirmed
                || self
                    .candidate
                    .as_ref()
                    .is_some_and(|(previous_frame, previous_payload)| {
                        *previous_frame != frame
                            && same_codex_spinner_shape(previous_payload, payload)
                    });
            self.candidate = Some((frame, payload.to_owned()));
            self.idle_title = None;
            self.rename_suffix = None;
            self.rename_completed = false;
            if confirmed {
                self.confirmed = true;
                self.activity = Some(SessionActivityState::Busy);
                self.working_payload = Some(payload.to_owned());
            } else {
                self.activity = None;
                self.working_payload = None;
            }
            return;
        }

        if self.confirmed
            && self.activity == Some(SessionActivityState::Busy)
            && self
                .working_payload
                .as_deref()
                .is_some_and(|payload| same_codex_spinner_shape(payload, title))
        {
            self.activity = Some(SessionActivityState::Idle);
            self.idle_title = Some(title.to_owned());
            self.rename_suffix = codex_rename_suffix(title).map(str::to_owned);
            self.rename_completed = false;
            self.candidate = None;
            return;
        }

        if self.activity == Some(SessionActivityState::Idle)
            && self
                .idle_title
                .as_deref()
                .is_some_and(|previous| same_codex_spinner_shape(previous, title))
        {
            self.idle_title = Some(title.to_owned());
            return;
        }

        if self.activity == Some(SessionActivityState::Idle)
            && self.rename_suffix.as_deref() == Some(title)
        {
            self.rename_completed = true;
            self.idle_title = Some(title.to_owned());
            return;
        }

        if self.activity == Some(SessionActivityState::Idle)
            && self.rename_completed
            && self.rename_suffix.as_deref().is_some_and(|suffix| {
                title
                    .strip_suffix(suffix)
                    .is_some_and(|prefix| !prefix.is_empty() && prefix.ends_with(" | "))
            })
        {
            self.idle_title = Some(title.to_owned());
            return;
        }

        self.clear_authority();
    }

    fn clear_authority(&mut self) {
        self.candidate = None;
        self.activity = None;
        self.working_payload = None;
        self.idle_title = None;
        self.rename_suffix = None;
        self.rename_completed = false;
    }
}

fn codex_working_title(title: &str) -> Option<(char, &str)> {
    let mut chars = title.chars();
    let frame = chars.next()?;
    if !CODEX_TITLE_SPINNER_FRAMES.contains(&frame) || chars.next() != Some(' ') {
        return None;
    }
    let payload = chars.as_str();
    (!payload.is_empty()).then_some((frame, payload))
}

fn same_codex_spinner_shape(left: &str, right: &str) -> bool {
    let mut left = left.chars();
    let mut right = right.chars();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(left), Some(right))
                if left == right
                    || (CODEX_TITLE_SPINNER_FRAMES.contains(&left)
                        && CODEX_TITLE_SPINNER_FRAMES.contains(&right)) => {}
            _ => return false,
        }
    }
}

fn codex_rename_suffix(title: &str) -> Option<&str> {
    let mut rest = title.strip_prefix("renaming... ")?.chars();
    let frame = rest.next()?;
    if !CODEX_TITLE_SPINNER_FRAMES.contains(&frame) {
        return None;
    }
    let suffix = rest.as_str().strip_prefix(" | ")?;
    (!suffix.is_empty()).then_some(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn osc_title(title: &str) -> Vec<u8> {
        format!("\x1b]0;{title}\x07").into_bytes()
    }

    #[test]
    fn codex_title_hint_requires_recorded_activity_frames_and_matching_rest() {
        let mut title = CodexTitleHint::default();
        assert_eq!(title.observe(&osc_title("⠙ project")), None);
        assert_eq!(
            title.observe(&osc_title("⠹ project")),
            Some(SessionActivityState::Busy)
        );
        assert_eq!(
            title.observe(&osc_title("project")),
            Some(SessionActivityState::Idle)
        );

        let mut rename = CodexTitleHint::default();
        assert_eq!(
            rename.observe(&osc_title("⠙ renaming... ⠙ | project")),
            None
        );
        assert_eq!(
            rename.observe(&osc_title("⠹ renaming... ⠹ | project")),
            Some(SessionActivityState::Busy)
        );
        assert_eq!(
            rename.observe(&osc_title("renaming... ⠹ | project")),
            Some(SessionActivityState::Idle)
        );

        let mut next_frame = CodexTitleHint::default();
        assert_eq!(
            next_frame.observe(&osc_title("⠹ renaming... ⠹ | project")),
            None
        );
        assert_eq!(
            next_frame.observe(&osc_title("⠸ renaming... ⠸ | project")),
            Some(SessionActivityState::Busy)
        );
        assert_eq!(
            next_frame.observe(&osc_title("renaming... ⠼ | project")),
            Some(SessionActivityState::Idle)
        );
        assert_eq!(
            rename.observe(&osc_title("renaming... ⠸ | project")),
            Some(SessionActivityState::Idle)
        );
        assert_eq!(
            rename.observe(&osc_title("project")),
            Some(SessionActivityState::Idle)
        );
        assert_eq!(
            rename.observe(&osc_title("Generated topic | project")),
            Some(SessionActivityState::Idle)
        );
    }

    #[test]
    fn codex_title_hint_rejects_missing_reset_custom_and_embedded_signals() {
        for title in [
            "project",
            "",
            "Ready | project",
            "Working | project",
            "topic ⠙ | project",
            "renaming... ⠙ | project",
            "user@host:~/⠙-project",
            "✳ Claude Code",
            "⠂ Claude Code",
        ] {
            let mut hint = CodexTitleHint::default();
            assert_eq!(hint.observe(&osc_title(title)), None, "{title:?}");
        }

        let mut hint = CodexTitleHint::default();
        assert_eq!(hint.observe(&osc_title("⠙ project")), None);
        assert_eq!(
            hint.observe(&osc_title("⠹ project")),
            Some(SessionActivityState::Busy)
        );
        assert_eq!(hint.observe(b"\x1b]0;\x07"), None);
        assert_eq!(hint.observe(&osc_title("project")), None);
        assert_eq!(
            hint.observe(&osc_title("⠙ project")),
            Some(SessionActivityState::Busy)
        );
        assert_eq!(hint.observe(b"\x1bc"), None);
        assert_eq!(hint.observe(&osc_title("project")), None);
    }

    #[test]
    fn codex_title_hint_parses_split_osc_and_preserves_semicolons() {
        let mut hint = CodexTitleHint::default();
        let first = osc_title("⠙ project; branch");
        let second = osc_title("⠹ project; branch");
        for split in 1..first.len() {
            let mut hint = CodexTitleHint::default();
            assert_eq!(hint.observe(&first[..split]), None);
            assert_eq!(hint.observe(&first[split..]), None);
            assert_eq!(
                hint.observe(&second),
                Some(SessionActivityState::Busy),
                "split {split}"
            );
        }
        assert_eq!(hint.observe(&first), None);
        assert_eq!(hint.observe(&second), Some(SessionActivityState::Busy));
        assert_eq!(
            hint.observe(&osc_title("project; branch")),
            Some(SessionActivityState::Idle)
        );
    }

    #[test]
    fn codex_title_authority_does_not_survive_a_new_runtime() {
        let mut first = CodexTitleHint::default();
        assert_eq!(first.observe(&osc_title("⠙ project")), None);
        assert_eq!(
            first.observe(&osc_title("⠹ project")),
            Some(SessionActivityState::Busy)
        );
        assert_eq!(
            first.observe(&osc_title("project")),
            Some(SessionActivityState::Idle)
        );

        let mut resumed = CodexTitleHint::default();
        assert_eq!(resumed.observe(&osc_title("project")), None);
    }
}
