# runner-app

Phase 3 walking skeleton for [the gpui-rewrite plan](../../docs/impls/gpui-rewrite/plan.md). It opens Runner's existing direct chats from the same SQLite database as the released app, spawns or resumes them through `runner_backend::session::SessionManager`, and renders the manager's PTY byte stream with `alacritty_terminal` on GPUI.

## Run

```sh
make run
```

The development command uses `~/Library/Application Support/com.wycstudios.runner-dev/runner.db`, matching the Tauri development environment and keeping production state isolated. A future packaged release build will use the production directory. Click an existing direct chat in the sidebar, then use either the terminal or composer.

GPUI requires the Xcode 26 Metal Toolchain component, already installed on the development machine.

## Fixture corpus (regression harness)

Recorded PTY byte logs in `fixtures/*.ndjson` (header line + base64 data/input/exit events), replayed into a headless `Term` and compared against blessed `*.snapshot.txt` grids:

```sh
cargo test -p runner-app
UPDATE_SNAPSHOTS=1 cargo test -p runner-app
```

Current corpus: `claude-session` (real interactive TUI boot → prompt → streamed reply → /exit palette), `top-busy` (full-screen redraw churn), `width-torture` (CJK/emoji/ZWJ/box-drawing/SGR glyph classes).

## Smoke test

1. Run `make run` and click a stopped direct chat in the sidebar. It should resume the same development conversation and render its live terminal.
2. Type in the terminal, submit a prompt, scroll, and resize the window. Output, input, and PTY geometry should remain live.
3. Click the composer, switch to Pinyin, type `中文测试`, choose a candidate, and press Enter once more. Composition should stay anchored in the field and the committed text should submit to the selected chat.
