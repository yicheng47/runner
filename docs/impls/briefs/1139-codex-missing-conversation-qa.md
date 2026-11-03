# #1139 — QA: Codex missing conversation

Live checks for the fix to [#1139](https://github.com/yicheng47/runner/issues/1139) on branch `fix/1139-codex-missing-conversation`, worktree `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1139-codex-missing-conversation`. Test the branch head at mission start; the pull request carries the code review and CI. macOS only; Windows is a recorded gap.

## What changed

- Codex now answers Runner's conversation probe. A key counts as present when `$CODEX_HOME/sessions/**/rollout-*-<key>.jsonl` or `.jsonl.zst` exists, or the rollout is in `archived_sessions/`. Before, the probe never answered, so a key with no rollout ran `codex resume <key>` on every Resume, which failed with `No saved session found` and showed "The codex agent exited while resuming its conversation; Resume will try it again."
- On a confirmed miss, Runner looks for the chat's original rollout: the one whose opening prompt carries the chat's capture marker and that was still being written after the missing key was minted. If it finds exactly one, it resumes that conversation and saves its key. Otherwise a manual Resume starts a fresh conversation, and resume-on-launch reports the conversation unavailable.
- Read first: `crates/runner-daemon/src/runtimes/codex/conversation.rs` and the `recovered_key` block in `crates/runner-daemon/src/session/manager/resume.rs`.

## Checks

Run each in the development app built from the worktree. Start each check in a new Codex direct chat with an inexpensive model at low effort, in a fresh folder under `/tmp`. Read a chat's key with the candidate CLI's `session show <id> --json` (`agent_session_key`); its rollout is `~/.codex/sessions/YYYY/MM/DD/rollout-*-<key>.jsonl`. Moving or compressing the rollouts of the test chats you create is authorized for these checks. Restore each one afterwards, and touch no other file under `~/.codex`.

1. **Missing conversation starts fresh once.** Send "Reply with the word apple.", wait for the reply, then Stop. Move the chat's rollout into your scratch directory and press Resume. To pass, a new Codex conversation starts at once, without the "exited while resuming" banner or `No saved session found`, and the key changes. Stop and Resume again: the new conversation continues. Move the rollout back.
2. **Replaced key resumes the original conversation.** Send "Remember the word banana. Reply OK.", wait, then Stop. You must not open Runner's database. Post the session ID to the human through Runner and wait: the human replaces the key with one that names no rollout and replies with both keys. Press Resume. To pass, the chat resumes its original conversation: asked "Which word did I ask you to remember?", it answers banana. `session show` must give the original key again, and no "exited while resuming" banner may appear.
3. **Compressed rollout still resumes.** Send "Remember the word cherry. Reply OK.", wait, then Stop, and press Resume: the conversation continues (regression check on an intact rollout). Stop again, run `zstd -q --rm` on its rollout so only `.jsonl.zst` remains, as Codex does to older rollouts, and press Resume. To pass, the key is unchanged and the chat answers cherry when asked which word it was told to remember. Decompress the rollout back afterwards with `zstd -d -q --rm`. If Codex itself cannot open a rollout compressed this way, mark the check Blocked with its output rather than Failed.

## Record and handoff

Write the result record to `docs/tests/1139-codex-missing-conversation.md` in the worktree: candidate SHA and build identity, method, a result matrix for the three checks, evidence paths and cleanup. Leave it uncommitted and report the verdict to the human through Runner.

## Authorization

One `qa` session (Codex `gpt-6.1-sol`, high) runs these checks under the standing development-app authorization. No commits, pushes or pull requests. The human performs the key replacement in check 2.
