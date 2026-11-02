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
        crate::i18n::text(self.label_key())
    }
    pub fn description(self) -> &'static str {
        crate::i18n::text(self.description_key())
    }
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Ask => "ui.ask",
            Self::Keep => "ui.keep_running",
            Self::Stop => "ui.stop_sessions",
        }
    }
    pub fn description_key(self) -> &'static str {
        match self {
            Self::Ask => "ui.ask_each_time_sessions_are_running",
            Self::Keep => "ui.sessions_keep_going_in_the_background",
            Self::Stop => "ui.stop_them_they_resume_on_next_launch",
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
    /// Sessions are stopping off the main thread; the app quits once they have.
    pub stopping: bool,
    /// The app has been told to quit and can no longer cancel it.
    pub committed: bool,
    owner: Option<gpui::WindowId>,
    generation: u64,
}
impl Global for QuitState {}
impl QuitState {
    pub fn begin(&mut self, owner: gpui::WindowId) -> Option<u64> {
        if self.asking || self.stopping {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.owner = Some(owner);
        self.asking = true;
        Some(self.generation)
    }
    pub fn is_current(&self, owner: gpui::WindowId, generation: u64) -> bool {
        self.asking && !self.stopping && self.owner == Some(owner) && self.generation == generation
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
        self.choice == QuitChoice::Stop && !update && !self.stopping
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
        crate::i18n::tnf(
            "format.session_running_working",
            self.live,
            &[("working_count", (self.working.len()).to_string())],
        )
    }
    pub fn stop_description(&self) -> String {
        let mut text = crate::i18n::text("ui.they_resume_the_next_time_you_open_runner").to_owned();
        match self.working.as_slice() {
            [] => {}
            [title] => text.push_str(&crate::i18n::tf(
                "format.is_working_and_loses_its_current_turn",
                &[("title", (title).to_string())],
            )),
            agents => text.push_str(&crate::i18n::tf(
                "format.agents_are_working_and_lose_their_current_turns",
                &[("working_count", (agents.len()).to_string())],
            )),
        }
        text
    }
}

pub fn working_caption(count: usize) -> Option<String> {
    (count > 0).then(|| crate::i18n::tnf("format.agent_working", count, &[]))
}

pub fn restart_message(count: usize, version: &str) -> Option<crate::i18n::Text> {
    (count > 0).then(|| {
        crate::i18n::message_count(
            "format.restarted_session_in_runner",
            count,
            &[("version", (version).to_string())],
        )
    })
}

pub fn crash_recovery_message(count: usize) -> crate::i18n::Text {
    crate::i18n::message_count(
        "format.background_service_restarted_session_stopped",
        count,
        &[],
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaemonNotice {
    Stopped,
    Repeated,
    InUse,
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
        state.stopping = true;
        assert!(!state.stop_sessions(false));
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
    fn crash_recovery_counts_use_singular_and_plural_sessions() {
        assert_eq!(
            crash_recovery_message(0).resolve(),
            "Background service restarted · 0 sessions stopped"
        );
        assert_eq!(
            crash_recovery_message(1).resolve(),
            "Background service restarted · 1 session stopped"
        );
        assert_eq!(
            crash_recovery_message(3).resolve(),
            "Background service restarted · 3 sessions stopped"
        );
    }

    #[test]
    fn existing_recovery_notice_resolves_again_after_switching() {
        use crate::i18n::{self, Locale};
        i18n::testing::with_locale(Locale::English, || {
            let message = crash_recovery_message(3);
            assert_eq!(
                message.resolve(),
                "Background service restarted · 3 sessions stopped"
            );
            i18n::set_active_locale(Locale::SimplifiedChinese);
            assert_eq!(message.resolve(), "后台服务已重启 · 3 个会话已停止");
            i18n::set_active_locale(Locale::English);
            assert_eq!(
                message.resolve(),
                "Background service restarted · 3 sessions stopped"
            );
        });
    }

    #[test]
    fn restart_and_update_counts_use_plain_words() {
        assert_eq!(working_caption(0), None);
        assert_eq!(working_caption(1).as_deref(), Some("1 agent working"));
        assert_eq!(working_caption(3).as_deref(), Some("3 agents working"));
        assert_eq!(restart_message(0, "0.13.1"), None);
        assert_eq!(
            restart_message(3, "0.13.1")
                .map(|message| message.resolve().to_string())
                .as_deref(),
            Some("Restarted 3 sessions in Runner 0.13.1")
        );
    }
}
