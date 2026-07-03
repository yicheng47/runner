//! Windows ↔ WSL path translation, confined to the WSL runtime boundary.
//!
//! The rest of the backend (manager, DB, commands, event bus) only ever
//! handles native Windows paths; the WSL shaper translates the few that
//! cross into the distro (cwd today; shim/bundled dirs and the event-log
//! path in M4+). Pure string mapping — no `wslpath` subprocess — so it's
//! fast and trivially testable.

use std::path::Path;

/// Map a Windows path to its in-distro WSL form.
///
/// * Drive paths: `C:\Users\x\p` → `/mnt/c/Users/x/p` (drive letter
///   lowercased, `\` → `/`).
/// * `\\wsl$\Ubuntu\home\h` / `\\wsl.localhost\Ubuntu\home\h` → `/home/h`
///   (already in-distro; strip the UNC prefix + distro segment).
/// * A path that already looks POSIX (`/home/h`) passes through.
///
/// Returns the original (slash-normalized) string if it matches no known
/// shape — better to hand WSL something than to drop the cwd silently.
pub fn win_to_wsl(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    win_to_wsl_str(&s)
}

fn win_to_wsl_str(s: &str) -> String {
    // Already POSIX.
    if s.starts_with('/') {
        return s.to_string();
    }

    // UNC into WSL: //wsl$/Distro/rest  or  //wsl.localhost/Distro/rest
    for prefix in ["//wsl$/", "//wsl.localhost/"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            // rest = "Distro/home/h" → drop the distro segment.
            return match rest.split_once('/') {
                Some((_distro, tail)) => format!("/{tail}"),
                None => "/".to_string(),
            };
        }
    }

    // Drive path: "C:/Users/x" → "/mnt/c/Users/x".
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = &s[2..]; // includes leading '/' for "C:/..."; "C:" → ""
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        return if rest.is_empty() {
            format!("/mnt/{drive}")
        } else {
            format!("/mnt/{drive}/{rest}")
        };
    }

    // Unknown shape — hand back the slash-normalized original.
    s.to_string()
}

/// Map an in-distro WSL path back to a Windows path. Inverse of
/// [`win_to_wsl`] for the `/mnt/<drive>/…` shape; other POSIX paths are
/// returned as a `\\wsl$`-relative best effort. Kept for completeness
/// (settings display, round-tripping) — not on the spawn hot path.
#[allow(dead_code)]
pub fn wsl_to_win(s: &str, distro: &str) -> String {
    if let Some(rest) = s.strip_prefix("/mnt/") {
        // "c/Users/x" → "C:\Users\x"
        let mut chars = rest.chars();
        if let Some(drive) = chars.next() {
            let tail = chars.as_str().strip_prefix('/').unwrap_or(chars.as_str());
            let tail_win = tail.replace('/', "\\");
            return if tail_win.is_empty() {
                format!("{}:\\", drive.to_ascii_uppercase())
            } else {
                format!("{}:\\{}", drive.to_ascii_uppercase(), tail_win)
            };
        }
    }
    // In-distro (ext4) path → UNC.
    let tail = s.strip_prefix('/').unwrap_or(s).replace('/', "\\");
    format!("\\\\wsl$\\{distro}\\{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn drive_paths() {
        assert_eq!(win_to_wsl(&PathBuf::from(r"C:\Users\Haochen\proj")), "/mnt/c/Users/Haochen/proj");
        assert_eq!(win_to_wsl(&PathBuf::from(r"D:\code")), "/mnt/d/code");
        assert_eq!(win_to_wsl(&PathBuf::from(r"C:\")), "/mnt/c");
    }

    #[test]
    fn unc_into_wsl() {
        assert_eq!(win_to_wsl(&PathBuf::from(r"\\wsl$\Ubuntu\home\h\p")), "/home/h/p");
        assert_eq!(win_to_wsl(&PathBuf::from(r"\\wsl.localhost\Ubuntu\home\h")), "/home/h");
    }

    #[test]
    fn posix_passthrough() {
        assert_eq!(win_to_wsl(&PathBuf::from("/home/h/proj")), "/home/h/proj");
    }

    #[test]
    fn spaces_preserved() {
        assert_eq!(
            win_to_wsl(&PathBuf::from(r"C:\Program Files\app")),
            "/mnt/c/Program Files/app"
        );
    }

    #[test]
    fn round_trip_drive() {
        let win = r"C:\Users\Haochen\proj";
        let wsl = win_to_wsl(&PathBuf::from(win));
        assert_eq!(wsl_to_win(&wsl, "Ubuntu"), r"C:\Users\Haochen\proj");
    }
}
