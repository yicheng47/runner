# #1272 — Remember each remote host's sidebar view

Jason asked on 2026-11-12 for the codex trio to fix [#1272](https://github.com/yicheng47/runner/issues/1272). Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1272-remote-sidebar-collapse` on `fix/1272-remote-sidebar-collapse`, cut from `origin/main` at `b841b51a`. Use `/tmp/1272-` as the prefix for any scratch file.

## Problem

Opening a remote host's window shows every project expanded. `Sidebar::set_front_page_target` (`sidebar/model.rs`) seeds a remote sidebar by copying the local settings and clearing the collapsed projects, Active groups and link groups, opening Projects and Chats and turning Active only off. Changes then stay on the sidebar's in-memory `remote_settings` (`Sidebar::update_app_settings`, `sidebar/state.rs`), which `RemoteClient::remember_remote_selection` keeps in `RemoteSelection.sidebar_settings` (`remote_client/mod.rs`) only while the app runs and only while that host's overview stays loaded. The restore in `remote_client/workspace/mod.rs` also drops the collapsed link groups. So every app launch, and every first visit in a new window, expands everything again.

## Decisions (Jason, 2026-11-12, recorded on the issue)

1. **Client-side, per host.** Save each host's remote sidebar view in this computer's app settings (`ui-settings.json`), keyed by computer id, and restore it whenever a window opens that host. No protocol or daemon change: it is a per-viewer preference, and the host's own sidebar state lives in its app's settings, not in `runnerd`.
2. **What is saved:** Show active only, the Projects and Chats sections' open state, and the collapsed projects, Active groups and link groups. The rest of the remote sidebar's settings keep following the local ones as today (keymap overrides, for example).
3. **First-connect default.** When a host has no saved view, seed it once, as soon as that host's overview is available: projects with no chats start collapsed, projects with chats expanded, Projects and Chats open, Active only off. Saving the seeded view is what marks the host as seen, so it never re-seeds. "No chats" means what the project row renders as Empty: no unpinned tab or mission whose parent is the project (see `render_project` in `sidebar/rows_render.rs`, `scope_rows`, `source_nodes`/`source_projects`). Pinned items do not count.
4. **Not a render rule.** Do not make "empty means collapsed" a rule evaluated while drawing: the setting stores only collapsed projects, so such a rule would leave no way to expand an empty project, for example to drag a chat into it. Accepted tradeoff: a project empty at first connect that later gets a chat on the host stays collapsed on this client; its header still shows the chat's status, and starting a chat in it from this client expands it (`reveal_started_scope`).
5. **Several windows on one host** each keep a working copy, every change saves, and a window that opens the host starts from the latest saved view. No live sync between open windows.

## Read first

- `crates/runner-app/src/surfaces/sidebar/model.rs`: `set_front_page_target`, `remote_overview`, `source_nodes`, `source_projects`.
- `crates/runner-app/src/surfaces/sidebar/state.rs`: `settings`, `update_app_settings`.
- `crates/runner-app/src/surfaces/remote_client/workspace/mod.rs`: where the remote `Sidebar` is created and `remember_remote_selection`; `remote_client/workspace/sync.rs`, which calls `set_front_page_target` on every overview sync (a natural point to seed once the overview arrives); `remote_client/workspace/layout.rs` `sidebar_nodes`.
- `crates/runner-app/src/app_settings.rs`: `AppSettings`, its `Default` and `normalize`.

## Deliverables

- A persisted per-host entry in `AppSettings` (for example `remoteSidebars`: computer id → the six fields above, with Projects and Chats defaulting open), loaded into `remote_settings` when the remote sidebar starts and written back through `AppStore::update_settings` on each persisted change. Older settings files without it load unchanged.
- Remove `RemoteSelection.sidebar_settings` and its restore if the persisted entry makes them redundant; keep restoring the selected tab, mission and layouts as today.
- The first-connect seed in decision 3.
- **Docs in the same PR:** the remote windows paragraph in `docs/guide/guides/remote-access.md` (it says a returning window "keeps that computer's ... collapsed projects"; it should say the view is remembered per host across windows and restarts, and that a new host opens with empty projects collapsed) and the matching paragraph in `docs/guide/zh-CN/guides/remote-access.md`. Update `REMOTE-WIN-05` in `docs/tests/regression/remote.md`, whose expected result ("first visits keep Show all/expanded defaults", "independently per window") encodes the old behavior.

**Non-goals:** syncing the view through the host or between client devices; cleaning up entries for forgotten hosts; any change to the local sidebar's defaults; any protocol, daemon or storage-schema change.

## Tests

In-process tests beside the existing remote sidebar tests in `remote_client/tests.rs`:

- The saved view survives a fresh app store loading the same settings file (the restart case), including collapsed link groups.
- First connect collapses exactly the empty projects (a project whose only chat is pinned counts as empty) and expands the others; a host with a saved view is not re-seeded, including a project the user expanded after the seed.
- Two hosts keep separate views; the local sidebar's settings stay unchanged.
- `switching_computers_restores_each_windows_remote_sidebar_view` asserts the old behavior (a second window starts at defaults; dropping the overview resets the view). Change those assertions deliberately and say so in the handoff. The base `overview()` fixture's project `p` holds only a pinned tab, so it becomes collapsed on first connect; fix any test that relied on it being expanded by expanding it explicitly, not by weakening the seed.

Gates: `runner-app` tests, workspace Clippy with warnings denied (with `--features updater` on macOS), `cargo fmt --all --check`. Drive macOS, Windows and Linux CI green.

## Live checks (qa)

Required, because the change alters what a person sees when opening a remote window:

1. **First connect.** On a client with no saved view for the host, open a host that has an empty project, a project whose only chat is pinned and a project with chats. The first two are collapsed, the third expanded, Projects and Chats open.
2. **Restart.** Collapse a project with chats, expand an empty one and collapse an Active group; quit and relaunch the client and open the host again. The same view returns.
3. **Second window.** Open the same host in a second window: it starts from the saved view.
4. **Isolation.** The local sidebar's collapse state is unchanged throughout (the `REMOTE-WIN-02` and `REMOTE-WIN-05` local-separation parts).

Use the same-Mac isolated host/client fixture from `docs/tests/1263-p1-route-robustness.md` (private homes, wrapper apps, real relay). Its two test enrollments and the pairing use the hosted account: ask Jason through `runner signal ask_human` before each enrollment and the pairing, and restore the account's computer list at cleanup as that record did. Take the QA lock first. Record results in `docs/tests/1272-remote-sidebar-state.md` with screenshot evidence; the coder folds it into the PR.

## Authorization

Jason approved the codex trio for this mission. The crew may commit, push this branch and open one PR against `main`.
