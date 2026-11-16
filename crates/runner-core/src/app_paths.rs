use std::ffi::OsString;
use std::fmt;
#[cfg(not(windows))]
use std::path::Component;
use std::path::{Path, PathBuf};

const APP_IDENTIFIER: &str = "com.wycstudios.runner";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcEndpoint(pub PathBuf);

impl fmt::Display for IpcEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::home_dir()
}

pub fn app_identifier(debug: bool) -> String {
    if debug {
        format!("{APP_IDENTIFIER}-dev")
    } else {
        APP_IDENTIFIER.to_string()
    }
}

pub fn app_data_dir(debug: bool) -> Option<PathBuf> {
    Some(app_data_dir_for_home(&home_dir()?, debug))
}

/// On Windows and Linux, `home` is the fallback when APPDATA or XDG_DATA_HOME is unset.
pub fn app_data_dir_for_home(home: &Path, debug: bool) -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = home.join("Library").join("Application Support");
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("share"));
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData").join("Roaming"));
    base.join(app_identifier(debug))
}

pub fn log_dir(debug: bool) -> Option<PathBuf> {
    Some(log_dir_for_home(&home_dir()?, debug))
}

pub fn log_dir_for_home(home: &Path, debug: bool) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Logs")
            .join(app_identifier(debug))
    }
    #[cfg(not(target_os = "macos"))]
    {
        app_data_dir_for_home(home, debug).join("logs")
    }
}

pub fn mcp_endpoint(app_data_dir: &Path, debug: bool) -> IpcEndpoint {
    #[cfg(unix)]
    {
        let _ = debug;
        IpcEndpoint(app_data_dir.join("mcp.sock"))
    }
    #[cfg(windows)]
    {
        let _ = app_data_dir;
        #[cfg(debug_assertions)]
        if let Some(endpoint) = std::env::var_os("RUNNER_TEST_MCP_ENDPOINT") {
            return IpcEndpoint(endpoint.into());
        }
        IpcEndpoint(PathBuf::from(format!(
            r"\\.\pipe\{}",
            app_identifier(debug)
        )))
    }
}

pub fn daemon_endpoint(app_data_dir: &Path, debug: bool) -> IpcEndpoint {
    #[cfg(unix)]
    {
        let _ = debug;
        IpcEndpoint(app_data_dir.join("runnerd.sock"))
    }
    #[cfg(windows)]
    {
        let _ = app_data_dir;
        #[cfg(debug_assertions)]
        if let Some(endpoint) = std::env::var_os("RUNNER_TEST_DAEMON_ENDPOINT") {
            return IpcEndpoint(endpoint.into());
        }
        IpcEndpoint(PathBuf::from(if debug {
            r"\\.\pipe\com.wycstudios.runnerd-dev"
        } else {
            r"\\.\pipe\com.wycstudios.runnerd"
        }))
    }
}

/// Canonicalizes an existing checkout (or its nearest existing ancestor when a
/// checkout folder has been removed) and strips Windows's `\\?\` verbatim
/// prefix, resolving 8.3 short names and unifying separators so paths from
/// `git`, `std::fs::canonicalize`, and session/settings strings share one
/// identity on every platform.
pub fn normalize_checkout_path(path: &Path) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        return clean_canonical_path(canon);
    }
    let mut missing_tail: Vec<OsString> = Vec::new();
    let mut cursor = path;
    while let Some(parent) = cursor.parent() {
        if parent.as_os_str().is_empty() || parent.parent().is_none() {
            break;
        }
        let Some(name) = cursor.file_name() else {
            break;
        };
        missing_tail.push(name.to_os_string());
        if let Ok(canon_parent) = parent.canonicalize() {
            let mut out = clean_canonical_path(canon_parent);
            for part in missing_tail.into_iter().rev() {
                out.push(part);
            }
            return out;
        }
        cursor = parent;
    }
    clean_lexical_path(path)
}

pub fn same_checkout_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    same_normalized_checkout_path(&normalize_checkout_path(a), &normalize_checkout_path(b))
}

pub fn same_normalized_checkout_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        match (a.to_str(), b.to_str()) {
            (Some(sa), Some(sb)) => sa.eq_ignore_ascii_case(sb),
            _ => a == b,
        }
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

pub fn normalized_checkout_starts_with(path: &Path, base: &Path) -> bool {
    #[cfg(windows)]
    {
        let mut path_components = path.components();
        for base_component in base.components() {
            let Some(path_component) = path_components.next() else {
                return false;
            };
            let p = path_component.as_os_str().to_string_lossy();
            let b = base_component.as_os_str().to_string_lossy();
            if !p.eq_ignore_ascii_case(&b) {
                return false;
            }
        }
        true
    }
    #[cfg(not(windows))]
    {
        path.starts_with(base)
    }
}

#[cfg(windows)]
fn clean_canonical_path(path: PathBuf) -> PathBuf {
    match path.to_str() {
        Some(raw) => PathBuf::from(normalize_windows_lexical(raw)),
        None => path,
    }
}

#[cfg(not(windows))]
fn clean_canonical_path(path: PathBuf) -> PathBuf {
    path
}

#[cfg(windows)]
fn clean_lexical_path(path: &Path) -> PathBuf {
    match path.to_str() {
        Some(raw) => PathBuf::from(normalize_windows_lexical(raw)),
        None => path.components().collect(),
    }
}

#[cfg(not(windows))]
fn clean_lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !out.has_root() {
                    out.push(Component::ParentDir);
                }
            }
            other => out.push(other),
        }
    }
    if out.as_os_str().is_empty() && !path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// Checks whether `path` currently resolves to an openable directory.
///
/// On Windows, `std::fs::metadata` (`Path::is_dir`) falls back to directory
/// enumeration on `ERROR_ACCESS_DENIED` and still reports `true` while a removed
/// directory remains `STATUS_DELETE_PENDING` behind an open handle. Checking
/// an open handle and distinguishing `STATUS_DELETE_PENDING` keeps deleted dirs
/// as `Ok(false)` while preserving real permission/access failures as `Err`.
pub fn check_existing_dir(path: &Path) -> std::io::Result<bool> {
    #[cfg(not(windows))]
    {
        match std::fs::metadata(path) {
            Ok(metadata) => Ok(metadata.is_dir()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_DELETE_PENDING};
        use windows_sys::Win32::Storage::FileSystem::{
            FileStandardInfo, GetFileInformationByHandleEx, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO,
        };

        let file = match std::fs::OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
        {
            Ok(file) => file,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(false);
            }
            Err(error) if error.raw_os_error() == Some(ERROR_DELETE_PENDING as i32) => {
                return Ok(false);
            }
            // NT maps STATUS_DELETE_PENDING to Win32 ERROR_ACCESS_DENIED; only
            // treat that error as missing when NTSTATUS proves DeletePending.
            Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
                if is_windows_delete_pending(path) {
                    return Ok(false);
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let mut info = FILE_STANDARD_INFO::default();
        // `file` is an open handle from `OpenOptions::open` above, and `info`
        // points to valid stack storage sized for `FILE_STANDARD_INFO`.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileStandardInfo,
                (&mut info as *mut FILE_STANDARD_INFO).cast(),
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )
        };
        if ok == 0 {
            return file.metadata().map(|metadata| metadata.is_dir());
        }
        Ok(info.Directory && !info.DeletePending)
    }
}

#[cfg(windows)]
fn windows_probe_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let wide: Vec<u16> = abs
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if wide.len() <= 248 {
        return wide;
    }
    if let Some(raw) = abs.to_str()
        && !raw.starts_with(r"\\")
        && raw.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && raw.get(1..3) == Some(r":\")
    {
        return std::ffi::OsStr::new(&format!(r"\\?\{raw}"))
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
    }
    wide
}

#[cfg(windows)]
fn is_windows_delete_pending(path: &Path) -> bool {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_ACCESS_DENIED, ERROR_DELETE_PENDING, INVALID_HANDLE_VALUE, NTSTATUS,
        STATUS_DELETE_PENDING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FileStandardInfo, GetFileInformationByHandleEx, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, OPEN_EXISTING,
    };

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetLastNtStatus() -> NTSTATUS;
    }

    let wide = windows_probe_wide(path);
    // `wide` is NUL-terminated for the lifetime of the call and `null()` security
    // attributes / template handle are documented defaults for `CreateFileW`.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        // Read `RtlGetLastNtStatus` and `GetLastError` immediately before any
        // allocation or cleanup can alter TEB status slots.
        let nt_status = unsafe { RtlGetLastNtStatus() };
        let win_err = unsafe { GetLastError() };
        return (win_err == ERROR_ACCESS_DENIED && nt_status == STATUS_DELETE_PENDING)
            || win_err == ERROR_DELETE_PENDING;
    }
    // `handle` is valid and non-`INVALID_HANDLE_VALUE`.
    let owned = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut info = FILE_STANDARD_INFO::default();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            owned.as_raw_handle(),
            FileStandardInfo,
            (&mut info as *mut FILE_STANDARD_INFO).cast(),
            std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    ok != 0 && info.DeletePending
}

/// Returns `true` if `path` exists as a live, openable directory.
pub fn is_existing_dir(path: &Path) -> bool {
    match check_existing_dir(path) {
        Ok(exists) => exists,
        // Keep ACL-protected folders out of the missing-directory branch while
        // still rejecting Windows `STATUS_DELETE_PENDING` via `Ok(false)` above.
        Err(_) => cfg!(windows) && path.is_dir(),
    }
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) fn normalize_windows_lexical(raw: &str) -> String {
    let stripped;
    let s = if let Some(after) = raw.strip_prefix(r"\\?\") {
        if after
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"UNC\"))
        {
            stripped = format!(r"\\{}", &after[4..]);
            stripped.as_str()
        } else if after
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
            && after.as_bytes().get(1) == Some(&b':')
            && matches!(after.as_bytes().get(2), None | Some(b'\\') | Some(b'/'))
        {
            after
        } else {
            return raw.to_owned();
        }
    } else {
        raw
    };

    if let Some(after_slashes) = s.strip_prefix(r"\\").or_else(|| s.strip_prefix("//")) {
        if after_slashes.starts_with(".\\")
            || after_slashes.starts_with("./")
            || after_slashes == "."
        {
            return raw.to_owned();
        }
        let mut parts = after_slashes.split(['/', '\\']).filter(|p| !p.is_empty());
        let Some(server) = parts.next() else {
            return r"\\".to_owned();
        };
        let Some(share) = parts.next() else {
            return format!(r"\\{server}");
        };
        let mut segs: Vec<&str> = Vec::new();
        for part in parts {
            match part {
                "." => {}
                ".." => {
                    segs.pop();
                }
                other => segs.push(other),
            }
        }
        return if segs.is_empty() {
            format!(r"\\{server}\{share}")
        } else {
            format!(r"\\{server}\{share}\{}", segs.join(r"\"))
        };
    }

    if s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && s.as_bytes().get(1) == Some(&b':')
    {
        let drive = char::from(s.as_bytes()[0].to_ascii_uppercase());
        let rest = &s[2..];
        let rooted = rest.starts_with(['/', '\\']);
        let mut segs: Vec<&str> = Vec::new();
        for part in rest.split(['/', '\\']) {
            match part {
                "" | "." => {}
                ".." => {
                    if segs.last().is_some_and(|last| *last != "..") {
                        segs.pop();
                    } else if !rooted {
                        segs.push("..");
                    }
                }
                other => segs.push(other),
            }
        }
        return if rooted {
            if segs.is_empty() {
                format!(r"{drive}:\")
            } else {
                format!(r"{drive}:\{}", segs.join(r"\"))
            }
        } else if segs.is_empty() {
            format!("{drive}:")
        } else {
            format!(r"{drive}:{}", segs.join(r"\"))
        };
    }

    let rooted = s.starts_with(['/', '\\']);
    let mut segs: Vec<&str> = Vec::new();
    for part in s.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if segs.last().is_some_and(|last| *last != "..") {
                    segs.pop();
                } else if !rooted {
                    segs.push("..");
                }
            }
            other => segs.push(other),
        }
    }
    if rooted {
        if segs.is_empty() {
            r"\".to_owned()
        } else {
            format!(r"\{}", segs.join(r"\"))
        }
    } else if segs.is_empty() {
        if raw.is_empty() {
            String::new()
        } else {
            ".".to_owned()
        }
    } else {
        segs.join(r"\")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_endpoint_separates_development_and_production() {
        let home = Path::new("/home/tester");
        let release = mcp_endpoint(&app_data_dir_for_home(home, false), false);
        let debug = mcp_endpoint(&app_data_dir_for_home(home, true), true);
        assert_ne!(release, debug);
        #[cfg(unix)]
        assert!(release.0.ends_with("com.wycstudios.runner/mcp.sock"));
        #[cfg(windows)]
        {
            assert_eq!(release.to_string(), r"\\.\pipe\com.wycstudios.runner");
            assert_eq!(debug.to_string(), r"\\.\pipe\com.wycstudios.runner-dev");
        }
    }

    #[test]
    fn windows_lexical_normalization_strips_verbatim_and_unifies_separators() {
        assert_eq!(
            normalize_windows_lexical(r"\\?\C:\Users\runneradmin\AppData\Local\Temp\.tmpzV6IBL"),
            r"C:\Users\runneradmin\AppData\Local\Temp\.tmpzV6IBL"
        );
        assert_eq!(
            normalize_windows_lexical(
                "C:/Users/runneradmin/AppData/Local/Temp/.tmpzV6IBL/.worktrees/feat-x/"
            ),
            r"C:\Users\runneradmin\AppData\Local\Temp\.tmpzV6IBL\.worktrees\feat-x"
        );
        assert_eq!(
            normalize_windows_lexical(r"c:\Users\runneradmin\repo\.\sub\..\.worktrees/feat-x"),
            r"C:\Users\runneradmin\repo\.worktrees\feat-x"
        );
        assert_eq!(
            normalize_windows_lexical(r"\\?\UNC\server\share\repo/.worktrees/wt/"),
            r"\\server\share\repo\.worktrees\wt"
        );
        assert_eq!(normalize_windows_lexical(r"\\?\c:\"), r"C:\");
        assert_eq!(normalize_windows_lexical("C:/"), r"C:\");
        assert_eq!(
            normalize_windows_lexical(r"\\.\pipe\com.wycstudios.runner"),
            r"\\.\pipe\com.wycstudios.runner"
        );
        // Non-ASCII first directory segments (CJK, accented Latin, emoji) must not slice inside UTF-8 characters.
        assert_eq!(
            normalize_windows_lexical(r"\\?\C:\项目\repo/.worktrees/feat-x/"),
            r"C:\项目\repo\.worktrees\feat-x"
        );
        assert_eq!(
            normalize_windows_lexical(r"\\?\d:\Équipe\🚀-app\sub\..\repo"),
            r"D:\Équipe\🚀-app\repo"
        );
        assert_eq!(
            normalize_windows_lexical(r"\\?\unc\服务器\共享\项目/repo/"),
            r"\\服务器\共享\项目\repo"
        );
    }

    #[test]
    fn normalize_checkout_path_unifies_existing_and_missing_leaf_paths() {
        let temp = tempfile::tempdir().unwrap();
        let wt_parent = temp.path().join("runner").join(".worktrees");
        let live_wt = wt_parent.join("feat-live");
        std::fs::create_dir_all(&live_wt).unwrap();

        let norm_live = normalize_checkout_path(&live_wt);
        assert!(
            !norm_live.to_string_lossy().starts_with(r"\\?\"),
            "verbatim prefix must be stripped: {norm_live:?}"
        );
        let dotted = wt_parent.join(".").join("feat-live");
        assert_eq!(normalize_checkout_path(&dotted), norm_live);
        assert!(same_checkout_path(&live_wt, &dotted));

        let gone_wt = wt_parent.join("feat-gone");
        let gone_alt = temp
            .path()
            .join("runner")
            .join(".")
            .join(".worktrees")
            .join("feat-gone");
        assert_eq!(
            normalize_checkout_path(&gone_wt),
            normalize_checkout_path(&gone_alt)
        );
        assert!(same_checkout_path(&gone_wt, &gone_alt));
        assert!(normalized_checkout_starts_with(
            &normalize_checkout_path(&gone_wt),
            &normalize_checkout_path(&wt_parent),
        ));
    }

    #[test]
    fn existing_dir_rejects_files_and_missing_paths() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        assert!(is_existing_dir(&folder));
        assert!(check_existing_dir(&folder).unwrap());

        let file = temp.path().join("file.txt");
        std::fs::write(&file, "hi").unwrap();
        assert!(!is_existing_dir(&file));
        assert!(!check_existing_dir(&file).unwrap());
        assert!(!is_existing_dir(&file.join("below")));
        assert!(!check_existing_dir(&file.join("below")).unwrap());

        let missing = temp.path().join("missing");
        assert!(!is_existing_dir(&missing));
        assert!(!check_existing_dir(&missing).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn existing_dir_rejects_legacy_delete_pending_directory_while_handle_is_open() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileDispositionInfo, SetFileInformationByHandle, DELETE, FILE_DISPOSITION_INFO,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("watched");
        std::fs::create_dir(&target).unwrap();

        let held = std::fs::OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&target)
            .unwrap();

        // Windows Server 2025 / 24H2 `RemoveDirectoryW` uses POSIX unlink semantics,
        // so set legacy `FileDispositionInfo` directly to keep `target` visible in
        // parent enumeration (`target.is_dir() == true`) while `held` stays open.
        let deleter = std::fs::OpenOptions::new()
            .access_mode(DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&target)
            .unwrap();
        let info = FILE_DISPOSITION_INFO { DeleteFile: true };
        // `deleter` is an open handle from `OpenOptions::open` above, and `info`
        // points to valid stack storage sized for `FILE_DISPOSITION_INFO`.
        let ok = unsafe {
            SetFileInformationByHandle(
                deleter.as_raw_handle(),
                FileDispositionInfo,
                (&info as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        };
        assert_ne!(
            ok,
            0,
            "SetFileInformationByHandle failed: {}",
            std::io::Error::last_os_error()
        );
        drop(deleter);

        assert!(
            target.is_dir(),
            "FileDispositionInfo unexpectedly hid delete-pending dir while handle was open"
        );
        assert!(!is_existing_dir(&target));
        assert!(matches!(check_existing_dir(&target), Ok(false)));

        drop(held);
        assert!(!target.is_dir());
    }

    #[cfg(windows)]
    #[test]
    fn existing_dir_rejects_remove_dir_all_while_handle_is_open() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("watched-tree");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("file.txt"), "data").unwrap();

        let held = std::fs::OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&target)
            .unwrap();

        // `remove_dir_all` attempts POSIX delete first on Windows 10+/Server
        // NTFS, so `target.is_dir()` may already be false or remain true on
        // non-POSIX filesystems; either way `is_existing_dir` reports missing.
        std::fs::remove_dir_all(&target).unwrap();
        assert!(!is_existing_dir(&target));
        assert!(matches!(check_existing_dir(&target), Ok(false)));

        drop(held);
        assert!(!is_existing_dir(&target));
    }

    #[cfg(windows)]
    #[test]
    fn existing_dir_preserves_permission_denied_as_error() {
        struct RestoreAcl(PathBuf);
        impl Drop for RestoreAcl {
            fn drop(&mut self) {
                let _ = std::process::Command::new("icacls")
                    .arg(&self.0)
                    .args(["/remove:d", "*S-1-1-0"])
                    .status();
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let locked = temp.path().join("locked");
        let saved = locked.join("saved");
        std::fs::create_dir_all(&saved).unwrap();
        let _guard = RestoreAcl(locked.clone());
        let status = std::process::Command::new("icacls")
            .arg(&locked)
            .args(["/deny", "*S-1-1-0:(OI)(CI)(RX)"])
            .status()
            .unwrap();
        assert!(status.success(), "icacls deny failed: {status}");

        let err = check_existing_dir(&saved).expect_err("permission denied must stay Err");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_minimal_service_environment_respects_xdg_and_development_roots() {
        if let Some(expected) = std::env::var_os("RUNNER_TEST_XDG_ROOT") {
            let expected = PathBuf::from(expected);
            let release = app_data_dir(false).unwrap();
            let development = app_data_dir(true).unwrap();
            assert_eq!(release, expected.join(app_identifier(false)));
            assert_eq!(development, expected.join(app_identifier(true)));
            assert_ne!(release, development);
            assert_eq!(
                daemon_endpoint(&release, false).0,
                release.join("runnerd.sock")
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        for custom_xdg in [false, true] {
            let home = root.path().join("home");
            let expected = if custom_xdg {
                root.path().join("xdg")
            } else {
                home.join(".local/share")
            };
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "app_paths::tests::linux_minimal_service_environment_respects_xdg_and_development_roots"])
                    .env_clear().env("HOME", home).env("RUNNER_TEST_XDG_ROOT", &expected);
            if custom_xdg {
                command.env("XDG_DATA_HOME", &expected);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "custom_xdg={custom_xdg}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}
