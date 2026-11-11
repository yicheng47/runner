# #1249 — Usage settings, and Skills and MCP inside each agent

Jason approved the codex trio for this mission on 2026-11-11. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1249-agent-integrations` on `feat/1249-agent-integrations`, based on `origin/main` at `c46354ef2993c562cfefa1d07c1d1da0532151bd`. The spec is [`docs/features/1249-agent-integrations.md`](../../features/1249-agent-integrations.md); it wins on behavior, and the signed-off design `runner/desktop/specs/1249-agent-integrations.pen` in `~/repos/yeelinks/yeelinks_design` wins on layout and copy. The spec lists the frame ids. Read the `.pen` only through the Pencil MCP tools; if they are unavailable, ask the operator for PNG exports of the frames you need.

## Problem

Settings › Integrations holds Agents, Skills and MCP. Skills and MCP are per-agent configuration with their own runtime dropdown, and they only read the main Claude Code account's folder. Usage has no settings: the sidebar badges, refresh interval and alerts are fixed. The change moves Skills and MCP into tabs on each agent's page with a Claude Code account dropdown, adds Settings › Usage, and adds a near-the-limit alert to Settings › Notifications.

## Read first

- `crates/runner-app/src/surfaces/settings/page/mod.rs`: `INTEGRATION_PANES`, `NAV_GROUPS`, `SettingsPane::from_route`, `filtered_nav_groups` and the nav search test.
- `crates/runner-app/src/surfaces/settings/agents/` (`render_agent_page`, `render_runtime_page`, `launch.rs`), `skills.rs`, `mcp.rs`, `claude_accounts.rs`, `notifications.rs`.
- `crates/runner-app/src/surfaces/app_shell/usage.rs` (`visible_usage_runtimes`, `sidebar_usage_element`, `render_usage_popover`, `render_claude_usage_picker`) and `app_shell/sidebar.rs` (the footer).
- `crates/runner-app/src/app_settings.rs` (`NotificationSettings`).
- `crates/runner-daemon/src/usage.rs` (`SCHEDULE_INTERVAL`, `OPEN_INTERVAL`, `set_enabled`), `crates/runner-core/src/protocol/api.rs` (`usage_set_enabled`, `usage_refresh`).
- `crates/runner-daemon/src/ops/skills.rs` (`skill_catalogs`, `set_global_enabled`), `ops/mcp.rs` (`mcp_catalog_with_env` and the edit/remove requests), `claude_accounts/sharing.rs` (`config_env`, `MAIN_FOLDER`, `MAIN_MCP`).

## Deliverables

The spec's Scope, phase by phase: agent page tabs with route and search redirects; the Claude Code account dropdown and account-scoped catalog, toggle and edit requests (an optional account id, none meaning the main account); the Usage page with its live footer preview, badge switches, check interval, 5-hour switch and Limits cards, and the popover's Settings actions opening it; the Notifications Usage group with the near-the-limit alert and the fetching rules; the guide pages the spec names, with their `zh-CN` copies. English and `zh-hans` strings for all new copy. Non-goals are in the spec.

## Validation and handoffs

The coder implements, adds the automated tests the spec's Verification lists, and runs the `runner-app`, `runner-daemon` and `runner-core` tests, workspace Clippy with warnings denied and `cargo fmt --all --check`, using this worktree's own target directory. The coder hands the candidate to the reviewer, who reviews the diff only.

After a clean review, `qa` runs the spec's five live checks in the development app on the reviewed commit, taking the qa lock first. For check 3, qa adds a second Claude Code account in the development app pointing at a fresh folder under `/tmp`, without signing in, and removes it at cleanup. qa restores any setting it changes (badge switches, 5-hour switch, threshold) and reports pass or fail per check with screenshots, recorded in the PR description.

## Authorization

The codex trio may commit, push this branch and open one PR against `main`, with one risk label and `needs-live-test` until qa passes.
