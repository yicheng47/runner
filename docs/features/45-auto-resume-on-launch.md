# 45 — Auto-resume running chats and missions on launch

> Tracking issue: [#320](https://github.com/yicheng47/runner/issues/320)

## Motivation

Quitting the app kills every running agent (PTYs die with the process; `stop_running_sessions_on_quit` stops direct chats gracefully, startup demotes stale `running` rows). On next launch the user manually resumes each chat and mission they were working in — pure friction, since the app already knows how to resume everything: sessions persist `agent_session_key`, `session_resume` respawns into the prior conversation (impl 0024 keeps claude-code scrollback), and missions keep `status = running` across restarts (`mount_all_running_mission_routers` re-mounts their buses at startup — only their sessions are dead).

The gap is memory plus initiative: nothing records *which* sessions were live at quit, and nothing acts on it at launch.

## Scope

### In scope

- **Mark at quit.** `stop_running_sessions_on_quit` already enumerates running direct sessions; stamp them (`resume_on_launch` flag on the session row) before killing. The same pass marks running-slot sessions of running missions (they're demoted by startup cleanup today with no trace). A crash skips the stamp — see key decision 2.
- **Auto-resume at launch.** After the webview is ready, resume every marked session that is still resumable (`agent_session_key` present, not archived), clearing the flag as each is consumed. Missions need no extra start step: their status is still `running` and buses re-mount as today — resuming their marked slot sessions brings the workspace back to life.
- **Staggered spawns.** Resume sequentially with a short gap, not as one burst — N simultaneous PTY spawns + login-shell env snapshots is a stampede for no benefit.
- **Failure tolerance.** The existing resume-failure heuristic (fast death → `crashed` + warning toast, next launch starts fresh) already covers rejected `--resume` keys; auto-resume inherits it. A failed auto-resume must not block the rest of the queue.
- **Setting to disable.** One toggle in Settings ("Resume running agents on launch", default on). No per-chat granularity in v1.

### Out of scope

- Restoring UI state beyond what already persists (sidebar tree, tab layouts, window geometry are all covered; the restored sessions simply light up their existing rows).
- Auto-resuming sessions the user stopped *manually* before quitting — stopped means stopped; only quit-time-running sessions are marked.
- Re-injecting prompts or auto-continuing agent work. Resume reopens the conversation; the agent stays idle until spoken to.
- Cross-device / sync anything.

### Key decisions

1. **Explicit flag, not timestamp inference.** Inferring "was running at quit" from `stopped_at` proximity to shutdown confuses deliberately-stopped chats with quit-killed ones. The quit hook knows exactly which rows it's killing; it should say so.
2. **Crash = no auto-resume.** The stamp lives in the graceful-quit path only. After a crash, sessions demote via startup cleanup as today and stay stopped — auto-respawning agents after a crash risks looping into whatever caused it. (If crash-restore is ever wanted, it's a separate, deliberate decision.)
3. **Resume, never fresh-spawn.** A marked session that lost its `agent_session_key` (or whose resume fails) stays stopped with the existing Resume affordance — auto-starting a *fresh* conversation the user didn't ask for is worse than doing nothing.

### To be decided

- Stagger interval (likely 250–500ms between spawns).
- Whether a launch with many marked sessions (say >6) should resume silently or show a one-line "Resuming N agents…" indicator.

## Implementation phases

1. **Schema + quit stamp** — `resume_on_launch` column (sessions), stamped in `stop_running_sessions_on_quit` for direct chats and running-mission slot sessions.
2. **Launch consumer** — post-ready sequential resume of marked resumable sessions via the existing `session_resume` path; flag cleared per session; Settings toggle gating the whole pass.
3. **Polish** — stagger tuning, resume indicator if decided in.

## Verification

- [ ] Quit with two running chats and a running mission → relaunch → all three come back live without interaction; scrollback intact for claude-code.
- [ ] A chat stopped manually before quit stays stopped after relaunch.
- [ ] Kill the app process (simulated crash) → nothing auto-resumes.
- [ ] A session with a rejected resume key surfaces the existing crash warning and doesn't block other resumes.
- [ ] Toggle off → relaunch restores nothing; rows keep their normal Resume buttons.
- [ ] `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint` clean.
