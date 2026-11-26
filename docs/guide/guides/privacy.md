# Privacy and telemetry

Runner never sends your prompts, code, or session transcripts in telemetry, and sends at most one anonymous daily heartbeat so maintainers can count active installs by version, operating system, architecture, and coarse feature use.

## What Runner sends

Signed release and nightly builds send one HTTPS `POST` request to `https://runnersh.dev/api/heartbeat` at most once every 24 hours. Unstamped local development builds never send heartbeats.

```json
{
  "install_id": "550e8400-e29b-41d4-a716-446655440000",
  "version": "0.20.0",
  "build_stamp": "20261126.1",
  "channel": "production",
  "os": "macos",
  "arch": "aarch64",
  "remote_enabled": true,
  "meta": {
    "active_runtimes": ["claude-code", "codex"],
    "used_mission_7d": true
  }
}
```

| Field | Description |
| --- | --- |
| `install_id` | Random lowercase UUID v4 generated locally for this install. Cleared immediately if you turn telemetry off, and rotatable at any time. |
| `version` | Runner application version. |
| `build_stamp` | Build identifier stamped during official release or nightly builds. |
| `channel` | Release channel: `production`, `nightly`, or `preview-nightly`. |
| `os` | Operating system: `macos`, `windows`, or `linux`. |
| `arch` | CPU architecture: `aarch64` or `x86_64`. |
| `remote_enabled` | Whether Remote Access is currently enabled (`true` or `false`). |
| `meta.active_runtimes` | Allowlisted coding-agent runtime keys started locally in the past 7 days (for example, `claude-code`, `codex`, `pi`). |
| `meta.used_mission_7d` | Whether at least one multi-agent mission ran locally in the past 7 days (`true` or `false`). |
| `country` (edge-derived) | Two-letter country code derived in memory at the Cloudflare edge from the incoming request; your IP address is never stored or logged. |

## What Runner never sends

- Prompts, terminal input or output, or agent conversations.
- Source code, file contents, repository names, or local file paths.
- Usernames, email addresses, hostnames, or authentication credentials.
- IP addresses — the heartbeat endpoint never stores IP addresses or writes them to logs.
- Per-click analytics, session recordings, or third-party tracking SDKs.

## Retention and storage

Heartbeats are stored in two Cloudflare D1 tables using UTC calendar dates (`YYYY-MM-DD`) only: `installs` retains one compact row per `install_id` (recording `first_seen_day`, `last_seen_day`, and latest version, platform, country, and feature metadata), and `daily_pings` stores at most one row per `(day, install_id)` containing that day's heartbeat fields. Rows in `daily_pings` older than 180 days can be pruned periodically while `installs` stays one row per install.

## Zero impact on the app

Heartbeats run on a detached background worker with a hard 5-second total timeout and redirects disabled, completely separate from update checks. DNS, TLS, proxy, timeout, redirect, and HTTP errors are dropped silently with no retries during that launch, no warnings or toasts, and no startup or shutdown delays.

## Opting out and resetting your Install ID

On a fresh install, Runner waits until you finish or skip First-run Setup before sending its first heartbeat, so unchecking **Send anonymous daily install count** in Setup prevents any initial ping. You can also turn telemetry off, inspect the exact live JSON payload, or click **Reset ID** to rotate your random UUID at any time in **Settings → Privacy**. Turning telemetry off immediately deletes `telemetry_install_id` from your local SQLite database.

### Environment variable overrides

Set `DO_NOT_TRACK=1` (or `true`) or `RUNNER_TELEMETRY=0` (or `false`) in the environment used to launch Runner to lock telemetry off regardless of the saved setting. Because the background `runnerd` daemon reads these variables when it starts, applying an environment variable to an already-running daemon requires quitting Runner, running `runner daemon stop`, and relaunching the app from an environment with the variable set (for example, exported in your shell/session or launched on macOS with `DO_NOT_TRACK=1 open -a Runner`). Note that `runner daemon stop` terminates live agent sessions kept running in the background (such as sessions left alive by **Keep running** quit behavior) — finish or stop those sessions first, or turn **Anonymous install heartbeat** off in **Settings → Privacy** for an immediate opt-out without stopping running sessions. While an override is active, **Settings → Privacy** shows which environment variable disabled telemetry, Runner does not create a new Install ID, and no heartbeat is sent.

```sh
runner daemon stop
export DO_NOT_TRACK=1
```
