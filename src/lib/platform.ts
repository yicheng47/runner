// Primary-modifier platform split. macOS shortcuts use ⌘ (Cmd); Windows and
// Linux use Ctrl. One place to branch so shortcut *listeners* and their
// *labels* never drift apart across platforms.
//
// Scope note: this is only for app-level shortcuts that are safe to rebind to
// Ctrl (e.g. Cmd+digit → Ctrl+digit, Cmd+S). Terminal-focused combos that map
// onto control characters (Ctrl+[ = ESC, Ctrl+S = XOFF inside the shell) are
// deliberately NOT rebound to Ctrl — see RunnerTerminal — so they keep their
// terminal meaning on Windows.

const IS_MAC =
  typeof navigator !== "undefined" && /mac/i.test(navigator.userAgent);

export const isMac = IS_MAC;

/** Display prefix for the primary modifier: "⌘" on macOS, "Ctrl+" elsewhere.
 *  So `` `${MOD_LABEL}K` `` renders "⌘K" on macOS and "Ctrl+K" on Windows. */
export const MOD_LABEL = IS_MAC ? "⌘" : "Ctrl+";

type ModEvent = { metaKey: boolean; ctrlKey: boolean };

/** True when the platform's primary modifier is held: ⌘ on macOS, Ctrl
 *  elsewhere. Use in keydown handlers so Windows honors Ctrl where macOS
 *  honors ⌘. */
export function isModKey(e: ModEvent): boolean {
  return IS_MAC ? e.metaKey : e.ctrlKey;
}

/** True when the *opposite* modifier is held — for handlers that must reject
 *  the cross-platform twin (e.g. ⌘+digit on macOS must ignore a stray Ctrl,
 *  and Ctrl+digit on Windows must ignore a stray ⌘). */
export function isOppositeMod(e: ModEvent): boolean {
  return IS_MAC ? e.ctrlKey : e.metaKey;
}
