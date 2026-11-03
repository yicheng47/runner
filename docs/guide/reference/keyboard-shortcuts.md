# Keyboard shortcuts

These are Runner's default macOS, Windows and Linux shortcuts. Open **Settings → Keyboard shortcuts** to view, change or unbind supported bindings and reset them to defaults.

Shortcut names, descriptions and command-palette search follow **Settings → General → Language**. The command palette also accepts the original English command names. Changing languages preserves your bindings and saved overrides.

On macOS, `⌘` is Command, `⇧` is Shift, `⌥` is Option and `⌃` is Control. The action depends on focus: pane shortcuts act on the current pane, and mission navigation acts on the current mission. Text fields and the agent's own terminal interface can have additional bindings.

In macOS terminal panes, **Settings → Terminal → Option as Meta key** defaults to **Automatic**: Option sends Meta (Esc) on U.S., U.S. International and ABC layouts, and types characters on other layouts, including `@` and `[` on German. **Both Option keys** keeps Meta on every layout; **Left Option only** and **Right Option only** leave the other Option key available for characters; **Off** uses both for characters. Option chords without printable text, including Option+Enter, Option+Backspace, Option+arrows and Ctrl+Option chords, keep their terminal shortcuts in every mode. Option+←/→ still move by word. When Option types characters, Option+Space types a regular space rather than a non-breaking one. Meta uses the character the key types without Option, so German Option+5 sends Esc followed by `5`.

## Navigation and creation

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| New window | ⇧⌘N | Ctrl+Shift+N |
| New tab: show the launcher page; the tab is created when something starts | ⌘N | Ctrl+N |
| New terminal; fill the focused empty pane or create a tab | ⌘T | Ctrl+Shift+T |
| New mission: show the launcher page's crew step without creating a tab | ⇧⌘M | Ctrl+Shift+M |
| Command palette | ⌘K | Ctrl+K |
| Toggle sidebar | ⌘S | Ctrl+S |
| Toggle mission terminal drawer | ⌥F12 | Alt+F12 |
| Open Settings | ⌘, | Ctrl+, |
| Previous recently viewed page | ⇧⌘[ | Ctrl+Shift+[ |
| Next recently viewed page | ⇧⌘] | Ctrl+Shift+] |
| Select visible sidebar row 1–9 | ⌘1–9 | Ctrl+1–9 |

In Projects, number shortcuts follow the visible tabs and missions, with a linked group's missions right after their chat. In Active, they follow tabs and missions from top to bottom across Agents and Terminals, skipping collapsed headings and the missions of a collapsed group; a split tab opens on the pane that needs attention. Hold the primary modifier to reveal row numbers when the selection binding is enabled.

## Panes and sessions

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Previous chat pane | ⌘[ | Ctrl+[ |
| Next chat pane | ⌘] | Ctrl+] |
| Split right | ⌘D | Ctrl+D |
| Split down | ⇧⌘D | Ctrl+Shift+D |
| Hide focused mission drawer, close focused pane, close chat tab, or close the launcher page | ⌘W | Ctrl+Shift+W |
| Stop focused chat or mission slot | ⇧⌘X | Ctrl+Shift+X |
| Resume focused stopped chat or mission slot | ⇧⌘R | Ctrl+Shift+R |
| Previous mission feed/session tab | ⌘[ | Ctrl+[ |
| Next mission feed/session tab | ⌘] | Ctrl+] |
| Copy terminal selection | ⌘C | Ctrl+C |
| Paste | ⌘V | Ctrl+V |

On Windows and Linux, **Ctrl+C** copies when terminal text is selected; otherwise it reaches the terminal as an interrupt. **Ctrl+W** remains available to terminal programs; Runner uses **Ctrl+Shift+W** to close a pane or tab.

## Find in terminal

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Find in terminal; with the find bar open, select its query | ⌘F | Ctrl+Shift+F |
| Next (older) match | Enter or ⌘G | Enter or F3 |
| Previous (newer) match | ⇧Enter or ⇧⌘G | Shift+Enter or Shift+F3 |
| Close the find bar | Esc | Esc |

Find in terminal can be changed in **Settings → Keyboard shortcuts**; the keys inside the find bar are fixed. On Windows and Linux, **Ctrl+F** still reaches the terminal. Esc closes the find bar only while the bar has focus; once the terminal has focus, Esc goes to the program. See [Find in a terminal](../guides/chats-and-windows.md#find-in-a-terminal).

## Dialogs

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Add a role to a running mission in its dialog | ⌘Enter | Ctrl+Enter |

The dialog confirmation binding is fixed. In the launcher, ↑/↓ select rows, Enter starts the selected row, 1–4 start ranked rows while search is empty, and Esc on the default page returns to the previous tab/view. In a split pane, Esc clears a non-empty search. On the launcher's crew step, ⇧Enter starts an Empty mission, and Esc leaves the step even during a search; Backspace leaves it when the field is empty. On its goal step, Enter starts the mission, ⇧Enter adds a line, and Esc returns to the crews. Digits type normally during a search. The command palette shortcut also works from the launcher's search field, unlike other text fields; closing the palette returns to the search.

## Appearance and window controls

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Zoom in | ⌘+ or ⌘= | Ctrl++ or Ctrl+= |
| Zoom out | ⌘− | Ctrl+− |
| Reset app zoom | ⌘0 | Ctrl+0 |
| Close window and save layout | ⇧⌘W | Alt+F4 |
| Minimize | ⌘M | Ctrl+M |
| Toggle fullscreen | ⌃⌘F | F11 |
| Quit Runner | ⌘Q | Close the last window |
| Quit and Stop Sessions | ⌥⌘Q | No default shortcut |
| Hide Runner | ⌘H | No default shortcut |
| Hide other apps | ⌥⌘H | No default shortcut |

App zoom changes the application controls. Set terminal text size separately in **Settings → Terminal**. Quit behavior follows **Settings → General → When Runner quits**, except the explicit **Quit and Stop Sessions** action.

## Customize

Select an editable shortcut in **Settings → Keyboard shortcuts** and press the new combination. Runner checks conflicts within overlapping contexts. Fixed window and system bindings cannot be reassigned; the tab-selection group can be unbound as a group. Use **Reset all to defaults** to restore the standard map.

These tables follow Runner's shared keymap and its Windows/Linux Ctrl defaults. Agent commands such as slash commands and terminal editing keys belong to the agent itself.

In chat tabs, Stop and Resume shortcuts act only on the focused chat. Terminals have no lifecycle controls, and the drawer shortcut does nothing in chat or terminal tabs.
