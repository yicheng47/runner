//! Panic hook that writes the panic body + a captured backtrace to
//! the same `log::error!` sink as the rest of the app, then chains
//! to the previously-installed hook.
//!
//! Chaining is deliberate: the default hook prints to stderr and is
//! what the OS CrashReporter keys on. Replacing it outright would
//! suppress that stderr line; we only want to *add* a persistent log
//! line in front of the existing abort/unwind path.
//!
//! Two writes happen per panic:
//!
//!  1. `log::error!(...)` — picked up by `tauri-plugin-log` once its
//!     setup callback has installed the global `log` subscriber. The
//!     usual path for any panic after Tauri's plugins have come up.
//!
//!  2. A direct best-effort append to `fallback_path`. Covers the
//!     window between `install()` and the plugin's setup running —
//!     panics during builder construction, plugin construction, or
//!     another plugin's setup land somewhere persistent even though
//!     no `log` subscriber has attached yet. This is the "next
//!     user-reported crash is unrecoverable" case spec #18 exists to
//!     fix.
//!
//! The two writes are independent — the file target uses a separate
//! fd, so we don't try to dedupe. Two log lines per crash is fine.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

pub fn install(fallback_path: PathBuf) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        log::error!("panic: {info}\n{bt}");
        // Direct-file fallback: append to fallback_path so a panic
        // that fires before the log plugin's setup callback runs
        // still has a persistent home. Best-effort: we're already
        // panicking, so swallow every error here.
        let _ = write_fallback(&fallback_path, info, &bt);
        prev(info);
    }));
}

fn write_fallback(
    path: &PathBuf,
    info: &std::panic::PanicHookInfo<'_>,
    bt: &std::backtrace::Backtrace,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    let ts = chrono::Utc::now().to_rfc3339();
    writeln!(f, "[{ts}] panic: {info}\n{bt}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Combined smoke test: installing the hook must (a) not break
    // the test harness — libtest's panic capture still gets called
    // via the prev() chain so the panicking thread still produces
    // `Err` on join — and (b) write the panic body to the fallback
    // file even when no `log` subscriber is attached (the case the
    // direct-file write exists for).
    //
    // Kept as a single test so the global `set_hook` only fires
    // once per test process; chained installs across multiple tests
    // would otherwise leak panics from unrelated tests into our
    // fallback file.
    #[test]
    fn install_writes_fallback_and_preserves_harness() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("runner.log");
        install(path.clone());
        let h = std::thread::spawn(|| panic!("intentional-marker-xyz"));
        let res = h.join();
        assert!(res.is_err(), "panic must still propagate to join");
        let contents = std::fs::read_to_string(&path).expect("fallback file written");
        assert!(
            contents.contains("panic:"),
            "fallback file must contain panic header; got: {contents}",
        );
        assert!(
            contents.contains("intentional-marker-xyz"),
            "fallback file must contain panic body; got: {contents}",
        );
    }
}
