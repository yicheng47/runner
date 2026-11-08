# #1221 — Resume Claude chats from the folder Claude actually uses

Jason asked for a mission to fix [#1221](https://github.com/yicheng47/runner/issues/1221) on 2026-11-08; this is the default claude trio. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1221-claude-config-dir-resume` on `fix/1221-claude-config-dir-resume`, based on `origin/main` at `449f3dbbdb57d6135dfc5098725fbfaa51fb1726`.

## Problem

Before resuming a Claude chat, Runner checks that its transcript exists. With one Claude account and no recorded account, `launch_account` returns `None`, so the check falls to `claude_code_conversation_exists`, which always looks in `~/.claude` and ignores `CLAUDE_CONFIG_DIR`. When the variable points elsewhere, Claude's transcripts are under that folder and the check reports the conversation missing. Automatic resume refuses the chat, and a manual Resume starts a new `--session-id` and overwrites the saved key, so the chat reopens blank.

The defect is confirmed from the code but has not been reproduced live. A Windows report may be this bug; the reporter has not confirmed it yet. The issue has the full trace.

## Outcome

The transcript check looks in the same folder Claude launches with. A Claude chat whose transcript exists resumes that conversation after a clean stop, an external kill or a restart. A chat whose transcript really is gone still starts fresh, as it does today.

## Read first

- `crates/runner-daemon/src/runtimes/claude_code/mod.rs`: `claude_code_conversation_exists`, `ClaudeCode::conversation_exists`.
- `crates/runner-daemon/src/runtimes/helpers.rs`: `conversation_file_exists`, its test home.
- `crates/runner-daemon/src/runtimes/mod.rs`: `ProbeContext`, `probe_uses_shell_env`.
- `crates/runner-daemon/src/session/manager/output.rs`: `conversation_exists`, `probe_env`.
- `crates/runner-daemon/src/session/manager/resume.rs`: `resume_with_fresh_fallback`; `fork.rs` for the fork check.
- `crates/runner-daemon/src/claude_accounts.rs`: `config_pin`, `launch_account`.
- `crates/runner-daemon/src/runtimes/claude_code/trust.rs`: `seed_project_trust`, `config_path`, which already honor the variable.
- `crates/runner-daemon/src/session/manager/spawn_spec.rs`: `agent_env`; `crates/runner-daemon/src/session/pty_runtime.rs` for how it is applied.
- `portable-pty` 0.9.0 `src/cmdbuilder.rs`: `get_base_env`.

## Deliverables

1. **One answer for which config folder.** Add one resolver for the `CLAUDE_CONFIG_DIR` the agent process will actually see, in the order spawn applies it: the spawn environment (shell variables, then role environment) over the base environment `portable-pty` gives the child, which on Windows includes system and user variables from the registry. Treat an empty value as unset and resolve a relative value against the cwd, as `trust.rs` does.
2. **The check.** `ClaudeCode::conversation_exists` uses that folder and falls back to `~/.claude`. Enable `probe_uses_shell_env` for Claude so the context carries shell variables. The resume, early-exit and fork checks all go through `output::conversation_exists`; confirm each one gets the fix.
3. **Inherited lookups.** `config_pin`'s inherited lookup uses the resolver, so a registry variable set after `runnerd` started still pins a multi-account chat on Windows. Swap the inherited lookup in `trust.rs` too if it is a direct replacement.
4. **Log the fallback.** Warn when a Resume starts fresh because the conversation is missing, naming the session, the key and the folder checked. Today it looks the same in `runnerd.log` as a success.
5. **Comment.** Correct the comment in `pty_runtime.rs` claiming `CommandBuilder` clears the inherited environment.

No guide change is needed: `docs/guide/concepts/sessions.md` already says Runner starts fresh only when the saved conversation is gone.

**Non-goals:**
- No change to whether Resume may replace a conversation without asking. That is a separate product decision.
- No account chip or other account behavior for one-account setups.
- No other runtimes and no change to the project-folder encoder.
- No repair of chats whose key was already overwritten; `/resume` inside the chat reaches those.

## Tests

- **Check:** with a fake home and the transcript under a custom folder, the check finds it when the variable is in the role environment, in the shell variables and in an injected base environment. Role beats shell. An empty value and an unset variable both use `~/.claude`. A transcript in neither place is missing.
- **Resume:** with one account, the variable in the role environment and the transcript there, Resume passes `--resume <key>` and keeps the key. With the transcript absent, it falls back as today and logs the warning.
- **Regression:** the resume test must fail without the fix. Say so in the handoff.
- **Isolation:** tests must not read the developer's real `CLAUDE_CONFIG_DIR` (#1159); this crew runs with it set. Never write to the real Windows registry; inject the base environment.
- **Gates:** `runner-daemon` tests, workspace Clippy with warnings denied and `cargo fmt --all --check`. Drive macOS and Windows CI green.

## Live checks (qa)

Use the macOS development app from this worktree, with an inexpensive model at low effort, tiny prompts and a fresh folder under `/tmp` for each chat. Take the QA lock as AGENTS.md describes.

Setup: the development app needs exactly one Claude account under **Settings → Agents → Claude Code → Accounts**. If it has more, back up the development database's `claude_accounts` state and restore it afterwards. Launch it with `CLAUDE_CONFIG_DIR` set to an already signed-in folder other than `~/.claude`. Do not sign in anew or create an account folder. Operate only chats you created.

- **L1, clean stop:** after one tiny turn, the transcript is at `<folder>/projects/<dir>/<key>.jsonl` and not under `~/.claude/projects`. Stop, then Resume: the history shows and `agent_session_key` is unchanged. Reason: the core defect.
- **L2, external kill (the report):** from another shell, kill only that chat's own Claude process. Resume: the history shows and the key is unchanged. Reason: the reported repro.
- **L3, restart:** with **Resume running agents on launch** on and a test chat running, quit and reopen the development app. The chat resumes automatically with its history. Reason: automatic resume refused these chats.
- **L4, genuinely missing:** move that chat's transcript aside and Resume: it starts fresh and `runnerd.log` has the new warning. Restore the file. Reason: the fallback and its log.
- **L5, no variable:** launch without the variable; Stop then Resume keeps the history. Mark it blocked if `~/.claude` is not signed in. Reason: the default path shares the changed check.

**W1, Windows (not for qa):** with the variable in the user environment and one account, kill the Claude process and Resume: the history shows. Jason or the reporter runs this, so the PR keeps `needs-live-test` until it passes.

The verdict gives pass or fail per check, with screenshot evidence, and the PR description records it.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. When the review is clean, the coder hands the reviewed commit to `qa` with L1–L5. Any failure, and any change after review, goes back through `reviewer` before qa reruns the affected checks.

The PR uses `Fixes #1221` and says the Windows reporter's case is still unconfirmed. It carries `risk-medium` and `needs-live-test`; `core-review` is added automatically.

## Authorization

The claude trio may commit, push this branch and open one PR against `main`. It does not merge.
