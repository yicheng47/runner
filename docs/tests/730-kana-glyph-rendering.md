# #730 Japanese terminal glyph rendering

## Environment and diagnosis

Verified on macOS 26.6.2 (25G83), Runner source `2dd05b6` plus the #730 mission brief, with `gpui-ce` 0.3.3 locked by `Cargo.lock`. The terminal request is bundled `JetBrainsMono Nerd Font Mono` at 16 px with `PingFang SC` first in its fallback list. The bundled family has regular, bold, italic and bold italic faces (`JetBrainsMonoNFM-Regular`, `-Bold`, `-Italic`, `-BoldItalic`). CoreText resolves kana to `PingFangSC-Regular` for plain/italic and `PingFangSC-Semibold` for bold/bold italic; Latin stays in the corresponding JetBrains face. Emoji resolves to `AppleColorEmoji`.

macOS 26 provides two different `PingFangSC-Regular` faces with the same PostScript name: `.../AssetData/PingFang.ttc` and `.../Reserved/PingFangUI.ttc`. The normal face maps `タ` to glyph 972; the reserved UI face has no `タ` mapping and uses glyph 972 for `强`. The two collections also contain `PingFangSC-Semibold` faces with the same name. GPUI 0.3.3 keyed `MacTextSystemState::font_ids_by_postscript_name` only by PostScript name. If the reserved face entered that map first, CoreText could shape terminal kana against the normal face and GPUI could attach the reserved face's `FontId`; `ShapedLine::paint` then passed that ID and the normal face's glyph ID to `Window::paint_glyph`, whose atlas key and rasterizer used the reserved face. This explains correct copied Unicode with incorrect pixels and why cache order matters. It does not depend on italic synthesis.

The patch keys macOS font IDs by PostScript name **and backing file path**, retaining the same `FontId` from shaping through atlas lookup and rasterization. Windows and the terminal's cell layout, fallback order, text, and copy paths are unchanged. This is a local patch to the locked GPUI version under `vendor/gpui-ce`; the maintenance cost is carrying the vendored crate until the same fix can be upstreamed or adopted during the separately tracked #733 GPUI upgrade.

## Reproduction evidence

The issue reported `タイムパラドックス` painted as unrelated Han characters while paste returned the original kana. A standalone CoreText check on this machine used the bundled JetBrains faces and `PingFang SC` fallback: all four styles shaped `タイムパラドックス` as glyphs `972, 945, 1005, 990, 1014, 982, 976, 956, 966` in the normal PingFang collection. The same check shaped `ココロありがとう` in PingFang and kept `ABC` in JetBrains, `中文` in PingFang, and `😀` in Apple Color Emoji. A headless GPUI 0.3.3 harness confirmed those per-cell glyph IDs and resolved font IDs over three passes before the fix. These checks used the real macOS CoreText system; they did not capture a Runner UI image.

The macOS GPUI regression deliberately loads the reserved UI regular and semibold faces first, then shapes both reported strings with `PingFang SC` fallback in plain, bold, italic, and bold italic styles. Before the patch it failed on the first `タ`: the shaped glyph was 972, but GPUI's selected font had no `タ` mapping. After the patch it passes in cold and warmed/reversed style order, comparing the bytes produced by GPUI's glyph rasterizer with bytes from the correct normal PingFang face. It runs only when both PingFang collections are installed; macOS systems without this duplicate pair skip that specific collision case. CI runs it separately because the vendored dependency is outside Runner's workspace formatting and Clippy scope.

The `runner-terminal` SGR test confirms that both strings remain the expected Unicode in the grid under all four styles, with two-column kana cells and the expected bold/italic flags. Existing terminal fixture coverage exercises representative Latin, Han, kana, emoji, box drawing, and wide-cell output. Pixel validation is from the automated macOS GPUI raster comparison, not a visual inspection of an actual Runner window. A visual smoke check in a disposable Runner terminal is still needed on the built app.

## Manual smoke

In a fresh disposable Runner terminal configured for JetBrains Mono at 16 px, run:

```sh
printf 'plain:  タイムパラドックス ココロありがとう\n\e[1mbold:   タイムパラドックス ココロありがとう\e[0m\n\e[3mitalic: タイムパラドックス ココロありがとう\e[0m\n\e[1;3mbold+it: タイムパラドックス ココロありがとう\e[0m\n'
```

Compare all four rows with Terminal.app, repeat after other Latin/Han/emoji output and in reverse style order, then select and copy each kana string. Check that kana pixels remain legible and copied text is exact, Latin bold/italic still looks styled, and terminal columns remain aligned. This Runner UI check has not yet been performed.

## Checks

| Command | Exit |
| --- | ---: |
| `cargo test --locked -p gpui-ce fallback_glyphs_keep_their_font_file_when_postscript_names_collide --lib -- --nocapture` (before fix, temporary workspace membership) | 101, expected failure on `タ` |
| `cargo test --locked -p gpui-ce fallback_glyphs_keep_their_font_file_when_postscript_names_collide --lib -- --nocapture` (after fix) | 0 |
| `CARGO_TARGET_DIR=target cargo test --locked --manifest-path vendor/gpui-ce/Cargo.toml fallback_glyphs_keep_their_font_file_when_postscript_names_collide --lib -- --nocapture` | 0 |
| `CARGO_TARGET_DIR=target cargo test --locked --manifest-path vendor/gpui-ce/Cargo.toml --lib` | 0 (72 passed, 1 ignored) |
| `cargo test --locked -p runner-terminal kana_stays_in_the_grid_across_sgr_styles` | 0 |
| `cargo test --locked -p runner-terminal` | 0 |
| `cargo test --locked -p runner-app` | 0 |
| `cargo test --locked --workspace --no-fail-fast --profile ci --timings` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 |
| `cargo clippy --locked -p runner-app --features updater --all-targets --profile ci -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `rustfmt --edition 2024 --check vendor/gpui-ce/src/platform/mac/text_system.rs` | 0 |
| `git diff --check` | 0 |

CI status will be recorded after the PR opens.
