# 1341 — Anonymous active-install heartbeat

Tracking issue: [#1341](https://github.com/yicheng47/runner/issues/1341).

Design: `runner/desktop/specs/1341-install-heartbeat.pen` in the design repo (committed at `d483cea`, signed off 2026-11-26): `YmUMc` / `b2AFwu` / `Fdc0i` (`S3` dedicated **Settings → Privacy** pane under **System**, Dark, Runner Light and `DO_NOT_TRACK=1` override), `A6YyP` (`F1` leading checkbox above the First-run Setup footer) and `Rhp9k` (states, payload options and zero-UX-impact contract).

## Motivation

GitHub release counters do not measure active users: macOS `appcast.xml` counts every relaunch and 24-hour poll, Windows installer downloads are inflated by `winget` automation, and Linux counts are negligible. A minimal, transparent daily heartbeat gives unique active installs (DAU / WAU / 28-day), cohort retention, version adoption and OS/architecture mix without collecting any user content or personal data.

## Data collected (payload contract)

The desktop app sends one JSON `POST` to `https://runnersh.dev/api/heartbeat` at launch (and at most once per 24 hours while running).

### Top-level fields (stored as D1 columns)

All three packaging scripts (`script/bundle-mac`, `script/bundle-windows.ps1`, `script/bundle-linux`) already compile in `CARGO_PKG_VERSION`, `RUNNER_BUILD_STAMP` and `RUNNER_RELEASE_CHANNEL`, so the payload uses those exact values without a second naming scheme:

| Field | Type | Example | Validation / allowed values | Purpose |
| --- | --- | --- | --- | --- |
| `install_id` | string | `8f3c2d10-4b7a-4e21-9c83-6a1f9e04a91e` | Strict lowercase UUID v4 (36 chars) | Deduplicate daily pings and measure retention without hardware or account IDs |
| `version` | string | `0.20.4` | Semver from `CARGO_PKG_VERSION` (`^\d+\.\d+\.\d+([-.][0-9A-Za-z.]+)?$`, max 32 chars). On `nightly` and `preview-nightly` this is the base crate semver on that branch (e.g. `0.20.4`); `build_stamp` identifies the exact nightly cut. | Version distribution and upgrade lag |
| `build_stamp` | string | `20261126.0220` | `RUNNER_BUILD_STAMP` (`^[0-9]{8}\.[0-9]{4}$` or fallback `^[0-9A-Za-z._-]{1,32}$`) | Uniquely identifies and chronologically orders every `nightly` and `preview-nightly` cut sharing a semver |
| `channel` | string | `production` | Exact `RUNNER_RELEASE_CHANNEL` enum: `production`, `nightly`, `preview-nightly` | Separate production vs nightly active installs |
| `os` | string | `macos` | Enum: `macos`, `windows`, `linux` | Platform mix |
| `arch` | string | `aarch64` | Enum: `aarch64`, `x86_64` | Apple Silicon vs Intel / Linux arch mix |
| `remote_enabled` | boolean | `false` | Boolean (`true` if Remote access is enrolled/enabled on this host, else `false`; stored as `0`/`1`) | Track remote/mobile adoption directly in online queries |

### `meta` JSON object (coarse non-indexed signals)

Coarse evolving signals that do not need indexed online queries are sent under an optional `meta` JSON object and stored in nullable `meta TEXT`, so adding a flag later never requires a D1 schema migration. The Worker allowlists known keys, rejects malformed values and caps serialized `meta` at 512 bytes.

| Key inside `meta` | Type | Example | Validation / allowed values | Purpose |
| --- | --- | --- | --- | --- |
| `active_runtimes` | string[] | `["claude-code", "codex"]` | Array of unique allowlisted runtime keys (`claude-code`, `codex`, `antigravity`, `pi`, `grok`, `copilot`, `cursor`, `opencode`, `trae`), max 10 items; runtimes with at least one session in the last 7 days | Validate first-tier runtime priority against real usage |
| `used_mission_7d` | boolean | `true` | `true` if any multi-agent mission ran in the last 7 days, else `false` | Measure crew/mission adoption vs direct-chat/terminal use |

Example payload (also shown verbatim in **Settings → Privacy**):

```json
{
  "install_id": "8f3c2d10-4b7a-4e21-9c83-6a1f9e04a91e",
  "version": "0.20.4",
  "build_stamp": "20261126.0220",
  "channel": "production",
  "os": "macos",
  "arch": "aarch64",
  "remote_enabled": false,
  "meta": {
    "active_runtimes": ["claude-code", "codex"],
    "used_mission_7d": true
  }
}
```

### Edge-derived column (`country`)

The Worker reads Cloudflare's edge `request.cf?.country` (validated 2-letter uppercase ISO 3166-1 alpha-2 code, e.g. `CN`, `US`, `JP`, else `NULL`) and stores it in `country TEXT` on both tables while still discarding the client IP. This gives a mainland-China vs overseas active-install split without ever logging or storing IP addresses, and is disclosed in **Settings → Privacy** and the privacy guide alongside the JSON fields.

### Never collected

- IP addresses (the Worker never reads, logs or stores `CF-Connecting-IP` or `X-Forwarded-For`, and does not store `User-Agent`).
- Prompts, messages, terminal output, code, diffs, file paths, repository names, role/crew prompts or working directories.
- Runner Hub account IDs, emails, agent credentials, hostnames, usernames or hardware/OS serial numbers.
- Sub-day timestamps (D1 stores UTC `YYYY-MM-DD` calendar dates only, avoiding timing correlation).

## Cloudflare Worker & D1 schema (`runner-web`)

The endpoint lives in `runner-web` alongside `/api/waitlist` (`worker/index.ts`, already routed via `"run_worker_first": ["/api/*"]`) using the existing Cloudflare D1 binding `DB` (`migrations/0003_heartbeat.sql`). No new Cloudflare Worker or database is required.

### D1 tables (`migrations/0003_heartbeat.sql`)

Stable, indexed dimensions live in typed columns; evolving non-indexed metadata lives in nullable `meta TEXT` (JSON):

```sql
CREATE TABLE installs (
  install_id TEXT PRIMARY KEY,
  first_seen_day TEXT NOT NULL,
  last_seen_day TEXT NOT NULL,
  version TEXT NOT NULL,
  build_stamp TEXT NOT NULL,
  channel TEXT NOT NULL,
  os TEXT NOT NULL,
  arch TEXT NOT NULL,
  country TEXT,
  remote_enabled INTEGER NOT NULL DEFAULT 0,
  meta TEXT
);

CREATE INDEX idx_installs_last_seen ON installs (last_seen_day);
CREATE INDEX idx_installs_first_seen ON installs (first_seen_day);

CREATE TABLE daily_pings (
  day TEXT NOT NULL,
  install_id TEXT NOT NULL,
  version TEXT NOT NULL,
  build_stamp TEXT NOT NULL,
  channel TEXT NOT NULL,
  os TEXT NOT NULL,
  arch TEXT NOT NULL,
  country TEXT,
  remote_enabled INTEGER NOT NULL DEFAULT 0,
  meta TEXT,
  PRIMARY KEY (day, install_id)
);

CREATE INDEX idx_daily_pings_day_dims ON daily_pings (day, os, arch, channel, version);
CREATE INDEX idx_daily_pings_day_country ON daily_pings (day, country);
```

### Worker endpoint (`POST /api/heartbeat`)

- Accepts `POST` with `Content-Type: application/json` and body size `<= 2048` bytes; returns `405` for other methods, `413` for oversized bodies and `400` for invalid JSON or out-of-range fields.
- Computes `day = new Date().toISOString().slice(0, 10)` (UTC `YYYY-MM-DD`) on the server; ignores client clocks.
- Upserts both tables in one D1 batch: `installs` preserves `first_seen_day` and updates `last_seen_day` + latest metadata; `daily_pings` uses `PRIMARY KEY (day, install_id)` with `ON CONFLICT(day, install_id) DO UPDATE`, capping storage at one row per install per UTC day even on retries.
- Returns `204 No Content` with `Cache-Control: no-store`. On D1 errors, returns `503` (which the desktop client ignores silently).
- Retention: `daily_pings` rows older than 180 days can be pruned periodically (`DELETE FROM daily_pings WHERE day < date('now', '-180 days')`); `installs` remains tiny (one row per install).

### Reporting queries (`wrangler d1 execute`)

```sql
-- DAU / WAU / 28-day active installs
SELECT
  (SELECT COUNT(*) FROM daily_pings WHERE day = date('now')) AS dau,
  (SELECT COUNT(DISTINCT install_id) FROM daily_pings WHERE day >= date('now', '-6 days')) AS wau,
  (SELECT COUNT(DISTINCT install_id) FROM daily_pings WHERE day >= date('now', '-27 days')) AS mau_28d;

-- 7-day active installs by platform, arch and channel
SELECT os, arch, channel, COUNT(DISTINCT install_id) AS active_7d
FROM daily_pings
WHERE day >= date('now', '-6 days')
GROUP BY os, arch, channel
ORDER BY active_7d DESC;

-- Version distribution among 7-day active installs (separates nightly cuts by build_stamp)
SELECT
  channel,
  CASE
    WHEN channel = 'production' THEN version
    ELSE version || '+' || build_stamp
  END AS build,
  COUNT(*) AS installs
FROM installs
WHERE last_seen_day >= date('now', '-6 days')
GROUP BY channel, build
ORDER BY installs DESC;

-- 7-day active installs by country (derived from Cloudflare edge header, no IP stored)
SELECT COALESCE(country, 'unknown') AS country, COUNT(DISTINCT install_id) AS active_7d
FROM daily_pings
WHERE day >= date('now', '-6 days')
GROUP BY country
ORDER BY active_7d DESC;

-- 7-day retention by install cohort day
SELECT
  first_seen_day,
  COUNT(*) AS new_installs,
  SUM(CASE WHEN last_seen_day >= date(first_seen_day, '+7 days') THEN 1 ELSE 0 END) AS retained_d7
FROM installs
WHERE first_seen_day <= date('now', '-7 days')
GROUP BY first_seen_day
ORDER BY first_seen_day DESC
LIMIT 30;

-- 7-day active installs with remote enabled + ad-hoc offline inspection of meta JSON
SELECT
  COUNT(*) AS active_7d,
  SUM(remote_enabled) AS remote_users_7d,
  SUM(CASE WHEN json_extract(meta, '$.used_mission_7d') = 1 THEN 1 ELSE 0 END) AS mission_users_7d
FROM installs
WHERE last_seen_day >= date('now', '-6 days');

-- 7-day active runtimes (offline ad-hoc via json_each on meta)
SELECT
  j.value AS runtime,
  COUNT(DISTINCT i.install_id) AS installs_7d
FROM installs i, json_each(i.meta, '$.active_runtimes') j
WHERE i.last_seen_day >= date('now', '-6 days') AND i.meta IS NOT NULL
GROUP BY j.value
ORDER BY installs_7d DESC;
```

## Zero-UX-impact guarantee (client contract)

Heartbeat delivery must never degrade, slow down or interrupt any user workflow:

- **Off the UI thread:** `runner-daemon` owns the state (`_app_state` keys for `telemetry_enabled`, `telemetry_install_id`, `telemetry_last_sent_at`) and sends the request on a detached background worker reusing `runner-daemon`'s existing proxy-aware `reqwest` client (works on macOS, Windows and Linux with no new HTTP dependency in `runner-app`).
- **Hard short timeout:** 5-second total request timeout (connect + TLS + response). No retry loop, no backoff timer storm, no blocking on app launch, Settings interaction, session spawn or quit.
- **Silent failure:** DNS errors, TLS errors, blocked routes, proxy errors, timeouts and non-2xx responses are dropped silently (logged at `debug` level only). No toast, no error banner, no sidebar badge and no retry inside the same launch; the next launch tries again if 24 hours have passed since the last successful `2xx`.
- **Completely separate from updates:** Sparkle / GitHub release update checks never depend on or wait for the heartbeat endpoint.
- **Strict gating:**
  - Unstamped / development builds (`RUNNER_BUILD_STAMP` absent or debug build) never send a ping and show the dev-build state in **Settings → Privacy**.
  - `DO_NOT_TRACK=1` (or `RUNNER_TELEMETRY=0`) in the environment overrides the stored setting to off and locks the toggle with an amber notice.
  - Default is ON for stamped builds. Toggling OFF immediately clears `telemetry_install_id` and stops future pings. **Reset ID** replaces `telemetry_install_id` with a fresh UUID v4.
  - On first launch while First-run Setup is visible, wait until Setup completes (or is skipped) before sending the first ping so a user who unchecks the Setup checkbox never sends an initial ping.

## Desktop UI (`runner-app`)

- **Settings → Privacy (`S3`):** New `SettingsPane::Privacy` (Lucide `shield` icon) placed first in `SYSTEM_PANES` (`Privacy`, `Updates`, `About`), searchable in Settings search.
  - **Telemetry card:** `Anonymous install heartbeat` row with opt-out toggle (default on), disclosure copy, `Privacy guide ↗` link and `Honors DO_NOT_TRACK=1` caption (or amber `DO_NOT_TRACK=1` override / dev-build notice); inline read-only **Payload preview** showing the exact JSON this install sends to `POST https://runnersh.dev/api/heartbeat`; `Install ID` row showing the local UUID (or cleared message when off) with a secondary **Reset ID** button.
  - **What stays on your machine card:** Two factual rows stating that prompts, code, terminal output and paths stay local, and that heartbeats log no IPs and never link to accounts.
- **First-run Setup (`F1`):** Leading 16×16 checked checkbox (`Anonymous install heartbeat`, `What's sent ↗`, one-line disclosure) in a hairline-separated strip above the Setup footer. Unchecking turns `telemetry_enabled` off before any ping is sent.

## Guide pages

- Add `docs/guide/guides/privacy.md` and `docs/guide/zh-CN/guides/privacy.md` (indexed in `docs/guide/README.md` and `docs/guide/zh-CN/README.md`) documenting the exact JSON fields, D1 retention, `DO_NOT_TRACK=1` override, **Settings → Privacy** and **Reset ID**. Link the **Privacy guide** / **What's sent** links to `https://runnersh.dev/docs/guides/privacy/` (and `/zh-CN/docs/guides/privacy/` for Simplified Chinese).

## Verification

### Automated

- `runner-web`: Worker unit tests covering valid payloads with and without `meta`, `remote_enabled` and `request.cf.country` persistence, `PRIMARY KEY (day, install_id)` deduplication, `first_seen_day` preservation on subsequent days, invalid UUID/channel/OS/arch/country/`meta` rejections (`400`), oversized body or `meta` > 512 bytes (`413`/`400`), wrong method (`405`), D1 error (`503`) and verifying IP/User-Agent are never persisted.
- `runner-daemon`: unit tests for gating (unstamped dev build never sends, `DO_NOT_TRACK=1` and `RUNNER_TELEMETRY=0` lock off, opt-out clears `install_id`, `Reset ID` rotates UUID, 24-hour throttle after 2xx, silent drop on timeout/500/unreachable local mock server without panicking or delaying shutdown, deferred first ping until onboarding finishes).
- `runner-app`: UI tests for **Settings → Privacy** (toggle on/off, live JSON preview updates, `Reset ID`, `DO_NOT_TRACK=1` locked state, dev-build state, Settings search finds Privacy) and First-run Setup checkbox (default checked, unchecking persists off before first ping).
- Gates: `make test`, `make clippy`, `cargo fmt --all -- --check`, `python3 script/check-guide-translations.py`, plus a 20-run nextest stress loop for any background-task test.

### Live checks

On the macOS development app, by `qa` holding `script/qa-lock`:

1. **Settings → Privacy (dark & light):** Open Settings → Privacy from the System group and via Settings search; verify toggle, live JSON preview, `Install ID` + **Reset ID**, `Privacy guide ↗` link, `DO_NOT_TRACK=1` locked state and unstamped dev-build notice. Reason: new Settings pane and controls.
2. **First-run Setup checkbox:** Open Setup (fresh dev data or reopened); verify the leading checked checkbox above the footer fits without clipping and toggling it syncs with **Settings → Privacy**. Reason: Setup layout changed.
3. **Zero-UX-impact check:** Point the test endpoint override at an unreachable blackhole / refused port and launch Runner; verify launch, pane switching and quit remain instantaneous with no error toast or banner. Reason: explicit failure-isolation requirement.
