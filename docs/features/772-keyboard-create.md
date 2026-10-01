# 772 — Start chats, terminals and missions from the keyboard

> Tracking issue: [#772](https://github.com/yicheng47/runner/issues/772)
> Priority: P2, 0.12. Platforms: macOS and Windows.
> Status: drafted 2026-10-01; open questions settled the same day with the recommended answers (see Decisions), and handed to a codex pair mission on `feat/772-keyboard-create`.

## Motivation

Start a chat (⌘N) is the main way into Runner, and it is built for the mouse. Here is what the keyboard does today, read from `surfaces/start_chat.rs` and `ui/overlay.rs`:

- **Cancel.** Esc closes the modal from any control in it, through `Modal`'s key handler. If a select's menu is open, Esc closes the menu first. Nothing on the Cancel button shows this.
- **Start chat.** Enter starts the chat only when focus is in a text field (Chat name, Working directory, Model), through `on_start_chat_key_down`. On the role or agent picker, or on Runtime, Effort or Speed, Enter opens the menu. On a Direct/Role segment it switches mode, and on Reset it resets. So whether Enter starts the chat depends on where focus is, no key starts it from every control, and the button shows no key.
- **Direct | Role.** These are two separate tab stops, each picked with Enter or Space. Focus opens in Chat name, below the cards, so reaching the switch takes Shift-Tab back through every control and Reset. There are no arrow keys and no shortcut.
- **Workspace keys go through.** GPUI matches key bindings before key-down listeners, and the modal declares no key context. So while it is open, ⌘1–9, ⌘D and ⌘[ still act on the workspace behind it.
- **The other two kinds.** The sidebar's + menu offers New chat, New mission and New terminal, but only New chat has a key. New terminal is a ⌘K palette command, and it opens in the terminal drawer, not as a tab like the + menu's. New mission has neither a key nor a palette command; you reach it with the mouse, from the + menu or a crew page.

## Proposal

### 1. Cancel and Start chat

- **⌘↵ starts the chat from anywhere in the modal** (Ctrl+Enter on Windows): from a text field, a picker, a select with its menu open, Reset, or a footer button. An open menu closes without picking its highlighted row, and the chat starts with the values the controls show. ⌘↵ does nothing when Start chat is disabled (no role, no enabled agent), while the chat is starting, or while an IME is composing.
- **Plain Enter and Esc keep their current behavior.** Enter still starts the chat from a text field, opens a picker, and presses a focused button. Esc still closes an open menu, otherwise the modal. Neither key changes meaning.
- **The footer shows the keys.** Cancel shows `esc` and Start chat shows `⌘↵`, as trailing keycaps in the faint meta style the sidebar's New chat row uses for ⌘N. The labels come from the keymap, so Windows shows `Ctrl+↵`.
- **Start mission gets the same ⌘↵ and keycaps.** It shares `Modal` and has the same split between Enter in a field and Enter on a picker.
- **Mechanism.** The modal root declares a `StartChat` key context (and Start mission a `StartMission` one), with a confirm action bound to `cmd-enter` in it. Bindings dispatch before key-down listeners, so `StyledSelect::on_key_down`, which matches `enter` without checking modifiers, never sees ⌘↵. These bindings are fixed and registered in the modal's context the way the terminal's Copy is, so Settings → Keymap does not list them.

### 2. Direct | Role

- **One tab stop.** The switch becomes a segmented control with a single tab stop: ←/→ move between Direct and Role while it has focus, and Enter and Space keep working.
- **⌘1 Direct, ⌘2 Role** from anywhere in the modal, shown as faint keycaps inside the segments. The fixed bindings require the `StartChat` context anywhere above the focused control and load after workspace and user bindings, so they also win from a nested `TextInput` context. Global Select tab 1/2 bindings and keys stay unchanged.
- **Switching moves focus to that mode's picker** (the role picker or the agent picker), because picking who the chat is with is why you switched. The same switch by mouse preserves focus when that control remains visible; when the old picker or another focused control disappears, focus moves to the new mode’s picker (or Chat name when it is unavailable).
- **Initial focus goes to the remembered mode's picker,** not Chat name. The quickest paths become: ⌘N ⌘↵ for the defaults, ⌘N ↵ ↓↓ ↵ ⌘↵ for a different role or agent, and ⌘N ⌘2 ↵ ↓ ↵ ⌘↵ from Direct mode. Chat name is optional and comes one Tab later. When the picker cannot take focus (no roles, no enabled agents), focus opens in Chat name as today.

### 3. Chats, terminals and missions: three keys, no merged form

Recommendation: give each kind its own key, and keep each kind's form or no form. Do not put one "New…" modal with a kind switch in front of them.

| Key (macOS / Windows) | Action | Form |
| --- | --- | --- |
| ⌘N / Ctrl+N | New chat (unchanged) | Start a chat |
| ⌘T / Ctrl+Shift+T | New terminal | None: fills the focused empty pane, otherwise opens a new tab, in the active project's directory or the default directory, as the + menu's New terminal does today |
| ⇧⌘M / Ctrl+Shift+M | New mission | Start mission, scoped to the active project, as the + menu's New mission does today |

- All three are rebindable keymap entries (`new-chat` already exists; add `new-terminal` and `new-mission`). The + menu rows show their keys, and ⌘K gains New chat and New mission commands beside its existing New terminal, which keeps its own placement (#574: drawer on a chat tab).
- On Windows, New terminal is Ctrl+Shift+T, as in Windows Terminal, so Ctrl+T stays transpose-characters in shells. This follows #725, which kept Ctrl+W for shells and made close tab Ctrl+Shift+W.

Why not one modal with Chat | Terminal | Mission at the top:

- A terminal needs no form. A modal would add a step to the quickest action.
- A mission asks for different things (crew, goal) and opens a different surface. A kind switch above Direct | Role would nest one segmented control inside another.
- With three keys, each form stays short. The + menu and the palette list the keys, so they are easy to find.

The alternative was a small chooser on ⌘N (Chat / Terminal / Mission, then the form). It adds a keystroke to the most common action, so the proposal leaves it out.

## Non-goals

- Changing what the forms ask for, how sessions start, or where a chat lands.
- Type-to-filter in the role and agent pickers; file it separately if wanted.
- Stopping other workspace shortcuts (⌘3–9, ⌘D, ⌘W) from acting behind an open modal; only ⌘1 and ⌘2 are taken over here, by the mode switch.
- Keyboard work on modals other than Start a chat and Start mission.

## Decisions (2026-10-01)

Jason asked for the mission on the drafted spec, so its open questions take the recommended answers:

1. **Initial focus:** the remembered mode's picker, falling back to Chat name when the picker cannot take focus.
2. **⌘T on the mission route:** a chat-surface terminal tab like ⌘N's chat, switching to the chat surface; the mission drawer keeps its own + and ⌥F12.
3. **The palette's New terminal** is unchanged; the palette shows no keys, so it does not contradict ⌘T.
4. **Mission key:** ⇧⌘M (Ctrl+Shift+M on Windows).
5. **Type-ahead in pickers:** out of scope.
6. **Report check:** the mission adds tests that open Start a chat through the New chat action with a terminal pane focused, then check that Esc closes it and Enter in Chat name starts the chat. If either fails, fixing that comes first.

## Implementation finding (2026-10-01)

A GPUI 0.3.7 dispatch regression test found that the proposed depth-only override of global Select tab 1/2 bindings does not hold: bindings with no context match at the deepest context depth, and later registration wins equal-depth ties (`keymap.rs::binding_enabled` and `bindings_for_input` in gpui-pre). The implementation registers fixed modal bindings after workspace and user bindings, using the stack-wide `!(!StartChat)` / `!(!StartMission)` predicates to match at the focused control’s depth, including under `TextInput`. This protects both confirm and mode keys against user overrides while leaving all global bindings unchanged. Tests exercise ⌘1/⌘2 from pickers, text fields, open menus and the switch, plus the workspace keys after closing the modal. Confirm bindings still dispatch before control key-down listeners, as verified from open selects and the mission goal textarea, including when a global user binding uses the confirm key. Review also identified that preserving focus on a picker removed by a mouse mode change leaves the modal without a live focus node; mouse switches now preserve surviving controls and move focus from removed controls to the new picker. Start mission renders alongside Start chat on every route, including over Settings, as Jason confirmed during review.

## Implementation phases

1. **Design:** no Pencil frames; the keycaps reuse the sidebar New chat row's shortcut style and the + menu's existing `UiMenuItem::shortcut`, and Jason judges the look in his smoke test.
2. **Modal keys:** the `StartChat` and `StartMission` key contexts with confirm and mode actions, the single-stop segmented switch, initial focus, the focus move on a mode switch, and keycaps from the keymap.
3. **Create shortcuts:** `NewTerminal` and `NewMission` actions with keymap entries, the + menu keys, and palette commands. Keep the empty-pane rule shared with ⌘N.

## Verification

- `runner-app` tests: ⌘↵ starts the chat from a text field, a closed picker, an open menu without taking its highlight, Reset and Cancel; it does nothing when Start chat is disabled, while starting, or while composing. ⌘1/⌘2 switch mode and focus the picker without changing the active tab behind the modal. ←/→ work on the switch, and the tab order holds. ⌘↵ works in Start mission. The keymap defaults include the Windows mapping test. ⌘T fills the focused empty pane, otherwise opens a new tab, and ⇧⌘M opens Start mission for the active project.
- Workspace Clippy is clean.
- Jason's smoke test on macOS and Windows, with no mouse: ⌘N ⌘2, pick a role, ⌘↵; ⌘T; ⇧⌘M, pick a crew, ⌘↵.

## Relevant code

- `crates/runner-app/src/surfaces/start_chat.rs`: `on_start_chat_key_down`, `render_mode_button`, `start_chat_focus_order`, initial focus in `open_start_chat_modal`, `new_terminal` and `new_terminal_tab`.
- `crates/runner-app/src/surfaces/start_mission.rs`: the Start mission modal.
- `crates/runner-app/src/ui/overlay.rs`: `Modal`'s Esc and Tab handling.
- `crates/runner-app/src/ui/select.rs`: `StyledSelect::on_key_down`, where Enter ignores modifiers.
- `crates/runner-app/src/keymap.rs`: keymap entries, Windows defaults, and binding contexts.
- `crates/runner-app/src/surfaces/sidebar/menus.rs`: the + menu's create entries; `surfaces/sidebar/elements.rs` for the New chat row's keycap style.
- `crates/runner-app/src/surfaces/command_palette.rs`: palette commands.
