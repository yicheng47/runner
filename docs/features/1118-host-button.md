# 1118 — Host button in the sidebar footer

Tracking issue: [#1118](https://github.com/yicheng47/runner/issues/1118). Design: `runner/desktop/specs/1118-host-button.pen` in the design repo, board `Lmi8p` (option A, three steps and notes) and board `sIhQ7` (M1, how this computer is marked), both signed off by Jason on 2026-11-04. Where the design and this spec disagree on visual detail, the design wins; on behaviour, this spec wins. Frame 1's caption mentions "open All hosts": that is left over from the dropped all-hosts view and is not part of this feature.

## Motivation

Signed in, the sidebar puts a full-width computer-picker trigger under the brand: a large bordered button with the computer's name that takes a prominent slot at the top of the sidebar for something most windows rarely change. It opens a popover that switches one window to one computer at a time. In the popover, this computer looks like every other row, so it is easy to miss which one is yours.

## Scope

### Host button

- The full-width trigger under the brand (`render_computer_trigger`) is removed.
- Signed in or out, the sidebar footer row reads: usage, flexible space, the host button, the Settings gear. The host button is a monitor glyph with the gear's 32 px hit area and hover treatment.
- Its dot (top-right, green) means another paired computer needs you, with exactly today's trigger rule: not this computer, not the computer this window already shows, and `needs_attention` true.
- While the popup is open the button shows the selected fill, as frame 1 draws it.
- While the window shows another computer the button turns green: accent glyph, accent border and an accent tint, as today's trigger does in that state (frame 2).
- Signed out the host button still shows (option S2, Jason, 2026-11-05; board "Signed out · S2 chosen" in the spec's `.pen` file). It never shows a dot signed out.

### Signed out

- The popup lists only this computer (green tile, "This computer", check), a divider, then one row "Sign in to use other computers" with a log-in icon. That row has the Remote access settings row's neutral colours, not accent, and opens Settings → Account.
- No account directory is fetched, and no cached computers, directory errors or action errors appear while signed out. Keyboard behaviour is the same as signed in.

### Host popup

- Clicking the button, or Enter, Space or Down while it is focused, toggles the popup, as the trigger does today. The popup opens above the footer, left-aligned with the sidebar's content inset, and is today's picker panel moved to a new anchor: the same rows (status, Not paired, attention dot, Open in new window, ⋯ More, pairing for unpaired computers), the directory-error warning, and Remote access settings at the bottom.
- Every computer row gets an icon tile (rounded, raised fill) holding its glyph: laptop, desktop or server as the design shows when the computer's existing `HostInfo` tells them apart, otherwise today's monitor glyph. No protocol or `runner-core` change for this.
- **This computer stands out (M1):** it is always the first row, a divider separates it from the other computers, its tile is green (accent tint and accent glyph), and its subtitle is "This computer". No badge, pill or extra text.
- The computer this window currently shows carries a check at the row's end. On a remote computer's row the check replaces the dot and the action buttons, and its subtitle becomes the connection status, replacing today's footer connection line: "Connected · relay · encrypted" while live, and today's Opening, Reconnecting, Unreachable, Revoked and Forgotten labels and colours otherwise.
- Choosing this computer from a remote window switches the window back, as today.

### Footer in a remote window

- The footer is the same in every window: usage, the host button and the gear, also while a window shows another computer. `render_remote_connection` no longer appears in the footer.
- Usage stays this computer's usage, and the gear opens this computer's Settings; frame 2 draws both and Jason signed it off.

### Connection trouble

The connection line in the remote footer is gone, so its states move to two places:

- the popup's current row, whose subtitle shows the state's label in its colour (see above);
- the host button, which uses the state's colour instead of green: warning for Reconnecting and Unreachable, danger for Revoked, green for Connected and Opening, muted for Forgotten.

The workspace's existing Reconnect / Close Tab and Pair again surfaces are unchanged.

### Keyboard

No new shortcut. The host button is in the footer's tab order next to the gear; Tab and Shift-Tab move through the popup's controls and close it past either end, and Escape closes it, as today. Closing it with Escape, Tab past either end or the host button returns focus to the host button. An outside click also closes it, but the surface clicked keeps its normal focus behaviour: a launcher or terminal takes focus as usual, and only a click on something that takes no focus leaves it on the host button.

### Out of scope

- An all-hosts view or a cross-computer "needs you" list (a possible follow-up).
- An account avatar in the footer.
- Changes to pairing, the picker's row actions, or remote-window behaviour beyond the footer.

## Implementation decisions

These settle details the scope above left open, decided during implementation on 2026-11-05:

- **The gear and Remote access settings in a remote window.** Settings stays unavailable while a window shows another computer, as before. Both the gear and the popup's Remote access settings row therefore switch the window back to this computer first, then open this computer's Settings (the Remote access pane for the row). Today's picker had no Remote access settings row; it is added here because the design draws it.
- **Tile glyphs.** `HostInfo` has no device class, so the glyph comes from its hostname: laptop when it contains "macbook" or "laptop", server when it contains "server", otherwise the monitor.
- **This computer's name signed out.** The account's host name is shown only while signed in, so signed out the row takes this computer's own hostname from `HostInfo`, read once per run, with "This computer" as its subtitle. Nothing is read from the account directory.
- **Current row.** The current computer's row has no selected fill; the check marks it, as the design draws. Its connection subtitle keeps today's labels, which name the computer, except the live label, which now reads "Connected · relay · encrypted".

## Affected docs

- `docs/guide/guides/remote-access.md`: every mention of the sidebar's computer picker and its trigger (the picker paragraphs under pairing and opening another computer, the signed-out note, the keyboard note, and the remote-window paragraph that says the footer shows the relay connection). Name it the host button and its popup.
- `docs/tests/regression/remote.md`: the cases that reference the trigger or the footer connection line, at least REMOTE-LOCAL-04, REMOTE-LOCAL-05, REMOTE-PAIR-01, REMOTE-PAIR-09, REMOTE-PICK-01, REMOTE-PICK-02, REMOTE-PICK-03 and REMOTE-WIN-01.
- `crates/runner-app/i18n/en.toml` and `zh-hans.toml` for any new or changed string, such as the host button's tooltip and "Connected · relay · encrypted".

## Implementation Phases

One change, in `crates/runner-app`:

1. Move the picker's anchor from the brand trigger to a footer host button beside the gear (`remote_client/picker.rs`, `app_shell/sidebar.rs`, `app_shell/settings_button.rs` for the shared footer button style). Show usage and the gear in remote windows.
2. Restyle the popup's rows: icon tiles, this computer first with its divider and green tile, the current computer's check and connection subtitle.
3. Drop the footer connection line and colour the host button by connection state.
4. Tests, guide and regression updates.

## Verification

Headless tests in `runner-app` (`debug_selector` with `VisualTestContext`, as the current picker and footer tests do):

- signed out: footer order usage, host button, gear, with no dot; the popup holds only this computer, a divider and the sign-in row, which opens Settings → Account; no directory fetch, cached computers or errors;
- signed in, local window: footer order usage, host button, gear; no trigger under the brand; the dot follows the attention rule above;
- the popup: this computer first with its divider and "This computer"; the current computer's check; the remote rows unchanged;
- remote window: footer still has usage, host button and gear, no connection line; the button's colour follows each connection state; the current row's subtitle shows each state's label;
- keyboard: Enter/Space/Down open, Tab past either end and Escape close, focus returns to the button.

Live checks in the development app need it signed in to the hosted account with a second paired computer for everything but the signed-out cases; see the mission brief.
