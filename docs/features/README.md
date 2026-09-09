# Feature specs

In-progress and planned feature specs. Shipped specs move to
[`archive/`](./archive/) once their tracking issue closes — the
implementation is the source of truth, but the spec stays around as the
"what we were going for" record (mirrors `docs/impls/archive/`).

Tracking lives in GitHub Issues with the `feature` label. Since 2026-09-01 a spec's number **is** its tracking issue number — file the issue first, then name the spec after it — so gaps in the doc sequence belong to bugs and PRs, never to skipped specs. Specs numbered 01–64 predate the alignment and keep their numbers in `archive/` and in the Dropped list; the then-active specs were renumbered to their issues (05→73, 58→393, 61→403). Spec 60 (fork a chat) is mid-mission and keeps its pre-alignment number; it archives as 60.

Since the GPUI rewrite shipped as `v0.6.0` (2026-08-23) there is one line of work on `main`; new features are Pencil-first in `design/runner.pen` and land as nightlies. The post-GA consolidation queue (M6) is [`../impls/gpui-rewrite/m6-remainder.md`](../impls/gpui-rewrite/m6-remainder.md), tracked in [#445](https://github.com/yicheng47/runner/issues/445).

## Index

- [533 — Update agent CLIs from Settings → Agents](./533-agent-cli-updates.md) — the other half of #475's trade: each Agents row shows the installed version from `<command> --version`, Claude Code and Codex rows get an **Update** button that opens a terminal pane running the CLI's own `update` subcommand with the agent spawn environment (non-resumable, exit overlay, version re-probed on exit), Windows disables it while sessions of that runtime are alive and macOS captions that running sessions keep the old version, and an **Update available** badge comes from the npm `latest` dist-tag fetched only while the pane is open; the #475 suppression stays ([#533](https://github.com/yicheng47/runner/issues/533)).
- [530 — Fold the MCP settings pane into Agents](./530-fold-mcp-pane-into-agents.md) — Settings → MCP only registers Runner as a tool server inside each agent's config, but the nav label promises host-side MCP management; register by default on macOS as Windows already does (once per client, Unregister sticks), show the state as a caption line on the Agents runtime rows with the Register/Unregister button, park the binding directory in a card at the bottom of Agents, drop the MCP nav item, and keep the `MCP servers` name free for the deferred #73 catalog ([#530](https://github.com/yicheng47/runner/issues/530)).
- [529 — Runner Light, a first-party light theme](./529-runner-light-theme.md) — the two light themes are palettes borrowed from the Tauri CSS tokens and `runner.pen` is dark-only, so light mode is a token swap on a dark layout wrapping a dark terminal; a `mode: light` axis on the canvas variables, a Light band of the key frames for Jason's sign-off, then `ThemeVariant::RunnerLight` as the default light theme read off the canvas, the `Runner` terminal theme following the app variant with a designed light palette, and the eight color literals outside `theme.rs` moved onto tokens; Codex Light and Latte stay selectable ([#529](https://github.com/yicheng47/runner/issues/529)).
- [527 — Mission permission mode, bypass by default](./527-mission-permission-mode.md) — a permission prompt inside a mission slot is a silent stall nobody answers and the byte-flow detector reads as idle; one app setting on a new Settings → Missions pane (Bypass default / Auto / Runner default; Default crew moves there from General), applied to every mission slot at spawn, with codex Bypass mapped to `danger-full-access` because `never` + `workspace-write` fails network and out-of-tree commands silently; no schema change, no per-mission or per-slot value, direct chats unchanged; the mission metadata panel shows the mode a mission spawned with; an optional `permission_prompt` hook signal is a later issue ([#527](https://github.com/yicheng47/runner/issues/527)).
- [524 — Answer each terminal query once](./524-double-terminal-query-replies.md) — delete the PTY reader's canned OSC 10/11, DSR, and DA1 responder (#213) so the alacritty terminal, which has answered every session from spawn since the GPUI cutover, is the only responder; measured with a ConPTY probe: under the Windows Terminal ConPTY shipped in 0.8.4 the duplicate replies reach Codex and Claude Code as input and appear as `[6c` / `C` in the composer, while either responder alone is clean ([#524](https://github.com/yicheng47/runner/issues/524)).
- [492 — Ship the Windows Terminal ConPTY and forward Windows output immediately](./492-windows-terminal-output-latency.md) — bundle Microsoft's `conpty.dll` and `OpenConsole.exe` (Windows Terminal 1.24, MIT, pinned by hash) beside `Runner.exe` so `portable-pty` uses them instead of the inbox conhost, and delete the Windows output coalescer so both platforms drain the PTY immediately; measured with a raw ConPTY probe: the newer ConPTY delivers Codex frames whole (0 split frames against 47–59), key echo drops from 47 ms to 31 ms, Codex's macOS floor, and Claude Code stays at 3–7 ms; macOS forwarder untouched ([#492](https://github.com/yicheng47/runner/issues/492)).
- [511 — Ask about a selection](./511-ask-about-selection.md) — ChatGPT-style side thread: select a passage in a chat pane, right-click or ⌥⌘A, edit the prefilled question, and the chat is natively forked (60) with the question and quoted selection as its first turn, titled from the question, opened in a split pane beside the source (new tab at the three-pane cap); the source never sees the detour; a markdown-only answer panel is a recorded non-goal ([#511](https://github.com/yicheng47/runner/issues/511)).
- [510 — Remote SSH session](./510-remote-ssh-session.md) — direct chats and terminals on another host: Start Chat (Runtime mode) and New terminal gain a Host field, the PTY child becomes `ssh -t <host> -- cd '<cwd>' && exec <runtime>` with auth left to `~/.ssh/config`, `sessions.remote_host` records the host, the local-only spawn checks (cwd, executable lookup, conversation-file probe, codex capture) step aside, claude-code resumes by uuid on the host and codex/shell respawn fresh; missions, the bus, and the `runner` CLI stay local, narrowing the vision non-goal to the multi-host coordination bus ([#510](https://github.com/yicheng47/runner/issues/510)).
- [497 — Windows code signing](./497-windows-code-signing.md) — sign the app, both CLI sidecars, the installer, and the uninstaller in both Windows packaging paths with a Certum Open Source Code Signing certificate (Open Source Developer Yicheng Wang) held in SimplySign's cloud HSM, with CI logging SimplySign Desktop in from a one-time code, `test-installer.ps1` asserting all five signatures, and the "More info → Run anyway" wording retired; SignPath (publisher name, manual approvals, no uninstaller signing) and Azure (region) were rejected ([#497](https://github.com/yicheng47/runner/issues/497)).
- [494 — macOS header navigation](./494-macos-header-navigation.md) — add Previous page and Next page arrows beside the sidebar toggle, matching Windows icons, tooltips, history navigation, and disabled states while preserving macOS window chrome ([#494](https://github.com/yicheng47/runner/issues/494)).
- [73 — Local skills management](./73-runner-skills.md) — a runner declares which skills it wants and Runner hides the rest at spawn via claude-code's `skillOverrides` riding the existing `--settings` (allowlist computed at spawn, nothing written into `~/.claude`); M1, the Settings → Skills pane (catalog per runtime, global on/off for Claude Code and Codex, in-modal `SKILL.md` editor), landed as #531 on 2026-09-09; M2 (allowlist backend) and M3 (runner-form picker with a listing-budget hint) follow ([#73](https://github.com/yicheng47/runner/issues/73)).
- [52 — Hook-based session status](./52-hook-based-session-status.md) — **record only**: #347 closed won't-do 2026-08-27, and the grid-scraping variant #455 closed 2026-08-28; busy/idle stays on the byte-flow `IdleDetector` (threshold raised to 2 s). As specced: authoritative `working`/`waiting`/`done` status from agent CLI hooks injected per spawn (claude `--settings`, codex hooks.json — never the user's config), with the byte-flow IdleDetector demoted to a universal fallback tier; adds the needs-you attention state.
- [393 — Runner & crew detail redesign](./393-runner-crew-detail-redesign.md) — Pencil-first redesign of both MVP-draft detail pages: crew detail puts the slot roster above the prose config sections instead of below them, runner detail clamps the system-prompt dump and consolidates redundant cards ([#393](https://github.com/yicheng47/runner/issues/393)).
- [403 — Opt-in worktree isolation per mission](./403-mission-worktree-isolation.md) — mission-start decorator: opted-in missions get `git worktree add <repo>/.worktrees/… -b mission/<short-id>-<slug>` and `mission.cwd` points into it, so concurrent crews and the human's checkout never collide; default stays the project checkout, all slots share the tree, archive offers safe (`git worktree remove`, dirty-refusing) cleanup ([#403](https://github.com/yicheng47/runner/issues/403)).
- [468 — Landing page for Runner](./468-landing-page.md) — one dark, responsive page that teaches runner → crew → mission before listing features, following the herdr/onorca category shape; designed first in `design/landing.pen`, hand-written HTML/CSS/JS in `site/`, deployed by a GitHub Pages workflow to `yicheng47.github.io/runner` and then `runner.wycstudios.com` ([#468](https://github.com/yicheng47/runner/issues/468)).
- [491 — Confirm quit while work is still running](./491-confirm-quit-running-work.md) — every quit path (⌘Q / menu, Dock → Quit and logout via a new `applicationShouldTerminate:` hook, last-window close on Windows) first asks the backend what is still running — agents the byte-flow detector reports busy, shell terminals with a foreground process — and, only if there is any, shows a **Quit Runner?** confirm naming them with Cancel / Quit anyway; idle chats never trigger it since auto-resume covers them; the follow-up #466's decision named when it declined the session daemon ([#491](https://github.com/yicheng47/runner/issues/491)).
- [466 — Sessions outlive the app process](./466-sessions-outlive-the-app.md) — **record only**: #466 closed won't-do 2026-09-02; PTYs stay in-process and auto-resume remains the relaunch story (vision §4.2, arch decision 12). As specced: session PTYs move into a small background host so quitting, updating, or crashing Runner.app no longer stops running agents, with reattach on relaunch and a leave-running vs stop-everything quit choice ([#466](https://github.com/yicheng47/runner/issues/466)).

## Archive

Shipped specs live in [`archive/`](./archive/), in spec-number order.
See the directory listing for what's there.

## Dropped

Considered and deliberately not built. Spec kept in-repo as the record.

- [19 — Mission split view](./19-mission-split-view.md)
  — closed as won't-do ([#255](https://github.com/yicheng47/runner/issues/255)):
  crew missions coordinate turn-based, so side-by-side slot PTYs mostly
  show one busy terminal next to an idle one; the feed + per-runner tabs
  cover monitoring, and split view already exists for direct chats.
- [21 — Import native agent sessions into a project](./21-import-native-sessions.md)
  — closed as won't-do ([#176](https://github.com/yicheng47/runner/issues/176)):
  the CLIs' own resume pickers (`claude --resume` / `codex resume` from a
  pane in the project cwd) cover the core need, so the native-store import
  machinery wasn't worth its maintenance surface.
- [24 — Cronjobs](./24-cronjobs.md)
  — closed as won't-do ([#193](https://github.com/yicheng47/runner/issues/193)):
  a resident scheduler (overlap, catch-up, timeouts, wake correctness)
  is always-on machinery inside an app whose identity is a cockpit you
  open to work in, and `mission_start` over MCP/CLI already lets any
  external scheduler fire missions on cron with zero app code. Revisit
  only if the same mission goal keeps getting launched manually on a
  rhythm.
- [53 — Session fork](./53-session-fork.md)
  — closed as won't-do ([#348](https://github.com/yicheng47/runner/issues/348)):
  months of daily use produced zero fork reaches, and the generic
  transcript-handoff tier was exactly the lossy per-runtime capture
  machinery the simplicity budget keeps refusing. The revisit trigger
  fired in 2026-08; the native tier returns, with destinations, as
  spec 60 ([#398](https://github.com/yicheng47/runner/issues/398)).
