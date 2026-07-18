# native-spike

Phase 1 spike for [impl 0031](../../docs/impls/0031-rust-native-ui-rewrite/plan.md): a native macOS window rendering a live claude-code session via `alacritty_terminal` on GPUI — one parser, one grid, no serialization boundary. Throwaway-friendly; nothing here is a public API.

## Run

```sh
cargo run -p native-spike                  # spawns `claude`
cargo run -p native-spike -- /bin/zsh      # any other command
```

The window is a terminal pane (click to focus, type to interact, scroll wheel for history) over an IME test composer (click it, type — Enter sends the buffer + CR to the PTY).

Requires the Xcode Metal Toolchain for gpui's shader build (`xcodebuild -downloadComponent MetalToolchain`, already installed on this machine). The fixture harness and recorder build without it: `--no-default-features`.

## Fixture corpus (regression harness)

Recorded PTY byte logs in `fixtures/*.ndjson` (header line + base64 data/input/exit events), replayed into a headless `Term` and compared against blessed `*.snapshot.txt` grids:

```sh
cargo test -p native-spike --no-default-features            # replay + width/reflow tests
UPDATE_SNAPSHOTS=1 cargo test -p native-spike --no-default-features   # re-bless
```

Record new fixtures from any command:

```sh
cargo run -p native-spike --no-default-features --bin record-fixture -- \
  --out crates/native-spike/fixtures/name.ndjson --cols 100 --rows 30 \
  --duration-ms 30000 --input '3000:some text' --input '4000:\r' -- claude
```

Current corpus: `claude-session` (real interactive TUI boot → prompt → streamed reply → /exit palette), `top-busy` (full-screen redraw churn), `width-torture` (CJK/emoji/ZWJ/box-drawing/SGR glyph classes).

## Packaging (exit criterion 5)

```sh
crates/native-spike/package.sh              # release build + .app + codesign
crates/native-spike/package.sh --notarize   # + notarytool submit + staple + spctl
```

Signing uses the first Developer ID Application identity in the keychain (`SIGN_IDENTITY` overrides). Notarization needs either `NOTARY_PROFILE=<notarytool keychain profile>` or `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` (same trio the release CI uses).

## Human verification checklist (exit criteria needing eyes)

1. **Busy output**: `cargo run -p native-spike` → give claude a chatty task (e.g. "explain this repo file by file"). Watch for tearing, lag, or garbled rows while the spinner runs and output streams. Resize the window mid-stream.
2. **Glyph widths**: in the spike terminal run `sh crates/native-spike/fixtures/width-torture.sh` — every `|`-delimited column should align, CJK exactly double-width, no overlap/gaps; compare against Ghostty/Terminal.app side by side.
3. **IME**: click the composer bar, switch to Pinyin, type 中文测试 — composition underline should render in place, the candidate window should anchor to the field, Enter during composition must commit (not submit). Then Enter to send it to claude.
4. **Resize reflow**: with scrollback content, narrow the window hard, widen it back — wrapped lines should reflow, no ring-buffer garbage, cursor stays with the prompt (#308 class).
5. **Packaging**: `package.sh --notarize` with notary credentials, then launch the stapled `target/native-spike-dist/Runner Native Spike.app` on a fresh Gatekeeper context (or `spctl -a -vv` output).
