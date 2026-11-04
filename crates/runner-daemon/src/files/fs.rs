//! Listing and reading inside a root, behind the path check.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use runner_core::companion::TerminalBytes;
use runner_core::protocol::files::{
    DirEntry, EntryKind, FileContent, GitStatus, BINARY_SNIFF_BYTES, MAX_READ_BYTES,
};

use crate::error::{Error, Result};

/// The root must be an existing absolute folder.
pub fn check_root(root: &Path) -> Result<()> {
    if !root.is_absolute() {
        return Err(Error::msg(format!(
            "{} is not an absolute path",
            root.display()
        )));
    }
    if !root.is_dir() {
        return Err(Error::msg(format!(
            "{} is not an existing folder",
            root.display()
        )));
    }
    Ok(())
}

/// Resolves `relative` inside `root`, refusing `..`, an absolute path, and
/// a path whose existing part leads outside the root through a symlink.
pub fn resolve(root: &Path, relative: &str) -> Result<PathBuf> {
    check_root(root)?;
    let refused = || Error::msg(format!("{relative:?} is outside the root"));
    #[cfg(windows)]
    let relative_path = relative.replace('\\', "/");
    #[cfg(not(windows))]
    let relative_path = relative.to_owned();
    if relative_path.starts_with('/') {
        return Err(refused());
    }
    let mut path = root.to_path_buf();
    for part in relative_path.split('/') {
        let mut components = Path::new(part).components();
        match (components.next(), components.next()) {
            (None, _) | (Some(Component::CurDir), None) => {}
            (Some(Component::Normal(name)), None) => path.push(name),
            _ => return Err(refused()),
        }
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| Error::msg(format!("{}: {error}", root.display())))?;
    let mut existing = path.as_path();
    let canonical = loop {
        match existing.canonicalize() {
            Ok(canonical) => break canonical,
            Err(_) => match existing.parent() {
                Some(parent) if parent.starts_with(root) => existing = parent,
                _ => return Err(refused()),
            },
        }
    };
    if !canonical.starts_with(&canonical_root) {
        return Err(refused());
    }
    Ok(path)
}

fn join(folder: &str, name: &str) -> String {
    let folder = folder.trim_matches('/');
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// A folder's entries, folders first, `.git` left out, ignored entries
/// marked from `status`.
pub fn list(root: &Path, folder: &str, status: Option<&GitStatus>) -> Result<Vec<DirEntry>> {
    let path = resolve(root, folder)?;
    let read = std::fs::read_dir(&path)
        .map_err(|error| Error::msg(format!("{}: {error}", path.display())))?;
    let mut entries = Vec::new();
    for entry in read {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let (kind, size) = if file_type.is_symlink() {
            (EntryKind::Symlink, None)
        } else if file_type.is_dir() {
            (EntryKind::Folder, None)
        } else {
            (
                EntryKind::File,
                entry.metadata().ok().map(|metadata| metadata.len()),
            )
        };
        let ignored = status.is_some_and(|status| status.is_ignored(&join(folder, &name)));
        entries.push(DirEntry {
            name,
            kind,
            ignored,
            size,
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Folder)
            .cmp(&(b.kind != EntryKind::Folder))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

/// Reads `relative` under `root`: its text, or for a binary file its size,
/// and its bytes too with `want_bytes`. Either way, nothing over
/// [`MAX_READ_BYTES`] and nothing outside the root.
pub fn read(root: &Path, relative: &str, want_bytes: bool) -> Result<FileContent> {
    let path = resolve(root, relative)?;
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileContent::Missing)
        }
        Err(error) => return Err(Error::msg(format!("{}: {error}", path.display()))),
    };
    let not_a_file = || Error::msg(format!("{relative:?} is not a regular file"));
    if !metadata.is_file() {
        return Err(not_a_file());
    }
    if metadata.len() > MAX_READ_BYTES {
        return Ok(FileContent::TooLarge {
            size: metadata.len(),
        });
    }
    // Non-blocking, so a file swapped for a FIFO after the check above
    // cannot hold the request open waiting for a writer.
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileContent::Missing)
        }
        Err(error) => return Err(Error::msg(format!("{}: {error}", path.display()))),
    };
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(not_a_file());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_READ_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::msg(format!("{}: {error}", path.display())))?;
    let size = bytes.len() as u64;
    if size > MAX_READ_BYTES {
        return Ok(FileContent::TooLarge { size });
    }
    if bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
        return Ok(FileContent::Binary {
            size,
            bytes: want_bytes.then_some(TerminalBytes(bytes)),
        });
    }
    Ok(FileContent::Text {
        bytes: TerminalBytes(bytes),
        size,
        modified: metadata.modified().ok().map(chrono::DateTime::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/deep")).unwrap();
        std::fs::write(dir.path().join("src/deep/x.rs"), "fn x() {}\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# hi\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("a.log"), "").unwrap();
        dir
    }

    #[test]
    fn dot_dot_and_absolute_paths_are_refused() {
        let dir = tree();
        for path in ["..", "../x", "src/../../x", "src/..", "/etc/passwd"] {
            assert!(resolve(dir.path(), path).is_err(), "{path} was accepted");
        }
        #[cfg(windows)]
        for path in ["C:\\Windows", "C:/Windows", "\\x", "src\\..\\..\\x"] {
            assert!(resolve(dir.path(), path).is_err(), "{path} was accepted");
        }
        assert_eq!(resolve(dir.path(), "").unwrap(), dir.path());
        assert_eq!(
            resolve(dir.path(), "src/./deep/x.rs").unwrap(),
            dir.path().join("src").join("deep").join("x.rs")
        );
        assert!(resolve(Path::new("relative"), "").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_root_is_refused_and_one_inside_reads() {
        let dir = tree();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("sf")).unwrap();
        std::os::unix::fs::symlink("src/deep", dir.path().join("in")).unwrap();
        assert!(read(dir.path(), "out/secret", false).is_err());
        assert!(read(dir.path(), "out/missing", false).is_err());
        assert!(read(dir.path(), "sf", false).is_err());
        assert!(list(dir.path(), "out", None).is_err());
        assert!(matches!(
            read(dir.path(), "in/x.rs", false).unwrap(),
            FileContent::Text { .. }
        ));
        let entries = list(dir.path(), "", None).unwrap();
        let kind = |name: &str| {
            entries
                .iter()
                .find(|entry| entry.name == name)
                .unwrap()
                .kind
        };
        assert_eq!(kind("out"), EntryKind::Symlink);
        assert_eq!(kind("in"), EntryKind::Symlink);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_without_waiting_for_a_writer() {
        let dir = tree();
        let fifo = dir.path().join("pipe");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        std::os::unix::fs::symlink("pipe", dir.path().join("to-pipe")).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let root = dir.path().to_path_buf();
        std::thread::spawn(move || {
            let _ = tx.send((
                read(&root, "pipe", false).is_err(),
                read(&root, "to-pipe", false).is_err(),
            ));
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)),
            Ok((true, true))
        );
    }

    #[test]
    fn listing_hides_git_sorts_folders_first_and_marks_ignored() {
        let dir = tree();
        let status = GitStatus::new(vec![], vec!["target/".into(), "a.log".into()]);
        let entries = list(dir.path(), "", Some(&status)).unwrap();
        let summary = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind, entry.ignored))
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                ("src", EntryKind::Folder, false),
                ("target", EntryKind::Folder, true),
                ("a.log", EntryKind::File, true),
                ("README.md", EntryKind::File, false),
            ]
        );
        assert_eq!(entries[3].size, Some(5));
        let nested = list(dir.path(), "target", Some(&status)).unwrap();
        assert!(nested[0].ignored);
    }

    #[test]
    fn reading_sorts_text_binary_large_and_missing() {
        let dir = tree();
        let content = read(dir.path(), "README.md", false).unwrap();
        let FileContent::Text {
            bytes,
            size,
            modified,
        } = content
        else {
            panic!("expected text, got {content:?}");
        };
        assert_eq!(bytes.0, b"# hi\n");
        assert_eq!(size, 5);
        assert!(modified.is_some());

        let mut binary = vec![b'a'; BINARY_SNIFF_BYTES];
        binary[BINARY_SNIFF_BYTES - 1] = 0;
        std::fs::write(dir.path().join("bin"), &binary).unwrap();
        assert_eq!(
            read(dir.path(), "bin", false).unwrap(),
            FileContent::Binary {
                size: BINARY_SNIFF_BYTES as u64,
                bytes: None,
            }
        );
        let mut late_nul = vec![b'a'; BINARY_SNIFF_BYTES + 1];
        late_nul[BINARY_SNIFF_BYTES] = 0;
        std::fs::write(dir.path().join("late"), &late_nul).unwrap();
        assert!(matches!(
            read(dir.path(), "late", false).unwrap(),
            FileContent::Text { .. }
        ));

        let limit = vec![b'x'; MAX_READ_BYTES as usize];
        std::fs::write(dir.path().join("limit"), &limit).unwrap();
        assert!(matches!(
            read(dir.path(), "limit", false).unwrap(),
            FileContent::Text { size, .. } if size == MAX_READ_BYTES
        ));
        let mut over = limit;
        over.push(b'x');
        std::fs::write(dir.path().join("over"), &over).unwrap();
        assert_eq!(
            read(dir.path(), "over", false).unwrap(),
            FileContent::TooLarge {
                size: MAX_READ_BYTES + 1
            }
        );

        assert_eq!(
            read(dir.path(), "nope.txt", false).unwrap(),
            FileContent::Missing
        );
        assert_eq!(
            read(dir.path(), "no/such/dir.txt", false).unwrap(),
            FileContent::Missing
        );
        assert!(read(dir.path(), "src", false).is_err());
    }

    #[test]
    fn a_binary_file_brings_its_bytes_only_when_asked_and_within_the_limit() {
        let dir = tree();
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        std::fs::write(dir.path().join("shot.png"), &png).unwrap();
        assert_eq!(
            read(dir.path(), "shot.png", false).unwrap(),
            FileContent::Binary {
                size: png.len() as u64,
                bytes: None,
            }
        );
        assert_eq!(
            read(dir.path(), "shot.png", true).unwrap(),
            FileContent::Binary {
                size: png.len() as u64,
                bytes: Some(TerminalBytes(png)),
            }
        );
        assert!(
            matches!(
                read(dir.path(), "README.md", true).unwrap(),
                FileContent::Text { .. }
            ),
            "text reads as before"
        );

        let mut limit = vec![0u8; MAX_READ_BYTES as usize];
        std::fs::write(dir.path().join("limit.bin"), &limit).unwrap();
        assert!(matches!(
            read(dir.path(), "limit.bin", true).unwrap(),
            FileContent::Binary { size, bytes: Some(bytes) }
                if size == MAX_READ_BYTES && bytes.0.len() == MAX_READ_BYTES as usize
        ));
        limit.push(0);
        std::fs::write(dir.path().join("over.bin"), &limit).unwrap();
        assert_eq!(
            read(dir.path(), "over.bin", true).unwrap(),
            FileContent::TooLarge {
                size: MAX_READ_BYTES + 1
            }
        );

        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.png"), b"\0secret").unwrap();
        let absolute = outside.path().join("secret.png");
        for path in [
            "../secret.png",
            "src/../../secret.png",
            absolute.to_str().unwrap(),
        ] {
            assert!(read(dir.path(), path, true).is_err(), "{path} was read");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
            std::os::unix::fs::symlink(absolute, dir.path().join("linked.png")).unwrap();
            assert!(read(dir.path(), "out/secret.png", true).is_err());
            assert!(read(dir.path(), "linked.png", true).is_err());
        }
    }
}
