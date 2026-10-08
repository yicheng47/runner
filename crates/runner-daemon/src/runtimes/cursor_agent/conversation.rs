use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use md5::{Digest, Md5};

use super::*;

pub(super) fn config_dir() -> Option<PathBuf> {
    let env = HashMap::new();
    config_dir_with(&env)
}

fn config_dir_with(env: &HashMap<String, String>) -> Option<PathBuf> {
    #[cfg(test)]
    use crate::golden::config_var_os as var_os;
    #[cfg(not(test))]
    use std::env::var_os;
    let value = |key: &str| {
        env.get(key)
            .map(std::ffi::OsString::from)
            .or_else(|| var_os(key))
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    value("CURSOR_CONFIG_DIR")
        .or_else(|| value("XDG_CONFIG_HOME").map(|path| path.join("cursor")))
        .or_else(|| {
            #[cfg(windows)]
            let name = "USERPROFILE";
            #[cfg(not(windows))]
            let name = "HOME";
            env.get(name)
                .filter(|value| !value.is_empty())
                .map(|value| PathBuf::from(value).join(".cursor"))
        })
        .or_else(|| config_home(None, ".cursor"))
}

fn chats_dir(cwd: Option<&str>, env: &HashMap<String, String>) -> Option<PathBuf> {
    let cwd = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    let cwd = if cwd.is_absolute() {
        cwd
    } else {
        std::env::current_dir().ok()?.join(cwd)
    };
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    let bucket = format!("{:x}", Md5::digest(cwd.to_string_lossy().as_bytes()));
    let config = config_dir_with(env)?;
    let config = if config.is_absolute() {
        config
    } else {
        cwd.join(config)
    };
    Some(config.join("chats").join(bucket))
}

#[cfg(any(not(windows), test))]
pub(super) fn store_path(
    cwd: Option<&str>,
    env: &HashMap<String, String>,
    key: &str,
) -> Option<PathBuf> {
    if !is_uuid(key) {
        return None;
    }
    Some(chats_dir(cwd, env)?.join(key).join("store.db"))
}

// Cursor closes the old store on /clear, /resume and /fork. Observe the
// launched process's open store, rather than guessing from the newest chat.
pub(super) struct Watcher {
    root: PathBuf,
    pid: Option<u32>,
    last_poll: Option<Instant>,
    key: Option<String>,
    empty_polls: u8,
}

impl Watcher {
    pub(super) fn new(spec: &SpawnSpec) -> Option<Self> {
        Some(Self {
            root: chats_dir(
                spec.cwd.as_deref().and_then(Path::to_str),
                &spec
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )?,
            pid: None,
            last_poll: None,
            key: None,
            empty_polls: 0,
        })
    }

    fn observe(&mut self, paths: &[PathBuf]) -> Option<String> {
        let mut keys: Vec<_> = paths
            .iter()
            .filter_map(|path| store_key(&self.root, path))
            .collect();
        keys.sort();
        keys.dedup();
        let next = match keys.len() {
            0 => {
                self.empty_polls = self.empty_polls.saturating_add(1);
                if self.empty_polls < 3 {
                    return None;
                }
                None
            }
            1 => {
                self.empty_polls = 0;
                keys.pop()
            }
            _ => {
                self.empty_polls = 0;
                return None;
            }
        };
        if next == self.key {
            return None;
        }
        self.key = next.clone();
        Some(next.unwrap_or_default())
    }
}

fn store_key(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = relative.components();
    let id = parts.next()?.as_os_str().to_str()?;
    if !is_uuid(id) || parts.next()?.as_os_str() != "store.db" || parts.next().is_some() {
        return None;
    }
    Some(id.to_owned())
}

impl HookWatcher for Watcher {
    fn spawned(&mut self, pid: u32) {
        log::debug!(
            "Cursor conversation watcher: pid={pid} root={}",
            self.root.display()
        );
        self.pid = Some(pid);
    }
    fn drain_events(
        &mut self,
        _cancel: u8,
        _emit: &mut dyn FnMut(
            crate::session::state::agent::AgentEvent,
        ) -> crate::session::state::agent::AdapterFeedback,
        session_start: &mut dyn FnMut(String),
    ) -> crate::error::Result<()> {
        if self
            .last_poll
            .is_some_and(|at| at.elapsed() < Duration::from_millis(500))
        {
            return Ok(());
        }
        let Some(pid) = self.pid else {
            return Ok(());
        };
        self.last_poll = Some(Instant::now());
        let Some(paths) = open_files(pid) else {
            self.empty_polls = 0;
            return Ok(());
        };
        if let Some(key) = self.observe(&paths) {
            log::debug!("Cursor conversation changed: pid={pid} key={key}");
            session_start(key);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn open_files(pid: u32) -> Option<Vec<PathBuf>> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    // PROC_PIDFDVNODEPATHINFO is absent from libc's exported constants.
    const VNODE_PATH_INFO: libc::c_int = 2;
    #[repr(C)]
    struct FileInfo {
        open_flags: u32,
        status: u32,
        offset: i64,
        kind: i32,
        guard_flags: u32,
    }
    #[repr(C)]
    struct VnodeFdInfo {
        file: FileInfo,
        path: libc::vnode_info_path,
    }
    let pid: libc::pid_t = pid.try_into().ok()?;
    let entry_size = std::mem::size_of::<libc::proc_fdinfo>();
    let size =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if size <= 0 {
        return None;
    }
    let count = size as usize / entry_size + 32;
    let mut entries: Vec<libc::proc_fdinfo> =
        (0..count).map(|_| unsafe { std::mem::zeroed() }).collect();
    let capacity: libc::c_int = (entries.len() * entry_size).try_into().ok()?;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            entries.as_mut_ptr().cast(),
            capacity,
        )
    };
    if read <= 0 || read == capacity || !(read as usize).is_multiple_of(entry_size) {
        return None;
    }
    entries.truncate(read as usize / entry_size);
    let mut paths = Vec::new();
    for entry in entries {
        if entry.proc_fdtype != libc::PROX_FDTYPE_VNODE as u32 {
            continue;
        }
        let mut info: VnodeFdInfo = unsafe { std::mem::zeroed() };
        let size: libc::c_int = std::mem::size_of_val(&info).try_into().ok()?;
        let read = unsafe {
            libc::proc_pidfdinfo(
                pid,
                entry.proc_fd,
                VNODE_PATH_INFO,
                (&mut info as *mut VnodeFdInfo).cast(),
                size,
            )
        };
        if read != size {
            return None;
        }
        let bytes: Vec<_> = info
            .path
            .vip_path
            .iter()
            .flatten()
            .take_while(|byte| **byte != 0)
            .map(|byte| *byte as u8)
            .collect();
        if !bytes.is_empty() {
            paths.push(PathBuf::from(OsString::from_vec(bytes)));
        }
    }
    Some(paths)
}

#[cfg(target_os = "linux")]
fn open_files(pid: u32) -> Option<Vec<PathBuf>> {
    Some(
        std::fs::read_dir(format!("/proc/{pid}/fd"))
            .ok()?
            .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
            .collect(),
    )
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn open_files(_pid: u32) -> Option<Vec<PathBuf>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_store_rekeys_preserves_ambiguity_and_debounces_empty_scans() {
        let root = PathBuf::from("/owned/chats/workspace");
        let first = "578dbdf3-2776-4aa0-ab95-ec4a2c3f58e4";
        let second = "1f5c822a-5704-4aab-a4c2-fae5757479c1";
        let store = |id: &str| root.join(id).join("store.db");
        let mut watcher = Watcher {
            root: root.clone(),
            pid: None,
            last_poll: None,
            key: None,
            empty_polls: 0,
        };
        assert_eq!(watcher.observe(&[store(first)]), Some(first.into()));
        assert_eq!(watcher.observe(&[store(first), store(first)]), None);
        assert_eq!(watcher.observe(&[]), None);
        assert_eq!(watcher.observe(&[]), None);
        assert_eq!(watcher.key.as_deref(), Some(first));
        assert_eq!(watcher.observe(&[store(first), store(second)]), None);
        assert_eq!(watcher.key.as_deref(), Some(first));
        assert_eq!(watcher.observe(&[]), None);
        assert_eq!(watcher.observe(&[]), None);
        assert_eq!(watcher.observe(&[]), Some(String::new()));
        assert_eq!(watcher.observe(&[store(second)]), Some(second.into()));
        assert_eq!(
            watcher.observe(&[
                PathBuf::from("/other/chats").join(first).join("store.db"),
                root.join("../escape/store.db")
            ]),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn store_path_uses_exact_cwd_bucket_and_role_config_override() {
        let env = HashMap::from([("CURSOR_CONFIG_DIR".into(), "/private/config".into())]);
        let key = "578dbdf3-2776-4aa0-ab95-ec4a2c3f58e4";
        let path = store_path(Some("/workspace"), &env, key).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/private/config/chats/eab0d61a99b6696edb3d2aff87b585e8")
                .join(key)
                .join("store.db")
        );
        assert!(store_path(Some("/workspace"), &env, "../../other").is_none());
    }
    #[test]
    fn inherited_config_uses_the_environment_seam_and_preserves_spaces() {
        crate::golden::with_config_env(
            std::collections::BTreeMap::from([
                ("CURSOR_CONFIG_DIR", " /inherited/cursor ".into()),
                ("XDG_CONFIG_HOME", "/inherited/xdg".into()),
            ]),
            || {
                assert_eq!(config_dir(), Some(PathBuf::from(" /inherited/cursor ")));
                let env = HashMap::from([("CURSOR_CONFIG_DIR".into(), String::new())]);
                assert_eq!(
                    config_dir_with(&env),
                    Some(PathBuf::from("/inherited/xdg/cursor"))
                );
            },
        );
    }

    #[test]
    fn role_home_controls_default_history_and_explicit_config_wins() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let profile = directory.path().join("profile");
        #[cfg(windows)]
        let home_var = "USERPROFILE";
        #[cfg(not(windows))]
        let home_var = "HOME";
        let mut env = HashMap::from([
            (home_var.into(), profile.display().to_string()),
            ("CURSOR_CONFIG_DIR".into(), String::new()),
            ("XDG_CONFIG_HOME".into(), String::new()),
        ]);
        let key = "578dbdf3-2776-4aa0-ab95-ec4a2c3f58e4";
        let path = store_path(directory.path().to_str(), &env, key).unwrap();
        assert!(path.starts_with(profile.join(".cursor")));
        env.insert(
            "XDG_CONFIG_HOME".into(),
            directory.path().join("xdg").display().to_string(),
        );
        assert!(store_path(directory.path().to_str(), &env, key)
            .unwrap()
            .starts_with(directory.path().join("xdg/cursor")));
        env.insert(
            "CURSOR_CONFIG_DIR".into(),
            directory.path().join("explicit").display().to_string(),
        );
        assert!(store_path(directory.path().to_str(), &env, key)
            .unwrap()
            .starts_with(directory.path().join("explicit")));
    }

    #[test]
    fn failed_scan_keeps_identity_and_breaks_the_empty_scan_streak() {
        let key = "578dbdf3-2776-4aa0-ab95-ec4a2c3f58e4";
        let root = PathBuf::from("/owned/chats/workspace");
        let mut watcher = Watcher {
            root,
            pid: Some(u32::MAX),
            last_poll: None,
            key: Some(key.into()),
            empty_polls: 2,
        };
        let mut updates = Vec::new();
        watcher
            .drain_events(0, &mut |_| Default::default(), &mut |key| updates.push(key))
            .unwrap();
        assert!(updates.is_empty());
        assert_eq!(watcher.key.as_deref(), Some(key));
        assert_eq!(watcher.empty_polls, 0);
        assert_eq!(watcher.observe(&[]), None);
        assert_eq!(watcher.observe(&[]), None);
        assert_eq!(watcher.observe(&[]), Some(String::new()));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn native_fd_paths_preserve_unicode_and_literal_escape_sequences() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let folder = directory.path().join(r"中文 profile \x41");
        std::fs::create_dir_all(&folder).unwrap();
        let store = folder.join("store.db");
        std::fs::write(&store, []).unwrap();
        let opened = std::fs::File::open(&store).unwrap();
        let store = store.canonicalize().unwrap();
        let found = (0..20).any(|_| {
            let found = open_files(std::process::id()).is_some_and(|paths| paths.contains(&store));
            if !found {
                std::thread::sleep(Duration::from_millis(10));
            }
            found
        });
        drop(opened);
        assert!(found, "native FD API did not return the original path");
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_fd_scan_keeps_key_when_child_closes_store_before_exit() {
        use std::io::{BufRead, Write};
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let config = directory.path().join("中文 config");
        let key = "578dbdf3-2776-4aa0-ab95-ec4a2c3f58e4";
        let env = HashMap::from([("CURSOR_CONFIG_DIR".into(), config.display().to_string())]);
        let store = store_path(directory.path().to_str(), &env, key).unwrap();
        std::fs::create_dir_all(store.parent().unwrap()).unwrap();
        std::fs::write(&store, []).unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("exec 9<\"$CURSOR_TEST_STORE\"; printf 'READY\\n'; read marker; exec 9<&-; printf 'CLOSED\\n'; read marker")
            .env("CURSOR_TEST_STORE", &store)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = String::new();
        let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        stdout.read_line(&mut ready).unwrap();
        assert_eq!(ready.trim(), "READY");
        let spec = SpawnSpec {
            cwd: Some(directory.path().to_path_buf()),
            env: env.into_iter().collect(),
            ..Default::default()
        };
        let mut watcher = Watcher::new(&spec).unwrap();
        watcher.spawned(child.id());
        let mut updates = Vec::new();
        watcher
            .drain_events(0, &mut |_| Default::default(), &mut |key| updates.push(key))
            .unwrap();
        child.stdin.as_mut().unwrap().write_all(b"close\n").unwrap();
        ready.clear();
        stdout.read_line(&mut ready).unwrap();
        assert_eq!(ready.trim(), "CLOSED");
        watcher.last_poll = None;
        watcher
            .drain_events(0, &mut |_| Default::default(), &mut |key| updates.push(key))
            .unwrap();
        assert_eq!(watcher.empty_polls, 1);
        assert_eq!(watcher.key.as_deref(), Some(key));
        child.stdin.take().unwrap().write_all(b"exit\n").unwrap();
        child.wait().unwrap();
        watcher.last_poll = None;
        watcher
            .drain_events(0, &mut |_| Default::default(), &mut |key| updates.push(key))
            .unwrap();
        assert_eq!(updates, [key]);
        assert_eq!(watcher.key.as_deref(), Some(key));
    }
}
