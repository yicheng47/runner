use std::time::Instant;

pub(crate) fn now() -> Instant {
    #[cfg(test)]
    if let Some(time) = REPLAY_TIME.with_borrow(|time| *time) {
        return time.0;
    }
    Instant::now()
}

pub(crate) fn timestamp_millis() -> i64 {
    #[cfg(test)]
    if let Some(time) = REPLAY_TIME.with_borrow(|time| *time) {
        return time.1;
    }
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
thread_local! {
    static REPLAY_TIME: std::cell::RefCell<Option<(Instant, i64)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn at<T>(now: Instant, timestamp_millis: i64, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<(Instant, i64)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            REPLAY_TIME.with_borrow_mut(|time| *time = self.0);
        }
    }
    let _restore =
        Restore(REPLAY_TIME.with_borrow_mut(|time| time.replace((now, timestamp_millis))));
    run()
}

pub(crate) fn state_now() -> super::state::Now {
    super::state::Now {
        monotonic: now(),
        wall_millis: timestamp_millis(),
    }
}
