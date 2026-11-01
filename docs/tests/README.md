# Validation docs

Use these documents to select checks and preserve their evidence. [Test Scope](../../AGENTS.md#test-scope) governs ordinary changes: name the changed behavior, choose checks that cover it, and explain the connection before running them. A documentation sweep does not run the app or establish a new product pass.

| Location | Purpose |
| --- | --- |
| [regression/README.md](regression/README.md) | Maintained live cases for the current product, with stable IDs, applicability and evidence requirements. |
| [full-smoke-test.md](full-smoke-test.md) | Execution procedure: candidate identity, development-app isolation, native control, bounded fixtures and cleanup. |
| [runs/](runs/) | Dated results for a particular candidate, platform and attempt. New records use `YYYY-MM-DD-<platform>.md`; existing filenames remain stable. |
| [archive/](archive/) | Historical feature validation, investigations and earlier smoke records, including failed and unverified checks. |
| This directory | Feature-specific validation plans or records while their work is active or awaiting a release sweep. |

For an ordinary feature, select relevant regression cases and add feature-specific checks. Use smoke-tier rows for an explicitly requested release smoke and all active rows for an explicitly requested complete regression or a major refactor requiring that coverage. Reusing the execution procedure does not expand a feature's test scope.

Write new run results separately from maintained cases. Record commands and exit codes, candidate identity, selected case instances, observations, evidence and gaps. Preserve failures and later retries; a source review, CLI probe, CI pass or historical user confirmation does not establish a current native pass.

At a release sweep, move shipped feature records to `archive/`, promote lasting current live checks into `regression/` under its [promotion rules](regression/README.md#promotion-and-maintenance), and repair references. Automated-only assertions and one-time investigations stay with their owning tests or records. Retain validation evidence even when a mission brief or obsolete proposal is pruned.

The 0.18.3 sweep archived the shipped [Linux platform](archive/823-linux-support.md), [updater](archive/823-linux-updates.md), [sign-in](archive/823-linux-remote-sign-in.md), [review fixes](archive/1027-linux-review-findings.md) and [local handoff](archive/823-linux-delivery-history.md), plus [program status](archive/861-program-status.md), [mission lead removal](archive/873-remove-mission-lead.md), [Codex input attention](archive/codex-input-attention.md), [project selection](archive/968-project-selector.md), [router delivery](archive/1075-router-own-notice-hold.md) and [CI speed](archive/1023-ci-speed.md). Historical failures, waivers and unverified native coverage remain in those records.

The 0.19.0 sweep archived [pane islands](archive/1086-pane-islands.md), [the launcher](archive/1092-chat-terminal-launcher.md) and [idle inbox delivery](archive/1046-idle-inbox-delivery.md), plus the closed #977 cleanup's [app layout](archive/977-app-layout.md), [remote client owner](archive/977-remote-root.md) and [workspace client](archive/977-workspace-client.md) records. Upgrade, status-fixture, Windows and Linux gaps remain open in those records; Jason's manual drag and scrollbar checks are attributed to him, not to QA.

The 0.19.1 sweep archived [the mission launcher](archive/1097-mission-launcher-qa.md) and [submit recovery and mid-turn delivery](archive/1102-submit-without-turn.md). The launcher's remote check stayed Blocked without a second host, and the delivery record keeps its masked first Codex busy run beside the isolated rerun that replaced it.
