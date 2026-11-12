# #1248 item 3 — Files module split

Changed behavior: none. The candidate replaces `files/view.rs` and `files/viewer.rs` with focused implementation modules while retaining `files::FilesView`, the `FilesView` and `Viewer` fields and initialization, and the existing models and helpers. No native app launch, real agent chat or live regression check is required for this mechanical move.

## Module map

Paths below are relative to `crates/runner-app/src/surfaces/files/`. Declaration counts include methods, helpers, state types, constants, trait implementations and the existing size-formatting test module.

| Original | Destination | Responsibility | Declarations |
| --- | --- | --- | ---: |
| `view.rs` | `panel_view.rs` | `FilesView` state, initialization, shared requests, focus, sizing and surface composition | 27 |
| `view.rs` | `panel_view/roots.rs` | Root resolution and selection, root picker, header and root edge states | 28 |
| `view.rs` | `panel_view/tree.rs` | Listings, navigation, row actions, icons and tree rendering | 17 |
| `view.rs` | `panel_view/watch.rs` | Visibility, watch sequencing, file events and git status | 7 |
| `viewer.rs` | `open_file.rs` | `Viewer` state, reads, reloads, tabs, source selection, focus and test accessors | 60 |
| `viewer.rs` | `open_file/find.rs` | In-file find, asynchronous results and find-bar composition over code | 12 |
| `view.rs` + `viewer.rs` | `open_file/open_in.rs` | Editor discovery, Open in, line links, reveal and its menu | 14 |
| `viewer.rs` | `open_file/render.rs` | File column, tab strip, overflow list and file edge states | 15 |

`tree.rs`, `tabs.rs`, `picker.rs`, `finder.rs`, `document.rs`, `links.rs` and `markdown.rs` retain their responsibilities. In particular, `finder.rs` still finds file names, separately from `open_file/find.rs` searching inside an open file.

## Source and wiring audit

The complete inventory accounts for all 180 original declarations: 140 `FilesView` methods (including 22 test-only accessors), two `Render` implementations and 38 other declarations. A Rust syntax-tree comparison checks each original declaration against its destination, including signatures, fields, comments, literals, rendering expressions, callbacks and cfg gates. It normalizes only declaration visibility, the extra relative module prefix, formatting and the explicit original tracing target described below. No declaration is dropped or duplicated. Temporary inventories, comparison scripts and logs live under `/tmp/1248-files-audit/`, outside the product.

Child implementation modules inherit access to their owner's private state. Methods needed by their owner or another child gain only `pub(super)`; existing methods exposed to the Files subtree use `pub(in super::super)` after moving one level deeper, preserving their original reach. All 25 newly exposed private methods/helpers are limited to their owning module; 14 existing Files-visible declarations retain that reach with the longer visibility path. No field visibility changes. The two reveal helpers are re-exported within Files through `open_file`; the caller-facing `files::FilesView` re-export is unchanged.

The moved paths contain no source-relative includes, `#[path]` attributes, custom macros or explicit module/type-name-dependent calls. Platform cfg gates and commands remain unchanged. The one `tracing::warn!` in `reveal_active` explicitly retains `runner_app::surfaces::files::viewer` as its target, preserving existing log filtering despite relocation; its message remains unchanged. Other diagnostic source locations naturally follow the moved files. No active architecture or guide reference points to the removed files; the mission brief names them as the historical starting point.

## Selected checks and results

These checks were selected before testing: declaration inventory and move-aware comparison to detect omissions or body changes; before/after app test lists and ignored lists to detect lost coverage; `make verify` to cover workspace compilation, tests, Clippy, macOS updater-feature Clippy and formatting; and fresh CI on macOS, Windows, Linux x86_64 and Linux arm64 to cover platform-specific imports and cfg paths. Existing Files tests exercise root switching, stale replies, watcher lifetime, status, navigation, tabs, reloads, focus, find, Open in/reveal and source/rendered files. No new behavioral tests or golden updates are needed.

| Check | Result |
| --- | --- |
| Complete source inventory/equivalence | 180 declarations accounted for; pass |
| Initial `cargo check -p runner-app --tests` | Exit 0 |
| `cargo test -p runner-app -- --list`, before/after | Exit 0 before and after: 1,421 library tests + 42 integration tests; all 169 Files tests retained after the one mapping below |
| Ignored test lists, before/after | All four ignored tests unchanged, including the Files Markdown small-stack subprocess helper |
| `make verify` | Exit 0 on the final source (519.7 s); workspace check/tests, both Clippy passes and formatting pass; initial invocation also exit 0 |
| Fresh four-platform CI | All four platforms passed on reviewed head `324308a07684a7757d5e46508a7450c678f84140`; attempt 1, no retries; evidence below |
| Independent review | Reviewer independently passed source snapshot `8d069b778ec2838248ac13417e956189639eb5f9` and [confirmed published head `324308a0`](https://github.com/yicheng47/runner/pull/1280#issuecomment-6815975895); no remaining must-fix issues and no required live checks |

Local validation runs on macOS arm64 with Rust 1.97.1 and this worktree’s own `target/`; `CARGO_TARGET_DIR` is unset. Both verification invocations completed successfully; the repeat validates the final tracing-target adjustment without changing test scope. The final verified app binaries also reproduce the complete test-list and ignored-status accounting above. Logs and exit-code records are retained under `/tmp/1248-files-audit/`.

The sole intentional test-name mapping is `surfaces::files::viewer::tests::sizes_read_as_people_write_them` → `surfaces::files::open_file::tests::sizes_read_as_people_write_them`. All other test names, assertions, fixtures, goldens, ignored attributes and platform cfgs remain unchanged. The shared `files/tests.rs` changes only its `absolute` helper import and the `IGNORED_REFRESH` path.

Before publication, `origin/main` advanced to `515b8f49` through #1278; its daemon/terminal changes do not overlap Files. The remaining open PR, #1277, also changes no Files code. The branch retains its original base, with fresh PR CI required against main’s current merge result. The reviewer’s independent audit artifacts are `/tmp/1248-files-review-audit.py` and `/tmp/1248-files-review/`.

## CI evidence

[PR #1280](https://github.com/yicheng47/runner/pull/1280), [CI run 43164799812](https://github.com/yicheng47/runner/actions/runs/43164799812), attempt 1, passed on reviewed head `324308a07684a7757d5e46508a7450c678f84140`. All platform test steps passed, with no failed attempt or retry. This evidence-only amendment changes no production or test source; current-head check results and the final head-specific review remain on the PR.

| Platform | Result | Job |
| --- | --- | --- |
| Linux (aarch64) | Pass | [Job](https://github.com/yicheng47/runner/actions/runs/43164799812/job/131004051581) |
| Windows | Pass | [Job](https://github.com/yicheng47/runner/actions/runs/43164799812/job/131004051787) |
| Linux (x86_64) | Pass | [Job](https://github.com/yicheng47/runner/actions/runs/43164799812/job/131004051885) |
| macOS | Pass | [Job](https://github.com/yicheng47/runner/actions/runs/43164799812/job/131004051949) |
