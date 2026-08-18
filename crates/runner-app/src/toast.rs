pub const DEFAULT_TOAST_DURATION_MS: u64 = 6_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ToastTone {
    #[default]
    Info,
    Success,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub message: String,
    pub tone: ToastTone,
    pub duration_ms: Option<u64>,
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

    pub fn show(&mut self, message: impl Into<String>, tone: ToastTone) -> u64 {
        self.show_with_duration(message, tone, Some(DEFAULT_TOAST_DURATION_MS))
    }

    pub fn show_with_duration(
        &mut self,
        message: impl Into<String>,
        tone: ToastTone,
        duration_ms: Option<u64>,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.active = Some(Toast {
            id,
            message: message.into(),
            tone,
            duration_ms,
        });
        id
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
    fn default_toast_has_the_react_timeout() {
        let mut host = ToastHost::default();
        host.show("Saved", ToastTone::Success);
        let toast = host.active().unwrap();
        assert_eq!(toast.duration_ms, Some(6_000));
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
