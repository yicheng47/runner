# Mission briefs

A brief gives a mission its working directory, problem, deliverables, validation and authorization. Write it on the mission's branch before launch, then fold it into an implementation commit when the work lands. Keep three examples of different sizes; other completed briefs live in git history. Active missions keep their briefs until their work is finished.

## Examples by size

These are real briefs, with their original dates, paths and task-specific permissions. Copy their structure and replace the details. They do not authorize a new mission or override the current [`AGENTS.md`](../../../AGENTS.md).

| Size | Brief | What to take from it |
|---|---|---|
| Small | [Mission inbox guidance](mission-inbox-guidance.md) | One coder, one focused change, named authoritative files, bounded checks and an open-PR handoff. Merged in [#985](https://github.com/yicheng47/runner/pull/985). |
| Medium | [#735 — Start a chat modal](735-start-chat-modal.md) | A designed UI feature: spec and PNG references, concrete states and request mappings, coder/reviewer handoffs and native-check gaps. Shipped in 0.12.5. |
| Large | [#823 — Linux support](823-linux.md) | A cross-platform change: ordered deliverables across runtime, UI, packaging and CI, review risks, and a separate live-test record. Merged to `main` in [#1027](https://github.com/yicheng47/runner/pull/1027), which replaced #982, and shipped as a beta in 0.18.2; #823 closed after the beta shipped, with unverified native coverage retained in the validation records. |

Size describes the work, not a required agent count or brief length. Small fixes normally stay inline; launch a mission only when Jason requests or approves one. Large programs may need several independently reviewable missions rather than one long goal. Keep each launch goal under 8 KB and link to its spec or implementation plan for detail. The historical medium and large files exceed that launch limit: use them as supporting references and summarize the goal instead of passing the whole file unchanged.

## Writing the next brief

1. **Starting point.** Name the issue or existing PR, branch, exact worktree and base revision. The root checkout stays on `main`; every worktree uses its own Cargo target directory.
2. **Problem and outcome.** Describe the trigger, current behavior and intended result. Name the spec and design in `yeelinks_design` when applicable, and say which source wins on detail.
3. **Read first.** Point to the few source files and docs needed to begin. Prefer symbols over line numbers that will drift.
4. **Deliverables and boundaries.** State observable acceptance criteria, affected guide pages, non-goals and any approved probes or fixtures. Avoid repeating the repo's rulebook.
5. **Validation and handoffs.** Tie each selected check to changed behavior under [Test Scope](../../../AGENTS.md#test-scope). State who implements, reviews and runs live checks, and what evidence the final handoff carries.
6. **Authorization.** State only what is specific to this mission: the approved crew and agent count, whether it may commit, push and open a PR, and any exception to the [Crew Missions](../../../AGENTS.md#crew-missions) rules. Do not restate those rules, such as ending at an open PR with CI green or folding review fixes into their commits: a crew follows them by default. One or two sentences is usually enough.

Use the current [Crew Missions](../../../AGENTS.md#crew-missions) workflow. In particular, the old blanket “crews never run the dev app” in the medium and large examples is historical: coders and reviewers do not run it, while the `qa` role added after review, or whoever else is assigned selected live checks, may use the isolated development environment under [Test Scope](../../../AGENTS.md#test-scope). Use current crate names and verification commands. Rebase only when the repo's current rules require it.

Inside a mission, participants finish their turn when waiting for a teammate and read the inbox when Runner notifies them. An outside operator starting a different mission follows the [external watch workflow](../../guide/guides/drive-runner-from-agents.md#start-and-follow-a-crew).

## Retention

At a docs sweep, prune completed briefs except these three examples. Keep a brief for active work even when it is not a sample. Before pruning, preserve unique current decisions in architecture docs, outstanding scope in its issue, and validation evidence in `docs/tests/`; fix references to use the owning document or a fixed git revision. Do not remove test evidence or implementation logs with a brief.

The 2026-10-22 sweep reduced ten briefs to these three. #823 remains both the large example and an active brief; the requirements in its open PR may have evolved beyond the original launch goal.

The 2026-10-24 release sweep pruned the completed #929, #1006 and #1007 briefs. The three examples remain; #823 is still open, and the running #836 mission keeps its brief in its own worktree.

The 0.18.0 release sweep pruned #836's 17 briefs, from Phase 1's `836-connect-host.md` to the landing review; the [program record](../archive/836-web-remote-control/README.md) links each at a fixed revision. The three examples remain, and #823 is still open.

The 0.18.1 release sweep pruned the completed #1031 and terminal-redraw-test-settle briefs. Their validation records remain in [the test archive](../../tests/README.md), including the redraw brief’s initial investigation and unresolved observations. The three examples remain; #823 is still open.

The 0.18.2 release sweep pruned three merged Linux briefs: the original local support mission, the Linux remote sign-in fix ([#1036](https://github.com/yicheng47/runner/pull/1036)) and the #1027 review findings ([#1040](https://github.com/yicheng47/runner/pull/1040)). Their test records stay in `docs/tests/` while #823 is open, and the Linux spec links the local brief at a [fixed revision](https://github.com/yicheng47/runner/blob/49e03dd419ab726383f6b8a96d693d85274173b4/docs/impls/briefs/823-linux-support-local.md). The three examples remain.

The 0.18.3 release sweep pruned 14 merged mission briefs, covering #873, #861, #968, #1047, #1075, Codex input attention, the Chinese guide, CI speed and the six merged #977 cleanup missions. The three examples remain; #977's plans and test records stay while that issue is open. Linux's closed #823 records are archived without turning native gaps into passes.

The 0.19.0 release sweep pruned five merged mission briefs: #1044, #1046, #1083, #1086 and #1092. Their validation records are in [the test archive](../../tests/README.md); #1083 had none of its own. The three examples remain.

The 0.19.1 release sweep pruned the #1097 mission launcher and #1102/#1103 submit-recovery briefs. Their validation records are in [the test archive](../../tests/README.md). The three examples remain.

The 0.19.2 release sweep pruned the #897 find in terminal, #984 usage, #1122 paired phone and #1125 confirmed-notice briefs. The #897 and #1125 validation records are in [the test archive](../../tests/README.md); #984 and #1122 had none of their own. The #1025 remote repaint brief stays while that issue is open. The three examples remain.

The 0.19.3 release sweep pruned the #748 mission link, #931 installed fonts, #1067 Claude trust seed, #1149 session CLI and #1155 role permission flags briefs. The #748 and #1149 validation records are in [the test archive](../../tests/README.md), and #1067's run is in [the 2026-11-04 macOS run](../../tests/runs/2026-11-04-macos.md); #931 and #1155 had none of their own. The #1025 remote repaint brief stays while that issue is open, and the #1160 brief lives in its mission's worktree. The three examples remain.

The 0.19.4 release sweep pruned 17 merged mission briefs: #1025, #1118, #1139, #1151, #1159, #1160, #1163, #1167, #1172, #1177, #1178, #1184, #1187, #1188, #899, #930 and the preview nightly channel (#1166). Their validation records are in [the test archive](../../tests/README.md) or in [the runs](../../tests/runs/); #1118, #1151, #1159, #1167, #1178, #1188 and #1166 had none of their own. The three examples remain.
