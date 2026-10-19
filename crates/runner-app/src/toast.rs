pub const DEFAULT_TOAST_DURATION_MS: u64 = 4_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastTone {
    #[default]
    Info,
    Success,
    Error,
    Restart,
    Warning,
}

impl ToastTone {
    pub fn icon(self) -> &'static str {
        match self {
            Self::Info => "info.svg",
            Self::Success => "circle-check.svg",
            Self::Error => "circle-x.svg",
            Self::Warning => "triangle-alert.svg",
            Self::Restart => "rotate-cw.svg",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub message: String,
    source: runner_app::i18n::Text,
    pub tone: ToastTone,
    pub duration_ms: Option<u64>,
    pub single_line: bool,
    pub icon: Option<&'static str>,
    pub bottom: bool,
}

#[derive(Debug, Default)]
pub struct ToastHost {
    active: Option<Toast>,
    next_id: u64,
}

impl ToastHost {
    pub fn active(&self) -> Option<&Toast> {
        self.active.as_ref()
    }

    pub fn show(&mut self, message: impl Into<runner_app::i18n::Text>, tone: ToastTone) -> u64 {
        self.show_with_duration(message, tone, Some(DEFAULT_TOAST_DURATION_MS))
    }

    pub fn show_with_duration(
        &mut self,
        message: impl Into<runner_app::i18n::Text>,
        tone: ToastTone,
        duration_ms: Option<u64>,
    ) -> u64 {
        let source = message.into();
        self.next_id += 1;
        let id = self.next_id;
        self.active = Some(Toast {
            id,
            message: source.resolve().to_string(),
            source,
            tone,
            duration_ms,
            single_line: false,
            icon: None,
            bottom: false,
        });
        id
    }

    pub fn relocalize(&mut self) {
        if let Some(toast) = &mut self.active {
            toast.message = toast.source.resolve().to_string();
        }
    }

    pub fn make_single_line(&mut self) {
        if let Some(toast) = &mut self.active {
            toast.single_line = true;
        }
    }

    pub fn pin_lock_warning(&mut self) {
        if let Some(toast) = &mut self.active {
            toast.icon = Some("lock.svg");
            toast.bottom = true;
            toast.single_line = true;
        }
    }

    pub fn dismiss(&mut self) {
        self.active = None;
    }

    pub fn expire(&mut self, id: u64) -> bool {
        if self.active.as_ref().is_some_and(|toast| toast.id == id) {
            self.active = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toast_dismisses_after_four_seconds() {
        let mut host = ToastHost::default();
        host.show("Saved", ToastTone::Success);
        let toast = host.active().unwrap();
        assert_eq!(toast.duration_ms, Some(DEFAULT_TOAST_DURATION_MS));
        assert_eq!(DEFAULT_TOAST_DURATION_MS, 4_000);
        assert_eq!(toast.tone, ToastTone::Success);
    }

    #[test]
    fn a_new_toast_replaces_the_current_one() {
        let mut host = ToastHost::default();
        let stale = host.show("First", ToastTone::Info);
        let current = host.show_with_duration("Second", ToastTone::Error, None);
        assert_ne!(stale, current);
        assert_eq!(host.active().unwrap().message, "Second");
        assert!(!host.expire(stale));
        assert_eq!(host.active().unwrap().id, current);
    }

    #[test]
    fn toast_can_expire_or_be_dismissed() {
        let mut host = ToastHost::default();
        let id = host.show("Info", ToastTone::Info);
        assert!(host.expire(id));
        assert!(host.active().is_none());

        host.show_with_duration("Persistent", ToastTone::Error, None);
        host.dismiss();
        assert!(host.active().is_none());
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn restart_icon_preserves_other_tones() {
        let mut host = ToastHost::default();
        let message = runner_app::lifecycle::restart_message(3, "0.13.1").unwrap();
        host.show(message, ToastTone::Restart);
        let toast = host.active().unwrap();
        assert_eq!(toast.tone.icon(), "rotate-cw.svg");
        assert_eq!(toast.message, "Restarted 3 sessions in Runner 0.13.1");
        assert_eq!(ToastTone::Info.icon(), "info.svg");
        assert_eq!(ToastTone::Success.icon(), "circle-check.svg");
        assert_eq!(ToastTone::Error.icon(), "circle-x.svg");
    }
}

#[cfg(all(test, debug_assertions))]
mod localization_tests {
    use super::*;
    use runner_app::i18n::{testing::with_locale, Locale};

    #[test]
    fn existing_toast_relocalizes_while_literal_backend_text_is_preserved() {
        with_locale(Locale::English, || {
            let mut host = ToastHost::default();
            let id = host.show(
                runner_app::lifecycle::restart_message(2, "0.16.3").unwrap(),
                ToastTone::Restart,
            );
            with_locale(Locale::SimplifiedChinese, || {
                host.relocalize();
                assert_eq!(
                    host.active().unwrap().message,
                    "已在 Runner 0.16.3 中重启 2 个会话"
                );
                assert_eq!(host.active().unwrap().id, id);
                host.show("backend {count} 用户", ToastTone::Error);
                host.relocalize();
                assert_eq!(host.active().unwrap().message, "backend {count} 用户");
            });
        });
    }
}
