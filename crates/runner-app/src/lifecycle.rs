use std::sync::atomic::{AtomicBool, Ordering};

use gpui::Global;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuitBehavior {
    Keep,
    Stop,
    #[default]
    #[serde(other)]
    Ask,
}

impl QuitBehavior {
    pub const ALL: [Self; 3] = [Self::Ask, Self::Keep, Self::Stop];
    pub fn key(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Keep => "keep",
            Self::Stop => "stop",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "Ask",
            Self::Keep => "Keep running",
            Self::Stop => "Stop sessions",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Ask => "Ask each time sessions are running",
            Self::Keep => "Sessions keep going in the background",
            Self::Stop => "Stop them; they resume on next launch",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuitChoice {
    #[default]
    Keep,
    Stop,
}
impl QuitChoice {
    pub fn behavior(self) -> QuitBehavior {
        match self {
            Self::Keep => QuitBehavior::Keep,
            Self::Stop => QuitBehavior::Stop,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuitRequest {
    User,
    StopSessions,
    Os,
    Update,
}

pub fn quit_choice(
    request: QuitRequest,
    behavior: QuitBehavior,
    live: usize,
) -> Option<QuitChoice> {
    match request {
        QuitRequest::Os | QuitRequest::Update => Some(QuitChoice::Keep),
        QuitRequest::StopSessions => Some(QuitChoice::Stop),
        QuitRequest::User if live == 0 => Some(QuitChoice::Keep),
        QuitRequest::User => match behavior {
            QuitBehavior::Ask => None,
            QuitBehavior::Keep => Some(QuitChoice::Keep),
            QuitBehavior::Stop => Some(QuitChoice::Stop),
        },
    }
}

#[derive(Default)]
pub struct QuitState {
    pub choice: QuitChoice,
    pub asking: bool,
    owner: Option<gpui::WindowId>,
    generation: u64,
}
impl Global for QuitState {}
impl QuitState {
    pub fn begin(&mut self, owner: gpui::WindowId) -> Option<u64> {
        if self.asking {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.owner = Some(owner);
        self.asking = true;
        Some(self.generation)
    }
    pub fn is_current(&self, owner: gpui::WindowId, generation: u64) -> bool {
        self.asking && self.owner == Some(owner) && self.generation == generation
    }
    pub fn cancel(&mut self, owner: gpui::WindowId) {
        if self.owner == Some(owner) {
            self.owner = None;
            self.asking = false;
        }
    }
    pub fn complete(&mut self, owner: gpui::WindowId, generation: u64) {
        if self.is_current(owner, generation) {
            self.cancel(owner);
        }
    }
    pub fn stop_sessions(&self, update: bool) -> bool {
        self.choice == QuitChoice::Stop && !update
    }
}

static UPDATE_QUIT: AtomicBool = AtomicBool::new(false);
pub fn mark_update_quit() {
    UPDATE_QUIT.store(true, Ordering::Release);
}
pub fn update_quit_pending() -> bool {
    UPDATE_QUIT.load(Ordering::Acquire)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionSummary {
    pub live: usize,
    pub working: Vec<String>,
}
impl SessionSummary {
    pub fn caption(&self) -> String {
        format!(
            "{} session{} running · {} working",
            self.live,
            if self.live == 1 { "" } else { "s" },
            self.working.len()
        )
    }
    pub fn stop_description(&self) -> String {
        let mut text = "They resume the next time you open Runner.".to_owned();
        match self.working.as_slice() {
            [] => {}
            [title] => text.push_str(&format!(
                " \"{title}\" is working and loses its current turn."
            )),
            agents => text.push_str(&format!(
                " {} agents are working and lose their current turns.",
                agents.len()
            )),
        }
        text
    }
}

pub fn working_caption(count: usize) -> Option<String> {
    (count > 0).then(|| format!("{count} agent{} working", if count == 1 { "" } else { "s" }))
}

pub fn restart_message(count: usize, version: &str) -> Option<String> {
    (count > 0).then(|| {
        format!(
            "Restarted {count} session{} in Runner {version}",
            if count == 1 { "" } else { "s" }
        )
    })
}

pub fn crash_recovery_message(count: usize) -> String {
    format!("Background service restarted · {count} sessions stopped")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaemonNotice {
    Stopped,
    Repeated,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_quit_path_has_the_required_outcome() {
        for behavior in QuitBehavior::ALL {
            assert_eq!(
                quit_choice(QuitRequest::Os, behavior, 3),
                Some(QuitChoice::Keep)
            );
            assert_eq!(
                quit_choice(QuitRequest::Update, behavior, 3),
                Some(QuitChoice::Keep)
            );
            assert_eq!(
                quit_choice(QuitRequest::StopSessions, behavior, 3),
                Some(QuitChoice::Stop)
            );
            assert_eq!(
                quit_choice(QuitRequest::User, behavior, 0),
                Some(QuitChoice::Keep)
            );
        }
        assert_eq!(quit_choice(QuitRequest::User, QuitBehavior::Ask, 1), None);
        assert_eq!(
            quit_choice(QuitRequest::User, QuitBehavior::Keep, 1),
            Some(QuitChoice::Keep)
        );
        assert_eq!(
            quit_choice(QuitRequest::User, QuitBehavior::Stop, 1),
            Some(QuitChoice::Stop)
        );
    }
    #[test]
    fn app_quit_default_is_keep_and_update_overrides_a_pending_stop() {
        let mut state = QuitState::default();
        assert!(!state.stop_sessions(false));
        state.choice = QuitChoice::Stop;
        assert!(state.stop_sessions(false));
        assert!(!state.stop_sessions(true));
    }
    #[test]
    fn stop_line_names_one_working_agent_and_counts_several() {
        let mut summary = SessionSummary {
            live: 3,
            working: vec![],
        };
        assert_eq!(
            summary.stop_description(),
            "They resume the next time you open Runner."
        );
        summary.working.push("Refactor the settings store".into());
        assert!(summary
            .stop_description()
            .contains("\"Refactor the settings store\" is working and loses its current turn."));
        summary.working.push("Test".into());
        assert!(summary
            .stop_description()
            .contains("2 agents are working and lose their current turns."));
        assert_eq!(summary.caption(), "3 sessions running · 2 working");
    }
    #[test]
    fn restart_and_update_counts_use_plain_words() {
        assert_eq!(working_caption(0), None);
        assert_eq!(working_caption(1).as_deref(), Some("1 agent working"));
        assert_eq!(working_caption(3).as_deref(), Some("3 agents working"));
        assert_eq!(restart_message(0, "0.13.1"), None);
        assert_eq!(
            restart_message(3, "0.13.1").as_deref(),
            Some("Restarted 3 sessions in Runner 0.13.1")
        );
    }
}
