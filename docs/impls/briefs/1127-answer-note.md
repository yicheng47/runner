# #1127 — `mission answer` carries a free-text note

Jason approved this mission on 2026-11-07 with a claude duo. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1127-answer-note` on `feat/1127-answer-note`, based on `origin/main` at `1db23e604ab24d93b02b9afe8db669bac8a56503`. Issue #1127 is the authority on scope; this brief adds what reading the code settled.

## Problem

`runner mission answer <mission> <question-id> <choice>` can only send one of the question's choices. A real answer often needs a condition the choices don't carry, so today it goes in a separate `runner msg post`, and the asker has to join two messages to learn what was decided.

## Read first

- `crates/runner-cli/src/command/cli.rs`: `MissionCommand::Answer`, and `AddRole`'s `task`/`task_file` for the flag pattern.
- `crates/runner-cli/src/command/mission.rs`: the `Answer` arm. `read_text` in `command/shared.rs` already reads an inline value, a file, or `-` for stdin.
- `crates/runner-daemon/src/router/handlers.rs`: `append_answer` builds the delivered message. The live handler and the recovery path in `router/mod.rs` both call it, so a change there covers both.
- `crates/runner-daemon/src/ops/mission.rs`: `mission_signal_impl` passes the `human_response` payload through, resolving only `question_id`.
- `crates/runner-cli/src/help.rs` and `crates/runner-core/src/protocol/agent_skill.rs`: the agent guide and the generated skill.

## Deliverables

1. **CLI.** `runner mission answer` takes an optional `--note <text>` or `--note-file <path|->`, which conflict with each other. Trim surrounding whitespace, because a note read from stdin usually ends in a newline. Refuse a blank note as a usage error, so a note the caller meant to send is never dropped silently.
2. **Signal.** `human_response` carries an optional `note` string. Without a note the payload is exactly what it is today.
3. **Delivered message.** With a note, the asker's inbox message reads `Answer to "<prompt, first 80 characters>": <choice> — <note>`. A signal with no `note`, or one that is not a string or is blank, produces today's text byte for byte, so older clients keep working.
4. **Guidance and docs.** `runner help agents` and both generated skill variants mention `--note` for answers that need a condition. Update `docs/guide/guides/drive-runner-from-agents.md` and its `docs/guide/zh-CN/` page, and regenerate the CLI reference with `make guide-cli`.

## Boundaries

- No app change. The issue rules out a note field on the question card. The feed hides the delivered answer message (`append_feed_block` in `surfaces/mission_feed.rs` skips a message with `question_id`), and the resolved card shows only the choice, so a note is not visible in the app's feed. Leave that as it is and state the limitation in the PR description.
- The remote and companion answer paths (`AnswerRequest`) stay choice-only.
- No limit on note length beyond what the event log already applies, no new signal type, and no change to `resolve_pending_question`.

## Validation

Name the changed behaviors and the check covering each before running them. Selected checks:

- **CLI:** tests in `crates/runner-cli/src/command.rs` for the flags, their conflict, `--note-file -`, trimming and the blank-note refusal; the signal payload with a note, and unchanged without one. Update `crates/runner-cli/tests/goldens/cli.txt` intentionally through its existing workflow.
- **Router:** tests in `crates/runner-daemon/src/router/tests/mod.rs` for the message with a note, byte-identical text without one, a blank or non-string `note`, and the recovery path through `router/mod.rs`.
- **Guidance:** the skill-rendering assertions in `agent_skill.rs`, `make guide-cli-check` and `script/check-guide-translations.py`.
- **Gates:** `cargo nextest run --locked -p runner-cli -p runner-core -p runner-daemon --cargo-profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`, using this worktree's own `target/`. Then CI on every required platform.

**Live checks:** none expected. The change alters nothing a person sees or operates in the running app, and the CLI and router tests exercise it in process. The reviewer confirms this from the diff. If the clean review does name a live check, the mission's operator adds the codex `qa` role after the mission settles; the coder and reviewer do not run the app.

## Authorization and handoff

The approved crew is this claude duo. The crew may commit, push this branch and open one PR against `main`.

The coder implements, opens the PR and hands the reviewer the PR number and head commit. The reviewer posts its result as a PR comment. A clean review adds `reviewed`, names the reviewed commit, and says which live checks are needed or why none are. Touching `router/` brings `core-review`, so Jason reviews this PR closely before it merges.

Finish by posting the PR URL, final head, what changed, validation results and the live-check decision with `runner msg post`.
