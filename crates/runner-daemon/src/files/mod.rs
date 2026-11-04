//! The Files view's service (#634): roots, listings, git status, worktrees,
//! reading and watching, all behind client-protocol requests so a remote
//! client can later be served the same way. Every path a client sends is a
//! root plus a path inside it, checked by [`fs::resolve`].

pub mod fs;
pub mod git;
pub mod root;

#[cfg(test)]
pub(crate) mod fixture {
    use std::path::Path;
    use std::process::Command;

    use super::git::Git;

    pub fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=Runner",
                "-c",
                "user.email=runner@example.com",
            ])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    pub struct Repo {
        pub dir: tempfile::TempDir,
        pub git: Git,
    }

    impl Repo {
        pub fn path(&self) -> &Path {
            self.dir.path()
        }
    }

    /// A committed repository on `main` with `src/deep/x.rs`, `a.txt`,
    /// `old.txt`, `bin.dat` and a `.gitignore` of `target/`, `.worktrees/`
    /// and `*.log`.
    pub fn repo() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]);
        git(path, &["config", "core.autocrlf", "false"]);
        std::fs::write(path.join(".gitignore"), "target/\n.worktrees/\n*.log\n").unwrap();
        std::fs::create_dir_all(path.join("src/deep")).unwrap();
        std::fs::write(path.join("src/deep/x.rs"), "fn x() {}\n").unwrap();
        std::fs::write(path.join("a.txt"), "a\n").unwrap();
        std::fs::write(path.join("old.txt"), "old contents\n").unwrap();
        std::fs::write(path.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
        git(path, &["add", "-A"]);
        git(path, &["commit", "-q", "-m", "init"]);
        Repo {
            dir,
            git: Git::find(std::env::var("PATH").unwrap_or_default()).expect("git on PATH"),
        }
    }
}

