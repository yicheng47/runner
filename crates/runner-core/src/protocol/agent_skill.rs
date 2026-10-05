use serde::{Deserialize, Serialize};
use std::path::Path;
use std::path::PathBuf;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillRootState {
    Missing,
    Current,
    Stale,
    Foreign,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRootStatus {
    pub root: PathBuf,
    pub folder: PathBuf,
    pub state: SkillRootState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallOutcome {
    Installed,
    Refreshed,
    Current,
    Foreign,
}

pub fn skill_name(debug: bool) -> &'static str {
    crate::runner_skill_name(debug)
}

pub fn sidecar_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir
        .join("bin")
        .join(format!("runner{}", std::env::consts::EXE_SUFFIX))
}

pub fn render(debug: bool, sidecar: &Path) -> String {
    let name = skill_name(debug);
    let description = if debug {
        "Development build of Runner (the `make run` app, separate data from the installed app): operate its local cockpit for CLI coding agents through the `runner-dev` skill. Use when the user says dev, development build, `make run` or `runner-dev`."
    } else {
        "Operate Runner, the local cockpit for CLI coding agents, through the `runner` CLI: start, follow and steer missions, list crews and roles, start chats. Use when the user mentions Runner, a mission, a crew or a role, or asks to hand work to another agent such as Codex, Claude Code or pi."
    };
    let invocation = if debug {
        format!(
            "This is Runner's development build. It talks only to the development app.\n\nAlways invoke \"{}\"; a bare `runner` on PATH may be the installed app.\n\nRun \"{}\" help agents for the version-matched command guide.",
            sidecar.display(),
            sidecar.display()
        )
    } else {
        format!(
            "Use `runner` when it is on PATH. Otherwise invoke \"{}\".\n\nRun `runner help agents` for the version-matched command guide.",
            sidecar.display()
        )
    };
    let command = if debug {
        format!("\"{}\"", sidecar.display())
    } else {
        "runner".to_owned()
    };
    format!(
        r#"---
name: {name}
description: "{description}"
---

# Runner

Runner is the local cockpit for coordinating CLI coding agents. Crews are reusable teams; missions are their live runs.

{invocation}

Rules:

1. Prefer `--json` for structured output.
2. Exit code 3 means Runner is not running; ask the user to open Runner. Exit code 5 means a sandbox kept the command from reaching Runner, so nothing is known about Runner yet: run the same command again outside the sandbox (in Codex, request escalated permissions for it) before telling the user anything.
3. Use `--help` rather than guessing commands.
4. Inside a mission, mission commands carry your own handle. Outside a mission you act for the user: your posts and answers appear as the person. Never pass `--as` to speak as a slot you were not given.
5. Start means start and watch: after every successful `{command} mission start`, use the returned mission ID to arm `{command} mission feed <id> --follow --json` in your host's supported background/watch facility before reporting delegation complete. Keep exactly one watcher per mission, bound to that exact ID; reuse it on later turns. Its events, stderr diagnostics and exit must reach you during later work and while idle. An unread background log or a completion-only notification is not a watch. Plain `mission start` still returns and exits.
6. Surface crew handoffs, human questions, completion, stop/crash and lost connections promptly; the follower polls every 3 seconds and flushes events immediately. Keep routine noise hidden (do not add `--all`). Clean up the watcher when the mission ends or all sessions exit; a resumed mission needs a new watch. A watch request timeout exits 1 and does not mean Runner is closed. On host expiry or unexpected exit, check `{command} mission show <id> --json`, report the gap, and if the mission is still active re-arm exactly one watch with `--since <next_offset> --oldest-first`, taking `next_offset` from the last event line you received; only when no event line arrived, use `--since 0 --oldest-first` and skip event IDs already seen.
7. A watch facility qualifies only if it delivers each output line and the exit to you while idle. In Claude Code, use Monitor with `2>&1` appended to the feed command and `timeout_ms` at its maximum. A completion-only background command, a PTY you must poll, a task list or a log file does not qualify. Without one, say plainly: "Mission <id> started, but this host cannot notify me of later events; automatic watching is unavailable." Give `{command} mission feed <id> --follow --json` as the foreground follow-up command. Never claim an active watch without a delivery path; stop any undeliverable follower before ending your turn.
"#
    )
}

pub const SKILL_ROOTS: &[&str] = crate::RUNNER_SKILL_ROOTS;
pub const SKILL_MARKER: &str = crate::RUNNER_SKILL_MARKER;
