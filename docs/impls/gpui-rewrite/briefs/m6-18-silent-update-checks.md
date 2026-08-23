# Mission brief — M6.18: silent background update checks

Drafted 2026-08-22 late evening. Start with `mission_start` on crew `codex-crew` (`01KVG1NCM4RJC23Q4ZQ5Z8SBHH`, coder = codex lead, reviewer = claude-code), project `runner` (`01KZGS24S5103N6B2HV72ZG1AD`), explicit `cwd` the `runner-gpui` worktree, title `M6.18 — Silent background update checks`, the text below as `goal_override`. No Sparkle update while the crew runs.

---

Implement **M6.18** (`docs/impls/gpui-rewrite/m6-consolidation.md` §M6.18 — read it first; this brief also lives at `docs/impls/gpui-rewrite/briefs/m6-18-silent-update-checks.md`). One feature branch in this worktree off `gpui-nightly`; leave uncommitted docs under `docs/impls/gpui-rewrite/` alone. One crate, one file: `crates/runner-app/src/updater.rs` (`runner-app` is the binary; `runner-backend` the core). Read `briefs/m6-9-update-hint.md` for where the delegate came from, and the standing GPUI rules in `impl_log.md` — the delegate runs on the main thread inside AppKit callbacks that cannot unwind; every entity touch goes through `cx.update` as `UpdaterDelegate::apply_transition` does today.

## The defect

`NativeUpdater::start` (`updater.rs` ~326-333) calls `startUpdater`, then `checkForUpdatesInBackground` when automatic checks are on, and Sparkle's scheduler repeats that check on its interval. Both run through `SPUStandardUpdaterController`'s standard user driver, which shows the "A new version is available" alert for background checks exactly as for user-initiated ones. The M6.9 delegate (`UpdaterDelegate`, ~186-224) only *observes* — `updater:didFindValidUpdate:` lights the Settings-row icon, nothing declines the alert — so a scheduled check that finds an update opens Sparkle's window on its own and the icon is redundant with it. Wanted: background checks light the icon and nothing else; Sparkle's sheet appears only when the human asks — the icon click, the Updates pane button, the app-menu item, all of which call `checkForUpdates:` today.

## Facts verified against Sparkle 2.9.5's headers (2026-08-22)

- `SPUUpdaterDelegate.h`: `- (BOOL)updater:(SPUUpdater *)updater shouldProceedWithUpdate:(SUAppcastItem *)updateItem updateCheck:(SPUUpdateCheck)updateCheck error:(NSError * __autoreleasing *)error;` — "Returns whether the updater should proceed with the new chosen update from the appcast"; when NO is returned **with a populated error**, "the user is not shown this updateItem nor is the update downloaded or installed."
- `SPUUpdateCheck.h`: `SPUUpdateCheckUpdates = 0` (user-initiated, `checkForUpdates`), `SPUUpdateCheckUpdatesInBackground = 1` (scheduled/background, `checkForUpdatesInBackground`), `SPUUpdateCheckUpdateInformation = 2` (`checkForUpdateInformation` probe).
- `SUErrors.h` has no "declined by delegate" code — `SUNoUpdateError = 1001` is the nearest. So the delegate supplies its **own** NSError (domain `com.wycstudios.runner.updater`, code 1, a localized description) and recognizes that domain later.
- The header does not state the call order between `shouldProceedWithUpdate` and `didFindValidUpdate`. Read Sparkle 2.9.5's source (`Sparkle/SPUBasicUpdateDriver.m`, `SPUUIBasedUpdateDriver.m` — search both selectors) and cite the lines in the handoff. The implementation below is correct in either order; the citation is the record.
- Declining aborts the check through `updater:didAbortWithError:`, which today applies `UpdateTransition::Aborted` and clears the icon. That path must ignore Runner's own decline.

## Scope

1. `UpdaterDelegate` gains `#[unsafe(method(updater:shouldProceedWithUpdate:updateCheck:error:))]`. Body: derive the version exactly as `did_find_valid_update` does (factor the display-version-or-version-string logic into one helper) and apply `UpdateTransition::Found(version)` — the existing `duplicate_found_update_does_not_change_available_state` behaviour makes a second `Found` from `didFindValidUpdate` harmless, whichever runs first. Then: `update_check == 1` → build the NSError above, write it into the `NSError **` out-parameter as an autoreleased pointer (objc2 `Retained::autorelease_return`; never leak, never double-release; handle a null out-pointer), return `false`; any other check kind → return `true` without touching the error. Get the ABI right: `BOOL` return, `NSInteger` for the enum, the out-parameter as `*mut *mut NSError`; a wrong signature is either a silent no-op or a crash inside an AppKit callback.
2. `did_abort_with_error` takes `&NSError` instead of `&AnyObject`; when `error.domain()` is Runner's domain it returns without a transition, otherwise `Aborted` as today.
3. `NativeUpdater::start` and the click/menu/pane paths are unchanged: the launch check stays `checkForUpdatesInBackground`; `check_for_updates` stays `checkForUpdates:` and now is the only way Sparkle's sheet appears.
4. Pure tests beside the existing ones (`updater.rs` ~411-460): `should_proceed(update_check) -> bool` is false only for 1; an abort carrying Runner's domain leaves `available` set while any other abort clears it; `Found` then Runner-abort then `Found` of the same version stays available and does not re-notify. Keep the objc2 glue thin over those pure functions so the tests cover the decision, not the bindings.

## Out of scope

The fallback design (Sparkle's scheduler off, Runner's own interval calling `checkForUpdateInformation`) — only if the decline path proves unusable, and say why in the handoff. M6.17 universal builds. Any UI change: the M6.9 icon, tooltip and click are final. `suppress`/skip semantics: "Skip this version" from Sparkle's sheet still clears the icon through `userDidMakeChoice`, as today.

## Gates

- `make verify` green; `git diff --check` clean; `cargo test -p runner-app` including the new pure tests.
- Handoff: the delegate selector as wired, the Sparkle source lines for the call order, and the statement of which check kinds are declined.
- Human verification rides the next nightly (with M6.17): on launch the icon lights with no window; clicking it opens Sparkle's sheet for that version; Check for Updates in the Updates pane and the app menu open the sheet directly; Skip clears the icon.
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) `*error` left null on decline (Sparkle's "not shown, not downloaded" guarantee is tied to a populated error); (2) the decline's abort still clearing the icon; (3) `Found` applied only in `didFindValidUpdate`, so a shouldProceed-first order never lights the icon; (4) check kinds 0 or 2 declined, or the check-kind compared against the wrong integer; (5) the objc2 method signature — `BOOL`/`NSInteger`/`NSError **` — and the out-pointer written without the autorelease convention, or the NSError leaked or released twice; (6) any entity update outside `cx.update`, or work on the delegate thread that can panic; (7) the decision logic buried in the objc2 glue instead of the pure functions the tests cover.
