use std::time::{Duration, Instant};

pub(crate) const SLOW_STAGE_THRESHOLD: Duration = Duration::from_secs(2);

pub(crate) struct StageClock {
    started: Instant,
    #[cfg(test)]
    fake: Option<(
        std::sync::Arc<std::sync::atomic::AtomicU64>,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
        u64,
    )>,
}

impl StageClock {
    #[cfg(not(test))]
    pub(crate) fn start(_scope: &str) -> Self {
        Self {
            started: Instant::now(),
        }
    }

    #[cfg(test)]
    pub(crate) fn start(scope: &str) -> Self {
        let fake = TEST_PROBES.lock().unwrap().get(scope).map(|probe| {
            let now = probe.clock_ns.load(std::sync::atomic::Ordering::Acquire);
            (
                std::sync::Arc::clone(&probe.clock_ns),
                std::sync::Arc::clone(&probe.step_ns),
                now,
            )
        });
        Self {
            started: Instant::now(),
            fake,
        }
    }

    pub(crate) fn elapsed(&self) -> Duration {
        #[cfg(test)]
        if let Some((clock, step, started_ns)) = &self.fake {
            let delta = step.load(std::sync::atomic::Ordering::Acquire);
            let now_ns = if delta > 0 {
                clock
                    .fetch_add(delta, std::sync::atomic::Ordering::AcqRel)
                    .wrapping_add(delta)
            } else {
                clock.load(std::sync::atomic::Ordering::Acquire)
            };
            return Duration::from_nanos(now_ns.saturating_sub(*started_ns));
        }
        self.started.elapsed()
    }
}

pub(crate) fn warn_if_slow(
    #[cfg_attr(not(test), allow(unused_variables))] scope: &str,
    elapsed: Duration,
    threshold: Duration,
    build_message: impl FnOnce() -> String,
) {
    if elapsed < threshold {
        return;
    }
    let message = build_message();
    log::warn!("{message}");
    #[cfg(test)]
    if let Some(sink) = TEST_PROBES
        .lock()
        .unwrap()
        .get(scope)
        .map(|probe| std::sync::Arc::clone(&probe.warnings))
    {
        sink.lock().unwrap().push(message);
    }
}

#[cfg(test)]
struct TestProbe {
    clock_ns: std::sync::Arc<std::sync::atomic::AtomicU64>,
    step_ns: std::sync::Arc<std::sync::atomic::AtomicU64>,
    warnings: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[cfg(test)]
static TEST_PROBES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, TestProbe>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

#[cfg(test)]
pub(crate) struct SlowStageProbeGuard {
    scope: String,
    step_ns: std::sync::Arc<std::sync::atomic::AtomicU64>,
    warnings: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[cfg(test)]
impl SlowStageProbeGuard {
    pub(crate) fn install(scope: impl Into<String>) -> Self {
        let scope = scope.into();
        let clock_ns = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let step_ns = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let warnings = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        TEST_PROBES.lock().unwrap().insert(
            scope.clone(),
            TestProbe {
                clock_ns,
                step_ns: std::sync::Arc::clone(&step_ns),
                warnings: std::sync::Arc::clone(&warnings),
            },
        );
        Self {
            scope,
            step_ns,
            warnings,
        }
    }

    pub(crate) fn set_elapsed_step(&self, step: Duration) {
        self.step_ns
            .store(step.as_nanos() as u64, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn take_warnings(&self) -> Vec<String> {
        std::mem::take(&mut *self.warnings.lock().unwrap())
    }
}

#[cfg(test)]
impl Drop for SlowStageProbeGuard {
    fn drop(&mut self) {
        TEST_PROBES.lock().unwrap().remove(&self.scope);
    }
}
