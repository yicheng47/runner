# 733 — Move GPUI from the yanked gpui-ce 0.3.3 to gpui-pre

Status: implemented in [PR #767](https://github.com/yicheng47/runner/pull/767); working-tree review clean on 2026-09-30. Awaiting Jason's smoke tests and merge.

Tracking issue: [#733](https://github.com/yicheng47/runner/issues/733). Chore, P1, milestone 0.12. Baseline `main` at `258a938` (2026-09-30). Target `gpui-pre =0.3.7`, the crates.io snapshot of `zed@1a28cff` published 2026-09-28. One mission, one PR, one commit.

## Why

Runner's `gpui` is `gpui-ce` 0.3.3, which was yanked from crates.io on 2026-08-28. It builds only because `Cargo.lock` pins it; a full `cargo update --dry-run` fails today with `version 0.3.3 is yanked`. gpui-ce has since become a fork that diverges on purpose, and Zed's own crates.io `gpui` has been frozen at 0.2.2 since 2025-10-22. `gpui-pre` republishes upstream Zed's GPUI crates weekly and names the Zed commit each release was cut from; gpui-component and gpui-kit moved onto it in 0.7.0. The issue carries the full reasoning and the alternatives that were not chosen.

## What ships

Runner builds on `gpui-pre =0.3.7` on macOS and Windows with no intended user-visible change. Every `gpui-pre-*` crate sits on the same exact version, and no `gpui-ce` remains in the dependency graph. The docs name the new dependency and how to bump it. Upstream capabilities that arrive with it (`Window::request_attention`, AccessKit accessibility, the new IME hooks) become available but are not adopted here.

## Size, measured by a probe compile

On 2026-09-30 a throwaway probe applied the dependency change below to `258a938` and fixed every compiler error mechanically until `runner-app` built. The probe was then reversed; this section records what it found.

- **`cargo check -p runner-app --all-targets` on macOS:** 29 errors in the library (`ui/`), the same set the 2026-09-29 spike found, then 24 more in the binary (`main.rs`, `surfaces/`, `terminal/`) once the library compiled. After the fixes in section 3 the check passes, and `cargo clippy -p runner-app --all-targets -- -D warnings` is clean.
- **`cargo test -p runner-app` on macOS:** 519 of 525 pass. The six failures are behavior differences, listed in section 5.
- **Windows** was not compiled: a cross-check from macOS stops in the C build scripts of `ring` and `libsqlite3-sys`, which need Windows SDK headers. The Windows-only Rust in `platform_ui/windows.rs` calls only GPUI APIs that still exist in 0.3.7, and `WindowControlArea` keeps `Drag`, `Close`, `Max` and `Min`. CI is the first Windows compile.
- `cargo update --dry-run` resolves; `cargo tree -i gpui-ce` finds nothing; there is one copy of each `gpui-pre-*` crate.

The API survey that preceded the probe holds: 133 of the 134 GPUI names Runner imports still exist (`Corner` became `Anchor`); `Element` and `EntityInputHandler` only gained defaulted methods; `ShapedGlyph`, `request_measured_layout` and the test harness (`TestAppContext`, `VisualTestContext`, `simulate_resize`, `debug_bounds`, `NoopTextSystem`) are intact. It missed four changes the compiler found: `FocusHandle::focus` taking `cx`, `ShapedLine::paint` gaining two parameters, `flex_shrink` taking a factor, and the `&App` helpers that now focus.

## Where it touches

### 1. Dependencies

In `crates/runner-app/Cargo.toml`, replace the four `gpui-ce` entries. Keep the dependency name `gpui` so the code reads like upstream. Target shape:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
gpui = { package = "gpui-pre", version = "=0.3.7" }
gpui_platform = { package = "gpui-pre-platform", version = "=0.3.7", features = ["font-kit"] }

[target.'cfg(windows)'.dependencies]
# Keep gpui's defaults except windows-manifest; resync when its defaults change.
gpui = { package = "gpui-pre", version = "=0.3.7", default-features = false, features = ["font-kit", "wayland", "x11"] }
# Not gpui_platform: it turns windows-manifest back on (see below).
gpui_windows = { package = "gpui-pre-windows", version = "=0.3.7", default-features = false }

[target.'cfg(windows)'.dev-dependencies]
gpui = { package = "gpui-pre", version = "=0.3.7", default-features = false, features = ["test-support"] }

[target.'cfg(target_os = "macos")'.dev-dependencies]
font-kit = { package = "zed-font-kit", version = "0.14.1-zed" }
gpui = { package = "gpui-pre", version = "=0.3.7", features = ["test-support"] }
```

Upstream split the platform out of `gpui` into `gpui_platform` (a convenience crate) and one crate per OS (`gpui_macos`, `gpui_windows`, `gpui_linux`, …).

- **macOS** uses `gpui_platform`, as Zed's own `main` does. `gpui-pre-platform` has no default features and needs `font-kit`, which enables `gpui_macos/font-kit`; without it GPUI falls back to a text system that lays text out but draws no glyphs.
- **Windows must not use `gpui_platform`.** `gpui-pre-platform` 0.3.7 declares `[target.'cfg(target_os = "windows")'.dependencies.gpui]` with `features = ["windows-manifest"]`, and Cargo unifies features, so it would re-enable the manifest that `817c8b5` turned off. GPUI's manifest (`gpui-pre-0.3.7/resources/windows/gpui.manifest.xml`, now with `SegmentHeap`) would then be embedded next to Runner's own `resources/windows/runner.manifest.xml` from `build.rs`, which carries `longPathAware` and is what the CI step "Verify Runner manifest and icon" checks. Depend on `gpui-pre-windows` directly with `default-features = false`: its `default` feature is `["gpui/default"]`, which also includes `windows-manifest`, and it depends on `gpui` with `default-features = false`.
- The test harness does not need `gpui_platform`: only GPUI's bench macros reference it, not `#[gpui::test]`, `TestAppContext` or `VisualTestContext`. No dev-dependency on it is needed on either platform.
- `gpui-pre`'s defaults are still `font-kit`, `wayland`, `x11`, `windows-manifest`; `font-kit` is the optional `zed-font-kit` dependency, macOS only.

### 2. Entry point

`crates/runner-app/src/main.rs:1250`, `Application::new().with_assets(Assets)`: `Application::new()` no longer exists. Split by platform, with the constructors `gpui-pre-platform` itself uses:

- macOS: `gpui_platform::application()`.
- Windows: `Application::with_platform(Rc::new(gpui_windows::WindowsPlatform::new(false)?))`, or with `.expect(…)` as `gpui_platform::current_platform` does. `WindowsPlatform::new(headless: bool) -> Result<Self>` and `Application::with_platform(Rc<dyn Platform>)` are public.

Drop `Application` from the `use gpui::{…}` list on the path that no longer names it, or Clippy fails on the unused import.

### 3. API breaks the compiler reports

Every row was hit by the probe compile. Paths are under `crates/runner-app/src/`, line numbers on `258a938`.

| Change in 0.3.7 | Sites | Fix |
| --- | --- | --- |
| `FocusHandle::focus(&self, window)` → `focus(&self, window, cx: &mut App)` | 94 calls in 34 files; most in `surfaces/chat.rs` (19), `settings/skills.rs` (8), `settings_page.rs`, `settings/mcp.rs`, `mission_workspace/actions.rs` (6 each) | Pass `cx`. Closures that ignored their context (`\|_, window, _\|`) must name it: `ui/field.rs:2339`, `surfaces/start_chat.rs:1942`, and the tests `crews/tests.rs:1135`, `start_chat.rs:2924` |
| `Window::focus(&handle)` → `focus(&handle, cx)`; `blur()`, `disable_focus()`, `focus_next()`, `focus_prev()` also take `cx` | 22 `focus` calls: `main.rs:917–933` (5), `crews/{add_slot,create,editor,slots}.rs`, `mission_workspace/{attach,feed,input}.rs`, `roles/{create,edit,list}.rs`, `settings_page.rs:976`, `sidebar/project.rs:175`, `start_mission.rs:230`; 1 `blur` in `sidebar/tests.rs:43` | Pass `cx` |
| Helpers that focus from a shared `&App` or with no context | `chat.rs:1215` `focus_active_terminal`, `mission_workspace/attach.rs:619` `focus_active_mission_terminal` and `:636` `focus_mission_drawer_terminal` (take `cx: &App`); `agent_update.rs:293` `restore_focus` and `settings_page.rs:748` `focus_settings_page` (take no `cx`) | Take `cx: &mut App`. Callers mostly pass a `Context<Self>` already. `agent_update.rs:740,906` call `dialog.read(cx).restore_focus(window)`, which cannot also lend `cx` mutably: clone the handle out of the read first (or go through `dialog.update`). `settings/agents.rs:106` names the closure's `cx` |
| `ShapedLine::paint(origin, line_height, window, cx)` → `paint(origin, line_height, align: TextAlign, align_width: Option<Pixels>, window, cx)` | `terminal/element.rs:1384, 1395, 1412` (grid lines, IME marked text, block-cursor text) | `TextAlign::Left, None`. gpui-ce's `paint` passed `TextAlign::default()` (Left) and `None` internally, so terminal output is unchanged |
| `ScrollHandle::max_offset()` returns `Point<Pixels>`, not `Size<Pixels>` | 14: `ui/{field,overlay,scrollbar,select}.rs`, `settings_page.rs:3229`, `start_chat.rs:3960`, `mission_workspace/state.rs:443`, `sidebar/tests.rs:1045–1048`; `ui/select.rs:999` reads `.height` from a stored `max_offset` | `.height` → `.y`, `.width` → `.x`. The sign convention is unchanged: the extent is positive and offsets are negative |
| `BoxShadow` gained `inset: bool` | 15 literals: `ui/{button (2), field, list (3), overlay, session_control, settings, toggle}.rs`, `surfaces/{app_shell (2), command_palette, settings_page, mission_workspace/rail}.rs` | `inset: false` |
| `Menu` gained `disabled: bool` | 5 literals in `main.rs` `app_menus()` (from line 1437) | `disabled: false` |
| `Corner` renamed `Anchor` (same four variants, plus `TopCenter` and `BottomCenter`) | imports and `.anchor(…)` in `ui/tooltip.rs:7,142`, `ui/menu.rs:6,833`, `surfaces/crews/popup.rs:11,162` | Rename |
| `Styled::flex_shrink()` → `flex_shrink(shrink: f32)` | `surfaces/app_shell.rs:521`, `surfaces/crews/popup.rs:123` | `flex_shrink(1.)`, which is what the old method set |

A site that needs more than this gets the smallest change that keeps today's behavior, and a line in the handoff.

### 4. Behavior that changes under the same code

These compile without edits. Keep upstream's defaults unless a test or the smoke test finds a problem, and name each in the PR body.

- **Layout engine.** `gpui-pre` depends on `taffy` 0.13.0; gpui-ce pinned `=0.9.0`. Runner does not use taffy directly (only the `[profile.*.package.taffy]` entries name it). Two of the six test failures are layout differences.
- **Padding snaps to the device pixel grid** when a scroll container computes its scrollable extent (`Interactivity` in `div.rs`), and Taffy lays padding out snapped. Fractional rem sizes can move a measurement by half a pixel.
- **`debug_bounds` are cleared every frame** under `test-support` (`Frame::clear`). gpui-ce kept them across frames, so tests could read a selector from an earlier frame. The comment at `surfaces/settings_page.rs:2977` describes the old behavior; update it, and check that no test depends on a stale selector.
- **Focus clears pending keystrokes.** `Window::focus` and `blur` now call `clear_pending_keystrokes`, which is why they take `cx`. A multi-stroke binding in progress is dropped when focus moves.
- **Inactive windows are throttled to about 30 fps.** `WindowOptions::inactive_frame_interval` defaults to `Some(33.333 ms)`, and Runner's `WindowOptions` in `main.rs` ends in `..Default::default()`, so it inherits the throttle. A terminal streaming agent output in an unfocused window animates at the lower rate. `None` turns it off if the smoke test finds that worse.
- **Titlebar dragging.** `WindowOptions::app_owns_titlebar_drag` (macOS only) defaults to `false`, which keeps AppKit's native titlebar drag that Runner uses with `appears_transparent: true` and `WindowControlArea::Drag`. Leave it `false`.
- **Text rendering.** Windows and the app gained a `TextRenderingMode` that defaults to `PlatformDefault`. No mode change is expected, but glyph rendering is on the smoke list. The atlas now quantizes glyph origins to four horizontal variants and one vertical variant, rather than gpui-ce's four by four.
- **Window roots.** `Window::draw_roots` now stretches auto-sized roots to the viewport. Content-height tests must measure a nested child, and resize tests must call `simulate_resize` to deliver the platform callback.

### 5. Tests that fail after the port

The probe's run of `cargo test -p runner-app` failed these six. For each, find what changed in GPUI, then either fix the code so today's rendering holds or, when the new rendering is correct and the old number was an artifact of gpui-ce, update the expectation and say why in the handoff. Never weaken an assertion just to pass.

- `ui::duplicate_subject_overlay::tests::wrapped_duplicate_overlay_preserves_bottom_padding_at_zoom` (`ui/duplicate_subject_overlay.rs:249`): `32px` against the expected `31.5px`. Likely the padding snap above.
- `surfaces::mission_markdown::tests::markdown_layout_collapses_adjacent_block_margins` (`surfaces/mission_markdown.rs:1346`): a gap of `987.5px` against the expected `6px`. Too large for rounding; look at what the test measures between (a `debug_bounds` read, a flex child growing under taffy 0.13) before touching the layout.
- `surfaces::settings::agents::tests::installed_header_wraps_without_overlapping_actions` (`surfaces/settings/agents.rs:2281`): at a 320 px window the header text lays out 1878 px wide and the actions land at x 3403, so the text no longer wraps. A min-content or shrink difference in taffy 0.13; a real user-visible regression until shown otherwise.
- `surfaces::start_chat::tests::{a_changed_value_carries_the_dot_and_its_reset_restores_only_that_value, another_agent_shows_its_own_defaults_at_full_contrast_and_only_runtime_carries_the_dot, speed_shows_for_codex_roles_codex_overrides_and_direct_codex}` (all at `surfaces/start_chat.rs:1459`, `expect("modal is open")`): `ModalHarness::reset` clicks the centre of `START_CHAT_RESET {kind}` and the click now closes the modal, so the harness's next render finds no modal. Find what the click hits: the Reset may be hidden or not hit-testable without a prior hover, or its bounds may be stale. Check the real Reset button works the same way, not only the harness.

### 6. Patches and forks

- **Zed's `[patch.crates-io]`** (`async-task`, `calloop`, `async-process`, `notify`, …) applies only inside Zed's workspace, and gpui-pre is built and tested against the crates.io versions. The macOS probe needed none. Add nothing to our root `Cargo.toml` unless the Windows build or the smoke test shows a need, and give each added entry a comment with its reason. Runner keeps its crates.io `notify`.
- **`zed-font-kit`** (macOS dev-dependency) already matches: `gpui-pre-macos` 0.3.7 depends on `zed-font-kit ^0.14.1-zed`.
- **`zed-reqwest`** (`runner-backend`, and `runner-app` on Windows for the updater) stays. No `gpui-pre-reqwest` enters the lock, so it is Runner's own; switching Runner's HTTP client is a separate decision. Declare `socks` explicitly on Runner's backend and Windows updater dependencies: gpui-ce's `gpui_http_client` previously enabled it through Cargo feature unification; the new GPUI HTTP crate no longer does. A captured `ALL_PROXY=socks5h://127.0.0.1:1080` must still build Runner's HTTP client without a network request.
- `block 0.1.6` still prints a future-incompatibility warning. It was already in the lock under gpui-ce, through `cocoa`; not this change's.

### 7. Build profiles

In the root `Cargo.toml`, rename `[profile.dev.package.gpui-ce]` and `[profile.ci.package.gpui-ce]` to `gpui-pre`, keeping their opt-levels. The Metal and DirectX renderers and the platform text systems lived inside `gpui-ce`, which dev builds at level 3; upstream moved them into `gpui-pre-macos` and `gpui-pre-windows` (`gpui-pre-apple` holds shared Apple code). Give those three `opt-level = 3` in `dev` too, so `make run` keeps today's rendering performance. The `ci` profile needs no new entries: `"*"` is already level 1.

### 8. Docs

- `docs/arch/arch.md:98`, the stack table row: `gpui-pre` 0.3.7 (`zed@1a28cff`), a crates.io snapshot of upstream Zed's GPUI.
- `docs/arch/arch.md:911`, the API-break risk line: `gpui-pre` is pinned exactly and bumped deliberately; the terminal element and the IME integration remain the surfaces most exposed.
- Beside the stack row, the bump procedure: every `gpui-pre-*` pin moves together to one release, the release's crates.io description names its Zed commit, and that commit is the Zed source to read. Check that the Windows manifest step in CI still passes after a bump, since a new `gpui-pre-windows` default could re-enable GPUI's manifest.
- `docs/tech/gpui-rendering.md:3` and `docs/tech/README.md:11,32`: point at `~/.cargo/registry/src/*/gpui-pre-0.3.7/src`, with the platform code in `gpui-pre-macos-0.3.7` and `gpui-pre-windows-0.3.7`. Check each claim the note makes about GPUI internals against the new source and correct any that moved (for example `ShapedLine::paint`'s signature and where the Metal renderer lives).
- `docs/arch/windows.md:114`: the DXGI debug probe still exists, as `check_debug_layer_available` in `gpui-pre-windows-0.3.7/src/directx_devices.rs`, only under `debug_assertions`. Reword the sentence to name `gpui-pre-windows`.
- `README.md:64,304` and `README.zh-CN.md:64,304`, in the same commit: Runner is built on Zed's GPUI through `gpui-pre`; the acknowledgements credit Zed and the gpui-pre publishers (gpui-kit, Jason Lee) instead of gpui-ce.
- `docs/features/701-desktop-notifications.md:84,96`: the spec says gpui-ce 0.3.3 has no `request_attention`, so the dock bounce goes in `platform_ui`. gpui-pre has `Window::request_attention`; note that in the spec and leave the design choice to #701. Line 96 cites gpui-ce source paths for `WindowKind::PopUp`; re-point them at `gpui-pre-windows` and `gpui-pre-macos`.
- `crates/runner-app/src/surfaces/settings_page.rs:2977`: the test comment about gpui-ce never clearing `debug_bounds` (section 4).

## Rules of the road

- Mission authorization follows AGENTS.md: branch `chore/733-gpui-pre` in `.worktrees/chore-733-gpui-pre` from `origin/main`; commits on it are authorized; everything, the brief included, is squashed into one commit before the push; the crew opens the PR against `main`, drives CI green on both platforms, and stops. No merge, no branch or worktree removal, no nightly or release.
- No behavior change beyond section 4. Do not adopt `request_attention`, accessibility or the new IME hooks, and do not change `inactive_frame_interval` or `app_owns_titlebar_drag` unless the smoke test asks for it.
- Do not launch the Runner app (`make run`); Jason smoke-tests.
- Gate imports and helpers that only `cfg(unix)` tests use behind `cfg(unix)`, or Windows Clippy goes red.

## Verification

- `make verify` on macOS, and CI green on macOS and Windows with the new `Cargo.lock` under `--locked`, including the Windows step "Verify Runner manifest and icon" (`longPathAware` must still be in `RT_MANIFEST #1`).
- `cargo update --dry-run` resolves without the yank error.
- `cargo tree -i gpui-ce` finds nothing, `cargo tree -d` shows one copy of each `gpui-pre-*` crate, and on Windows `cargo tree -e features -i gpui-pre --target x86_64-pc-windows-msvc` shows no `windows-manifest`.
- The six tests in section 5 pass, with the reason for each fix or changed expectation in the handoff.
- CI build times, cold and warm, against the last green `main` run, in the PR body.
- Smoke test by Jason on macOS, then on Windows on JASONPC: terminal rendering and scrolling, including agent output streaming into an unfocused window; IME, Chinese input in a chat and in a text field; window chrome, titlebar drag and double-click; the menu bar; tooltips and popovers (`Anchor`); focus moves in the role, crew and mission forms and modals, and focus returning to the terminal after a modal closes; scrollbars and scroll-to-end (`max_offset`); Settings › Agents at a narrow window (the header wraps); mission feed markdown spacing; Start Chat's per-control Reset.

## Non-goals

Pulse, which is on a `gpui-ce` git rev and is a separate decision; adopting gpui-component or gpui-kit; accessibility; `request_attention`, which is #701's; any `gpui-pre` version other than 0.3.7; Linux; switching Runner's own HTTP client off `zed-reqwest`.
