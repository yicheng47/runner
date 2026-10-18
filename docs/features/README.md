# Feature specs

Specs of features in development. A feature is filed as an issue only; its spec is written when development starts, on the feature's branch, and lands in the same pull request as the code. At a release sweep, retain a spec in [`archive/`](./archive/) only when its decisions, compatibility constraints or implementation context still help. Remove obsolete or duplicate proposals after preserving useful unique content and fixing references; an issue and git history are enough for many dropped ideas. Priorities and milestones live on the issues; [`../roadmap.md`](../roadmap.md) mirrors them.

Since 2026-09-01 a spec's number **is** its tracking issue number: file the issue first, then name the spec after it, so gaps in the sequence belong to bugs and PRs. Specs 01–64 predate the alignment and keep their numbers in `archive/`; the then-active ones were renumbered to their issues (05→73, 58→393, 61→403), and 60 (fork a chat) kept its number and shipped 2026-09-01.

## Active

- [965 — Filtered message read feedback](./965-inbox-read-feedback.md) — actionable unread-inbox feedback for filtered CLI reads and clearer agent guidance ([#965](https://github.com/yicheng47/runner/issues/965)).
- [947 — Mission feed scrollbar](./947-mission-feed-scrollbar.md) — the shared app scrollbar on the virtual mission feed ([#947](https://github.com/yicheng47/runner/issues/947)).

- [748 — Mission updates reach the starting chat](./748-mission-watch-delivery.md) — the return path for delegation; on hold after #826 since 2026-10-17, because Jason rejected Runner typing updates into a normal chat, and the spec keeps the refreshed proposal for the redesign ([#748](https://github.com/yicheng47/runner/issues/748)).
- [634 — File preview and code review](./634-file-preview-and-review.md) — a project tree, read-only file/diff tabs, a base picker and review comments returned to the agent; design before implementation ([#634](https://github.com/yicheng47/runner/issues/634)).
- [559 — Command palette](./559-command-palette.md) — command mode over the existing keymap and quick-switcher overlay ([#559](https://github.com/yicheng47/runner/issues/559)).
- [836 — Web remote control](./836-web-remote-control.md) — Phase 1 connects a host to an account: device-code enrollment from Settings → Remote access or `runner remote login`, who may connect a host, secrets in the OS secret store, the outbound relay connection and its states; the companion protocol, encrypted channel and terminals follow, then a Tailscale route released together with the hosted service ([#836](https://github.com/yicheng47/runner/issues/836)); Phase 1's desktop side merged into the `nightly-remote` umbrella on 2026-10-18 ([#945](https://github.com/yicheng47/runner/pull/945)); see the [program record](../impls/836-web-remote-control.md).

Future work, including [iOS follow-through](https://github.com/yicheng47/runner/issues/912), [shell/agent separation](https://github.com/yicheng47/runner/issues/795) and [pi MCP management](https://github.com/yicheng47/runner/issues/764), stays in its issue until development starts. Milestones and priorities are listed in the [roadmap](../roadmap.md), not duplicated in this index.

## Recently shipped

Reconciled on 2026-10-19 against closed issues and published releases. These specs are archived; their bodies preserve the original scope and dated design decisions.

| Spec | Shipped in |
| --- | --- |
| [706 — Plan usage for Claude Code and Codex](./archive/706-agent-usage.md) | 0.11.4, 2026-09-23 |
| [533 — Update agent CLIs from Settings → Agents](./archive/533-agent-cli-updates.md) | 0.11.5, 2026-09-26 |
| [575 — Splits follow the shell's live cwd](./archive/575-live-cwd.md) | 0.11.5, 2026-09-26 |
| [393 — Role page redesign](./archive/393-role-page.md) | 0.12.0, 2026-09-27 |
| [699 — Crew page redesign](./archive/699-crew-page.md) | 0.12.0, 2026-09-27 |
| [740 — Codex speed in role settings](./archive/740-codex-role-speed.md) | 0.12.1, 2026-09-28 |
| [724 — Preserve terminal wheel coordinates](./archive/724-terminal-wheel-coordinates.md) | 0.12.2, 2026-09-28 |
| [644 — Antigravity CLI runtime](./archive/644-antigravity-runtime.md) | 0.12.3, 2026-09-29; native Windows smoke pending |
| [735 — Start a chat modal redesign](./archive/735-start-chat-modal.md) | 0.12.5, 2026-09-30 |
| [756 — Model and effort in the chat side panel](./archive/756-chat-side-panel-model-effort.md) | 0.12.5, 2026-09-30 |
| [731 — Role side panel redesign](./archive/731-role-side-panel.md) | 0.12.6, 2026-10-01 |
| [768 — New role and New crew open their pages](./archive/768-new-role-crew-pages.md) | 0.12.6, 2026-10-01 |
| [772 — Start chats, terminals and missions from the keyboard](./archive/772-keyboard-create.md) | 0.12.8, 2026-10-04 ([PR #776](https://github.com/yicheng47/runner/pull/776)) |
| [777 — Runtime adapter trait](./archive/777-runtime-adapter.md) | 0.12.8, 2026-10-04 ([PR #779](https://github.com/yicheng47/runner/pull/779), [#780](https://github.com/yicheng47/runner/pull/780), [#792](https://github.com/yicheng47/runner/pull/792)); no user-visible change |
| [791 — Session state: one model for agent status, drafts and conversation identity](./archive/791-session-state.md) | 0.12.8, 2026-10-04 ([PR #796](https://github.com/yicheng47/runner/pull/796), [#798](https://github.com/yicheng47/runner/pull/798), [#800](https://github.com/yicheng47/runner/pull/800)); fixes #784, #785, #786 and #781 |
| [645 — runnerd](./archive/645-session-host.md) | 0.13.0, 2026-10-06 ([#811](https://github.com/yicheng47/runner/pull/811)); the CLI moved to its client protocol on 2026-10-07 ([#822](https://github.com/yicheng47/runner/pull/822), #821), and remote machines over ssh continue as [#808](https://github.com/yicheng47/runner/issues/808) |
| [797 — Hook status over local IPC](./archive/797-hook-status-ipc.md) | 0.13.1, 2026-10-08 |
| [793 — Liquid glass appearance](./archive/793-liquid-glass.md) | 0.14.0, 2026-10-09 ([#852](https://github.com/yicheng47/runner/pull/852), follow-up [#854](https://github.com/yicheng47/runner/pull/854)). Menus and the usage popover stayed solid; Windows is Solid only |
| [723 — Cursor Agent CLI runtime](./archive/723-cursor-agent-runtime.md) | 0.15.0, 2026-10-10 ([#827](https://github.com/yicheng47/runner/pull/827)); enabled by default since 0.15.2; native Windows unverified |
| [723 — Grok Build runtime](./archive/723-grok-build-runtime.md) | 0.15.2, 2026-10-12 ([#871](https://github.com/yicheng47/runner/pull/871), follow-up [#872](https://github.com/yicheng47/runner/pull/872)), enabled by default; native Windows and Linux unverified |
| [701 — Desktop notifications](./archive/701-desktop-notifications.md) | 0.16.1, 2026-10-14 ([#891](https://github.com/yicheng47/runner/pull/891)); the operating system's own notifications, background-only, with chat events on and four mission events off by default; native checks were left to Jason's PR build |
| [592 — OpenCode v2 runtime](./archive/592-opencode-runtime.md) | 0.16.1, 2026-10-14 ([#876](https://github.com/yicheng47/runner/pull/876)) from a contributor's fork after the original #592 closed unbuilt; enabled by default; inference-dependent native checks and Windows unverified |
| [849 — Start mission dialog redesign](./archive/849-start-mission-modal.md) | 0.16.2, 2026-10-15 ([#906](https://github.com/yicheng47/runner/pull/906)), with No crew for [562](./archive/562-mission-spawn.md)'s Phase 3; native Windows unverified |
| [562 — Missions as containers](./archive/562-mission-spawn.md) | 0.15.2 to 0.16.2; [#562](https://github.com/yicheng47/runner/issues/562) closed 2026-10-16. The roster in 0.15.2 ([#868](https://github.com/yicheng47/runner/pull/868)), independent sessions in 0.15.3 ([#874](https://github.com/yicheng47/runner/pull/874)), adding and removing roles in 0.16.0 ([#889](https://github.com/yicheng47/runner/pull/889)) and the empty start in 0.16.2 ([#906](https://github.com/yicheng47/runner/pull/906)). The lead's own verbs and outside seats continue in [#916](https://github.com/yicheng47/runner/issues/916), the starting-chat return path in [#748](https://github.com/yicheng47/runner/issues/748) |
| [565 — English and Simplified Chinese UI](./archive/565-i18n.md) | 0.16.4, 2026-10-17 ([#926](https://github.com/yicheng47/runner/pull/926)); System, English and 简体中文 in Settings → General, switching live across windows. The final candidate's native retest was waived; native Windows and an alternate OS language unverified |
| [826 — Targeted crew messaging](./archive/826-targeted-messaging.md) | 0.16.5, 2026-10-18 ([#939](https://github.com/yicheng47/runner/pull/939)), on [#907](https://github.com/yicheng47/runner/issues/907)'s reliable delivery ([#921](https://github.com/yicheng47/runner/pull/921), plan in [`impls/archive/`](../impls/archive/907-reliable-delivery.md)); several recipients, leading-mention addressing and unread-only reads with a digest. A pre-existing empty-prompt draft hold continues as [#940](https://github.com/yicheng47/runner/issues/940); native Windows unverified |
| [898 — Drop files onto a terminal](./archive/898-terminal-file-drop.md) | 0.16.5, 2026-10-18 ([#903](https://github.com/yicheng47/runner/pull/903)); native Windows unverified |
| [887 — Active sidebar and sidebar refresh](./archive/887-active-sidebar.md) | 0.16.6, 2026-10-19 ([#958](https://github.com/yicheng47/runner/pull/958)); one row per tab with attention-pane activation, and the Codex and Claude usage footer. Jason confirmed it natively on macOS; Windows unverified |

## Selected decision records

- [42 — Headless terminal model](./archive/42-headless-terminal-model.md) — the failed replay approaches and terminal-fixture requirements behind the native rewrite; the proposed Tauri implementation was never built.
- [403 — Opt-in worktree isolation per mission](./archive/403-mission-worktree-isolation.md) — why Runner leaves checkout creation to the crew's brief.

Obsolete proposals are in their issues and git history. Scheduled runs continue in [#193](https://github.com/yicheng47/runner/issues/193), delegation in [#562](https://github.com/yicheng47/runner/issues/562), and semantic shell integration in [#904](https://github.com/yicheng47/runner/issues/904); their discarded predecessors are not separate plans to implement.
