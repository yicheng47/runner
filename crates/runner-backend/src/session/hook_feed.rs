use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;

use super::launch::shell_quote;
use crate::error::{Error, Result};
use crate::model::Runtime;

const STATUS_DIR: &str = "session-status";

pub(crate) const fn hooks_supported(runtime: Option<Runtime>, windows: bool) -> bool {
    matches!(
        (runtime, windows),
        (
            Some(Runtime::ClaudeCode | Runtime::Codex | Runtime::Copilot | Runtime::Pi),
            false | true
        ) | (Some(Runtime::Antigravity), false)
    )
}

/// Renders a status feed path for a hook command or env var. Git Bash strips
/// the payload basename at `/` only, and PowerShell accepts either separator.
pub(crate) fn hook_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.into_owned()
    }
}

pub(crate) fn status_path(app_data_dir: &Path, session_id: &str) -> PathBuf {
    app_data_dir
        .join(STATUS_DIR)
        .join(format!("{session_id}.ndjson"))
}

pub(crate) fn clear_leftovers(app_data_dir: &Path) -> Result<()> {
    let entries = match fs::read_dir(app_data_dir.join(STATUS_DIR)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(error) => {
                log::warn!("read stale agent status entry: {error}");
                continue;
            }
        };
        if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("ndjson" | "sh" | "ps1" | "tmp")
        ) || path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains(".ndjson."))
        {
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("remove stale agent status file {}: {error}", path.display());
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn hook_command(path: &Path, event: &str) -> String {
    // A bounded append names the payload so parallel, large hooks cannot interleave JSON.
    format!(
        "(sh {} {} {} || cat >/dev/null) 2>/dev/null; exit 0",
        shell_quote(&hook_path(&script_path(path))),
        shell_quote(&hook_path(path)),
        shell_quote(event),
    )
}

/// The Windows reporter for Codex and Copilot: `feed` and `event` are PowerShell
/// expressions naming the feed and the hook event. It copies stdin as raw bytes, since
/// the console input encoding would corrupt non-ASCII payloads, and contains no double
/// quote, so the JSON layer of Copilot's plugin carries it untouched.
///
/// A .NET `Append` stream is not an atomic append across processes, so each record is
/// written at the end of a feed handle opened under the per-feed mutex
/// `runner-status-<FNV-1a of the feed path>` (a hash keeps the kernel object name
/// short for any path), where the end cannot move. The feed is opened existing-only
/// and every handle shares delete, so teardown can always remove both files and a hook
/// never recreates a feed that is gone.
pub(crate) fn powershell_reporter(feed: &str, generation_env: &str, event: &str) -> String {
    let field = |name: &str, value: &str| format!("$q+'{name}'+$q+':'+$q+{value}+$q");
    format!(
        "$ErrorActionPreference='Stop';$f={feed};$g=$env:{generation_env};$p=$null;\
         try{{$m=New-Object IO.MemoryStream;[Console]::OpenStandardInput().CopyTo($m);\
         if([IO.File]::Exists($f)){{$x=$null;$o=$false;\
         try{{$n=$f+'.'+[Guid]::NewGuid().ToString('N').Substring(0,8);\
         $w=[IO.File]::Open($n,'CreateNew','Write',[IO.FileShare]'Read, Delete');$p=$n;\
         try{{$m.WriteTo($w)}}finally{{$w.Dispose()}};$q=[string][char]34;\
         $b=[Text.Encoding]::UTF8.GetBytes('{{'+{}+','+{}+','+{}+'}}'+[char]10);\
         $h=[uint64]2166136261;foreach($c in $f.Replace([char]92,[char]47).ToCharArray()){{$h=(($h -bxor [int]$c)*16777619)%4294967296}};\
         $x=New-Object Threading.Mutex($false,('runner-status-'+$h.ToString('x8')));\
         try{{$o=$x.WaitOne(1000)}}catch [Threading.AbandonedMutexException]{{$o=$true}};\
         if($o){{$a=[IO.File]::Open($f,'Open','Write',[IO.FileShare]'ReadWrite, Delete');\
         try{{[void]$a.Seek(0,'End');$a.Write($b,0,$b.Length)}}finally{{$a.Dispose()}}}}}}\
         finally{{if($o){{$x.ReleaseMutex()}};if($x){{$x.Dispose()}}}};\
         if($o -and [IO.File]::Exists($f)){{$p=$null}}}}}}catch{{}};\
         if($p){{Remove-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue}}",
        field("generation", "$g"),
        field("hook_event_name", event),
        field("payload_file", "[IO.Path]::GetFileName($p)"),
    )
}

pub(crate) fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) fn script_path(path: &Path) -> PathBuf {
    path.with_extension("sh")
}

pub(crate) fn powershell_script_path(path: &Path) -> PathBuf {
    path.with_extension("ps1")
}

fn temporary_path(script: &Path) -> PathBuf {
    let mut path = script.as_os_str().to_owned();
    path.push(".tmp");
    path.into()
}

struct StatusFiles {
    path: PathBuf,
    owned_script: Option<PathBuf>,
}

impl Drop for StatusFiles {
    fn drop(&mut self) {
        let mut paths = vec![self.path.clone()];
        if let Some(script) = self.owned_script.as_ref() {
            paths.push(script.clone());
            paths.push(temporary_path(script));
        }
        for path in paths {
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("remove agent status file {}: {error}", path.display());
                }
            }
        }
        // Payloads go last: a PowerShell reporter that appended before the feed was
        // removed has already left its payload here, and one after it removes its own.
        if let Some(parent) = self.path.parent() {
            if let Ok(entries) = fs::read_dir(parent) {
                let prefix = format!("{}.", self.path.file_name().unwrap().to_string_lossy());
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with(&prefix) {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
}

pub(super) struct HookFeed {
    path: PathBuf,
    reporter_path: PathBuf,
    reader: BufReader<File>,
    pub(super) pending: Vec<u8>,
    generation: String,
    pub(super) dirty: Arc<AtomicBool>,
    last_read: Instant,
    _watcher: RecommendedWatcher,
    _files: StatusFiles,
}

impl HookFeed {
    pub(super) fn start(path: &Path, generation: String, script_body: &str) -> Result<Self> {
        Self::start_with_script(path, generation, script_path(path), script_body)
    }

    pub(super) fn start_powershell(
        path: &Path,
        generation: String,
        script_body: &str,
    ) -> Result<Self> {
        Self::start_with_script(path, generation, powershell_script_path(path), script_body)
    }

    fn start_with_script(
        path: &Path,
        generation: String,
        script: PathBuf,
        script_body: &str,
    ) -> Result<Self> {
        fs::create_dir_all(path.parent().expect("status file has a parent"))?;
        let files = StatusFiles {
            path: path.to_owned(),
            owned_script: Some(script.clone()),
        };
        let temporary = temporary_path(&script);
        fs::write(&temporary, script_body)?;
        fs::rename(temporary, &script)?;
        Self::start_with_reporter(path, generation, script, files)
    }

    pub(super) fn start_external(
        path: &Path,
        generation: String,
        reporter_path: &Path,
    ) -> Result<Self> {
        fs::create_dir_all(path.parent().expect("status file has a parent"))?;
        let files = StatusFiles {
            path: path.to_owned(),
            owned_script: None,
        };
        Self::start_with_reporter(path, generation, reporter_path.to_owned(), files)
    }

    fn start_with_reporter(
        path: &Path,
        generation: String,
        reporter_path: PathBuf,
        files: StatusFiles,
    ) -> Result<Self> {
        if !reporter_path.exists() {
            return Err(Error::msg(format!(
                "agent status reporter unavailable: {}",
                reporter_path.display()
            )));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        let dirty = Arc::new(AtomicBool::new(true));
        let dirty_for_watch = Arc::clone(&dirty);
        let mut watcher = notify::recommended_watcher(move |event| {
            if let Err(error) = event {
                log::warn!("agent status notify: {error}");
            }
            dirty_for_watch.store(true, Ordering::Release);
        })
        .map_err(|error| Error::msg(format!("agent status watcher: {error}")))?;
        watcher
            .watch(path, RecursiveMode::NonRecursive)
            .map_err(|error| {
                Error::msg(format!("watch agent status {}: {error}", path.display()))
            })?;
        Ok(Self {
            path: path.to_owned(),
            reporter_path,
            reader: BufReader::new(file),
            pending: Vec::new(),
            generation,
            dirty,
            last_read: Instant::now(),
            _watcher: watcher,
            _files: files,
        })
    }

    pub(super) fn drain(&mut self, force: bool, mut observe: impl FnMut(Value)) -> Result<()> {
        if !self.dirty.swap(false, Ordering::AcqRel)
            && self.last_read.elapsed() < Duration::from_secs(1)
            && !force
        {
            return Ok(());
        }
        if !self.path.exists() || !self.reporter_path.exists() {
            return Err(Error::msg("agent status bridge unavailable"));
        }
        self.last_read = Instant::now();
        while self.reader.read_until(b'\n', &mut self.pending)? != 0 {
            if self.pending.last() != Some(&b'\n') {
                break;
            }
            if let Some(report) = self.read_report() {
                observe(report);
            }
            self.pending.clear();
        }
        Ok(())
    }

    fn read_report(&self) -> Option<Value> {
        let mut report: Value = serde_json::from_slice(&self.pending).ok()?;
        if report.get("generation")?.as_str()? != self.generation {
            return None;
        }
        if let Some(file) = report.get("payload_file").and_then(Value::as_str) {
            let prefix = format!("{}.", self.path.file_name()?.to_string_lossy());
            if !file.starts_with(&prefix) || file.contains(['/', '\\']) {
                return None;
            }
            let event = report.get("hook_event_name")?.as_str()?.to_owned();
            let path = self.path.with_file_name(file);
            let payload = fs::read(&path);
            let _ = fs::remove_file(path);
            report = serde_json::from_slice(&payload.ok()?).ok()?;
            let object = report.as_object_mut()?;
            let reported_event = object
                .entry("hook_event_name")
                .or_insert(Value::String(event.clone()));
            if reported_event.as_str() == Some("") {
                *reported_event = Value::String(event.clone());
            }
            if reported_event.as_str() != Some(&event) {
                return None;
            }
        }
        Some(report)
    }
}

pub(super) struct TranscriptTail {
    pub(super) path: PathBuf,
    pub(super) reader: BufReader<File>,
    pub(super) pending: Vec<u8>,
}

impl TranscriptTail {
    pub(super) fn open(path: &Path) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let start = file.metadata()?.len().saturating_sub(1024 * 1024);
        file.seek(SeekFrom::Start(start))?;
        let mut reader = BufReader::new(file);
        if start > 0 {
            reader.read_until(b'\n', &mut Vec::new())?;
        }
        Ok(Self {
            path: path.to_owned(),
            reader,
            pending: Vec::new(),
        })
    }
}

#[cfg(all(test, windows))]
pub(crate) const POWERSHELLS: [&str; 2] = ["pwsh", "powershell"];

/// Runs a hook command the way Codex and Copilot do on Windows, or returns `None`
/// with a printed reason when the shell is not installed.
#[cfg(all(test, windows))]
pub(crate) fn run_powershell(
    shell: &str,
    command: &str,
    env: &[(&str, &str)],
    payload: &[u8],
) -> Option<std::process::Output> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let started = Instant::now();
    let mut child = match Command::new(shell)
        .args(["-NoProfile", "-Command", command])
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!("skipping {shell}: {error}");
            return None;
        }
    };
    child.stdin.take().unwrap().write_all(payload).unwrap();
    let output = child.wait_with_output().unwrap();
    eprintln!("{shell} hook took {:?}", started.elapsed());
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_are_gated_per_runtime() {
        for windows in [false, true] {
            for runtime in Runtime::ALL {
                assert_eq!(
                    hooks_supported(Some(runtime), windows),
                    matches!(
                        runtime,
                        Runtime::ClaudeCode | Runtime::Codex | Runtime::Copilot | Runtime::Pi
                    ) || (runtime == Runtime::Antigravity && !windows),
                    "{runtime:?} windows={windows}"
                );
            }
            assert!(!hooks_supported(None, windows));
        }
    }

    #[test]
    fn hook_paths_keep_the_platform_separator_rules() {
        assert_eq!(
            hook_path(Path::new("/Users/jason/app data/it's.ndjson")),
            "/Users/jason/app data/it's.ndjson"
        );
        assert_eq!(
            powershell_quote("C:/Jason's ''dir"),
            "'C:/Jason''s ''''dir'"
        );
        assert!(!powershell_reporter("$env:FEED", "GENERATION", "'Stop'").contains('"'));
    }

    #[cfg(windows)]
    #[test]
    fn windows_hook_paths_use_forward_slashes() {
        let path = Path::new(r"C:\Users\Jason Wang\it's app data\session-status\s.ndjson");
        assert_eq!(
            hook_path(path),
            "C:/Users/Jason Wang/it's app data/session-status/s.ndjson"
        );
        assert_eq!(
            hook_command(path, "Stop"),
            r"(sh 'C:/Users/Jason Wang/it'\''s app data/session-status/s.sh' 'C:/Users/Jason Wang/it'\''s app data/session-status/s.ndjson' 'Stop' || cat >/dev/null) 2>/dev/null; exit 0"
        );
    }

    #[test]
    fn removing_the_sh_or_ps1_reporter_reports_bridge_loss() {
        let root = tempfile::tempdir().unwrap();
        let scripted = status_path(root.path(), "scripted");
        let mut feed = HookFeed::start(&scripted, "current".into(), "#!/bin/sh\n").unwrap();
        feed.drain(true, |_| panic!("empty feed")).unwrap();
        fs::remove_file(script_path(&scripted)).unwrap();
        assert!(feed.drain(true, |_| {}).is_err());
        drop(feed);

        let powershell = status_path(root.path(), "powershell");
        let mut feed = HookFeed::start_powershell(&powershell, "current".into(), "exit 0").unwrap();
        feed.drain(true, |_| panic!("empty feed")).unwrap();
        fs::remove_file(powershell_script_path(&powershell)).unwrap();
        assert!(feed.drain(true, |_| {}).is_err());
        drop(feed);
        assert_eq!(
            fs::read_dir(root.path().join(STATUS_DIR)).unwrap().count(),
            0
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_reporter_keeps_raw_bytes_and_wakes_the_file_watch() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(&root.path().join("Jason's app data"), "session");
        // Sessions receive the feed through a forward-slash env value.
        let path = PathBuf::from(hook_path(&path));
        let command = |path: &Path| {
            format!(
                "{};exit 0",
                powershell_reporter(
                    &powershell_quote(&hook_path(path)),
                    "RUNNER_TEST_STATUS_GENERATION",
                    "'UserPromptSubmit'",
                )
            )
        };
        let mut payload = serde_json::json!({
            "session_id": "main",
            "prompt": format!("你好，世界 ✓ {}", "x\n".repeat(100 * 1024)),
        });
        let pretty = serde_json::to_vec_pretty(&payload).unwrap();
        assert!(pretty.len() > 200 * 1024);
        for shell in POWERSHELLS {
            let mut feed = HookFeed::start_powershell(&path, "current".into(), "").unwrap();
            feed.drain(true, |_| panic!("empty feed")).unwrap();
            let Some(output) = run_powershell(
                shell,
                &command(&path),
                &[("RUNNER_TEST_STATUS_GENERATION", "current")],
                &pretty,
            ) else {
                continue;
            };
            assert!(output.status.success(), "{shell}: {output:?}");
            assert!(output.stdout.is_empty(), "{shell}: {output:?}");
            assert!(output.stderr.is_empty(), "{shell}: {output:?}");
            let deadline = Instant::now() + Duration::from_millis(900);
            while !feed.dirty.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                feed.dirty.load(Ordering::Acquire),
                "{shell}: notify missed the append"
            );
            let mut reports = Vec::new();
            feed.drain(false, |report| reports.push(report)).unwrap();
            payload["hook_event_name"] = "UserPromptSubmit".into();
            assert_eq!(reports, [payload.clone()], "{shell}");
            payload.as_object_mut().unwrap().remove("hook_event_name");

            let missing = root.path().join("missing dir").join("session.ndjson");
            let output = run_powershell(
                shell,
                &command(&missing),
                &[("RUNNER_TEST_STATUS_GENERATION", "current")],
                &pretty,
            )
            .unwrap();
            assert!(output.status.success(), "{shell}: {output:?}");
            assert!(output.stdout.is_empty() && output.stderr.is_empty());
            assert!(!missing.parent().unwrap().exists());
            drop(feed);
            assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
        }
    }

    /// Holds the reporter's per-feed mutex, the way a concurrent hook would.
    #[cfg(windows)]
    struct HeldFeedLock(windows_sys::Win32::Foundation::HANDLE);

    #[cfg(windows)]
    impl HeldFeedLock {
        fn acquire(path: &Path) -> Self {
            let hash = hook_path(path)
                .encode_utf16()
                .fold(2_166_136_261_u32, |hash, unit| {
                    (hash ^ u32::from(unit)).wrapping_mul(16_777_619)
                });
            let name = format!("runner-status-{hash:08x}")
                .encode_utf16()
                .chain([0])
                .collect::<Vec<_>>();
            let handle = unsafe {
                windows_sys::Win32::System::Threading::CreateMutexW(
                    std::ptr::null(),
                    1,
                    name.as_ptr(),
                )
            };
            assert!(!handle.is_null());
            Self(handle)
        }
    }

    #[cfg(windows)]
    impl Drop for HeldFeedLock {
        fn drop(&mut self) {
            unsafe {
                windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }

    #[cfg(windows)]
    fn start_powershell_hook(shell: &str, path: &Path) -> std::process::Child {
        use std::process::{Command, Stdio};
        let command = format!(
            "{};exit 0",
            powershell_reporter(
                &powershell_quote(&hook_path(path)),
                "RUNNER_TEST_STATUS_GENERATION",
                "'Stop'",
            )
        );
        Command::new(shell)
            .args(["-NoProfile", "-Command", &command])
            .env("RUNNER_TEST_STATUS_GENERATION", "current")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[cfg(windows)]
    fn spawn_powershell_hook(shell: &str, path: &Path, id: usize) -> std::process::Child {
        use std::io::Write;
        let mut child = start_powershell_hook(shell, path);
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{{\"id\":{id}}}").as_bytes())
            .unwrap();
        child
    }

    #[cfg(windows)]
    fn payload_count(path: &Path) -> usize {
        let prefix = format!("{}.", path.file_name().unwrap().to_string_lossy());
        fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&prefix)
            })
            .count()
    }

    #[cfg(windows)]
    fn wait_for_payloads(path: &Path, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while payload_count(path) < count {
            assert!(
                Instant::now() < deadline,
                "hooks never wrote their payloads"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(windows)]
    fn assert_quiet_success(child: std::process::Child) {
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "{output:?}"
        );
    }

    #[cfg(windows)]
    fn drained_ids(feed: &mut HookFeed) -> Vec<usize> {
        let mut ids = Vec::new();
        feed.drain(true, |report| {
            assert_eq!(report["hook_event_name"], "Stop");
            ids.push(report["id"].as_u64().unwrap() as usize);
        })
        .unwrap();
        ids.sort_unstable();
        ids
    }

    #[cfg(windows)]
    #[test]
    fn powershell_reporter_waits_for_the_feed_mutex() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(&root.path().join("Jason's app data"), "held");
        for shell in POWERSHELLS {
            if run_powershell(shell, "exit 0", &[], b"").is_none() {
                continue;
            }
            let mut feed = HookFeed::start_powershell(&path, "current".into(), "").unwrap();

            let lock = HeldFeedLock::acquire(&path);
            let blocked = spawn_powershell_hook(shell, &path, 0);
            wait_for_payloads(&path, 1);
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(
                fs::metadata(&path).unwrap().len(),
                0,
                "{shell} appended while the lock was held"
            );
            drop(lock);
            assert_quiet_success(blocked);

            assert_eq!(drained_ids(&mut feed), vec![0], "{shell}");
            drop(feed);
            assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
        }
    }

    /// The reporter waits one second for the mutex and then gives its report up, so a hook
    /// can never stall an agent. Nothing may be appended and no payload may be left behind.
    #[cfg(windows)]
    #[test]
    fn powershell_reporter_drops_its_report_when_the_feed_mutex_stays_held() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(&root.path().join("Jason's app data"), "dropped");
        for shell in POWERSHELLS {
            if run_powershell(shell, "exit 0", &[], b"").is_none() {
                continue;
            }
            let mut feed = HookFeed::start_powershell(&path, "current".into(), "").unwrap();

            let lock = HeldFeedLock::acquire(&path);
            let starved = spawn_powershell_hook(shell, &path, 7);
            wait_for_payloads(&path, 1);
            assert_quiet_success(starved);
            assert_eq!(
                fs::metadata(&path).unwrap().len(),
                0,
                "{shell} appended without the lock"
            );
            assert_eq!(payload_count(&path), 0, "{shell} left its payload behind");
            drop(lock);

            assert!(drained_ids(&mut feed).is_empty(), "{shell}");
            drop(feed);
            assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
        }
    }

    /// A reporter that gives up (timeout or swallowed error) removes its payload;
    /// an overwritten append leaves an orphan.
    /// Every surviving payload must have exactly one intact record, even under contention.
    #[cfg(windows)]
    #[test]
    fn powershell_reporter_loses_no_record_when_appends_overlap() {
        use std::io::Write;
        const WAVE: usize = 6;
        const WAVES: usize = 6;
        let root = tempfile::tempdir().unwrap();
        let path = status_path(&root.path().join("Jason's app data"), "concurrent");
        for shell in POWERSHELLS {
            if run_powershell(shell, "exit 0", &[], b"").is_none() {
                continue;
            }
            let mut feed = HookFeed::start_powershell(&path, "current".into(), "").unwrap();
            let mut overlapping_appends = false;

            // Exceed the pipe buffer so every reporter reaches CopyTo and waits for EOF
            // before any append starts. Closing all stdins releases the wave together.
            for wave in 0..WAVES {
                let before = payload_count(&path);
                let mut hooks = (0..WAVE)
                    .map(|_| start_powershell_hook(shell, &path))
                    .collect::<Vec<_>>();
                let stdins = hooks
                    .iter_mut()
                    .enumerate()
                    .map(|(index, hook)| {
                        let mut stdin = hook.stdin.take().unwrap();
                        let id = wave * WAVE + index;
                        let payload =
                            serde_json::json!({"id": id, "padding": "x".repeat(1024 * 1024)});
                        stdin.write_all(payload.to_string().as_bytes()).unwrap();
                        stdin
                    })
                    .collect::<Vec<_>>();
                assert!(hooks
                    .iter_mut()
                    .all(|hook| hook.try_wait().unwrap().is_none()));
                assert_eq!(payload_count(&path), before);
                drop(stdins);
                hooks.into_iter().for_each(assert_quiet_success);
                overlapping_appends |= payload_count(&path) >= before + 2;
            }

            assert!(
                overlapping_appends,
                "{shell}: no wave landed overlapping appends"
            );
            let prefix = format!("{}.", path.file_name().unwrap().to_string_lossy());
            let mut expected_ids = fs::read_dir(path.parent().unwrap())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(&prefix)
                })
                .map(|path| {
                    let payload: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                    payload["id"].as_u64().unwrap() as usize
                })
                .collect::<Vec<_>>();
            expected_ids.sort_unstable();
            assert!(expected_ids.iter().all(|id| *id < WAVES * WAVE));
            assert!(expected_ids.windows(2).all(|ids| ids[0] != ids[1]));
            let records = fs::read_to_string(&path).unwrap();
            assert!(records.ends_with('\n'), "{shell}: torn final record");
            assert_eq!(records.lines().count(), expected_ids.len(), "{shell}");
            for line in records.lines() {
                serde_json::from_str::<Value>(line).expect("torn feed record");
            }
            assert_eq!(drained_ids(&mut feed), expected_ids, "{shell}");
            assert_eq!(payload_count(&path), 0, "{shell}: orphaned payload");
            drop(feed);
            assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
        }
    }

    #[test]
    fn startup_clears_powershell_reporters_left_by_a_crash() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "stale");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(powershell_script_path(&path), "exit 0").unwrap();
        fs::write(temporary_path(&powershell_script_path(&path)), "exit 0").unwrap();
        clear_leftovers(root.path()).unwrap();
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
    }

    #[test]
    fn failed_powershell_setup_removes_its_reporter() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "bad-file");
        fs::create_dir_all(&path).unwrap();
        assert!(HookFeed::start_powershell(&path, "current".into(), "exit 0").is_err());
        assert!(!powershell_script_path(&path).exists());
        assert!(!temporary_path(&powershell_script_path(&path)).exists());
    }

    #[cfg(windows)]
    #[test]
    fn powershell_reporter_in_flight_at_teardown_leaves_nothing_behind() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "teardown");
        for shell in POWERSHELLS {
            if run_powershell(shell, "exit 0", &[], b"").is_none() {
                continue;
            }
            let feed = HookFeed::start_powershell(&path, "current".into(), "").unwrap();
            let lock = HeldFeedLock::acquire(&path);
            let hook = spawn_powershell_hook(shell, &path, 1);
            wait_for_payloads(&path, 1);
            drop(feed);
            assert_eq!(
                payload_count(&path),
                0,
                "{shell}: teardown kept the payload"
            );
            drop(lock);
            assert_quiet_success(hook);
            assert!(!path.exists(), "{shell} recreated the feed");
            assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
        }
    }
}
