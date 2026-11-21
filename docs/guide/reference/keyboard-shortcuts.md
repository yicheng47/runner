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
| Command palette | ⌘K | Ctrl+Shift+P |
| Search files by name in the side panel's folder | ⇧⌘O | Ctrl+Shift+O |
| Toggle session or mission info popover | ⌘I | Ctrl+Shift+I |
| Toggle sidebar | ⌘S | Ctrl+B |
| Switch sidebar view (Projects / Active) | ⌘. | Ctrl+. |
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
| Split right | ⌘D | Ctrl+Shift+D |
| Split down | ⇧⌘D | Alt+Shift+D |
| Maximize the focused pane of a split tab or the focused side panel, or restore it | ⇧⌘↩ | Ctrl+Shift+Enter |
| Close focused mission drawer shell, close focused pane, close chat tab, or close the launcher page | ⌘W | Ctrl+Shift+W |
| Stop focused chat or mission slot | ⇧⌘X | Ctrl+Shift+X |
| Resume focused stopped chat or mission slot | ⇧⌘R | Ctrl+Shift+R |
| Previous mission feed/session tab | ⌘[ | Ctrl+[ |
| Next mission feed/session tab | ⌘] | Ctrl+] |
| Copy terminal selection | ⌘C | Ctrl+C |
| Paste | ⌘V | Ctrl+V |

On Windows and Linux, **Ctrl+C** copies when terminal text is selected and clears the selection, so the next **Ctrl+C** reaches the terminal as an interrupt; without a selection it is an interrupt. **Ctrl+W**, **Ctrl+D**, **Ctrl+K**, **Ctrl+S** and **Ctrl+M** remain available to terminal programs: Runner uses **Ctrl+Shift+W** to close a pane or tab, **Ctrl+Shift+D** and **Alt+Shift+D** to split, **Ctrl+Shift+P** for the command palette and **Ctrl+B** for the sidebar.

## Ctrl keys in terminals

On Windows and Linux, **Settings → Keyboard shortcuts → Ctrl keys in terminals** decides which side gets a bare Ctrl key while a terminal has focus: a chat pane, terminal tab, mission slot, mission drawer or remote terminal.

- **Runner first** (the default) keeps every Runner shortcut working, including in a focused terminal.
- **Terminal first** sends a shortcut made of Ctrl and one key, without Shift or Alt, to the focused terminal instead. With the default keys this frees **Ctrl+N**, **Ctrl+B**, **Ctrl+.**, **Ctrl+[** / **Ctrl+]**, **Ctrl+1–9** and **Ctrl+,**; a shortcut you changed to a bare Ctrl key is freed too. Those rows show an **Outside terminals** tag, and the shortcuts still work everywhere else, such as from the sidebar.

With Terminal first, Ctrl+. sends a plain period in legacy terminal programs. Programs that enable Kitty keyboard disambiguation receive Ctrl+. as a distinct key.

Copy with a selection, paste and zoom work in a terminal in both modes. The setting takes effect immediately. macOS has no such setting because Runner shortcuts use ⌘ rather than Control, so bare Control keys already reach the terminal; on macOS only the three line-editing chords below reach a terminal when no Runner shortcut uses them.

## macOS terminal editing keys

When a terminal pane has focus on macOS (chat pane, terminal tab, mission slot, mission drawer or remote terminal), these standard macOS line-editing chords send control bytes to the terminal:

| Action | macOS | Sends |
| --- | --- | --- |
| Move to start of line | ⌘← | `^A` (`\x01`) |
| Move to end of line | ⌘→ | `^E` (`\x05`) |
| Delete to start of line | ⌘⌫ | `^U` (`\x15`) |

If you bind a Runner shortcut to one of these chords in **Settings → Keyboard shortcuts**, that shortcut wins over the terminal key. Other `⌘` chords continue to route to Runner.

## Find in terminal

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Find in terminal; with the find bar open, select its query | ⌘F | Ctrl+Shift+F |
| Next (older) match | Enter or ⌘G | Enter or F3 |
| Previous (newer) match | ⇧Enter or ⇧⌘G | Shift+Enter or Shift+F3 |
| Close the find bar | Esc | Esc |

Find in terminal can be changed in **Settings → Keyboard shortcuts**; the keys inside the find bar are fixed. On Windows and Linux, **Ctrl+F** still reaches the terminal. Esc closes the find bar only while the bar has focus; once the terminal has focus, Esc goes to the program. See [Find in a terminal](../guides/chats-and-windows.md#find-in-a-terminal).

## Files in the side panel

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Open the selected file in the tree | Space or Return | Space or Enter |
| Next file tab | ⌃Tab | Ctrl+Tab |
| Previous file tab | ⌃⇧Tab | Ctrl+Shift+Tab |
| Close the active file tab | ⌘W | Ctrl+Shift+W |
| Find in the file | ⌘F | Ctrl+F |
| Copy the selected text | ⌘C | Ctrl+C |
| Larger or smaller text in the viewer | ⌘+ / ⌘− | Ctrl+= / Ctrl+− |

These keys act while the side panel's Files view has focus and, except for zoom, are fixed. With focus elsewhere, or no file open, ⌘W (Ctrl+Shift+W) closes the pane as usual. In the file's find bar, Return and ⇧Return step through the matches and Esc closes the bar. The zoom keys change the viewer's text only while the file, its find bar or a rendered page has focus, and follow any rebinding of **Zoom in** and **Zoom out**; in the tree and elsewhere they zoom the app. **Reset zoom** resets the app's zoom everywhere, the viewer included. ⌘-click (Ctrl+click) on a file path in a terminal opens the file in a tab at its line when it is inside the Files view's folder. See [Read a file in the side panel](../guides/chats-and-windows.md#read-a-file-in-the-side-panel).

## Changes in the side panel

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Select previous / next file or commit; in the diff, move the cursor line | ↑ / ↓ or k / j | ↑ / ↓ or k / j |
| Move focus through the changed files, Commits, Base and the checkout controls | Tab / ⇧Tab | Tab / Shift+Tab |
| Open the highlighted file's diff or move focus into the diff | Return | Enter |
| Return focus from the diff to the list (or close an overlay sheet) | Esc | Esc |
| Mark or unmark the selected file as viewed, in the files list | Space | Space |
| Next / previous hunk | ] / [ | ] / [ |
| Scroll the diff sideways | ← / → | ← / → |
| Copy the selected diff text | ⌘C | Ctrl+C |
| Larger or smaller text in the diff | ⌘+ / ⌘− | Ctrl+= / Ctrl+− |
| Maximize or restore the side panel | ⇧⌘↩ | Ctrl+Shift+Enter |

These keys act only while the side panel's Changes view has focus and, except for zoom and maximize, are fixed; when a terminal has focus, `j`, `k`, `Space`, `[` and `]` reach the terminal as usual. Clicking a line in the diff also moves focus into the diff at that line, and **Mark <file> viewed** at the end of a diff is toggled by clicking rather than **Space**. **⇧⌘↩** (**Ctrl+Shift+Enter**) maximizes or restores the side panel across any of its views and follows any rebinding of **Toggle Maximize Pane**. The zoom keys change the viewer's text size only while the diff has focus, shared with Files, and follow any rebinding of **Zoom in** and **Zoom out**. See [Review changes in the side panel](../guides/chats-and-windows.md#review-changes-in-the-side-panel).

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
| Minimize | ⌘M | No default shortcut |
| Toggle fullscreen | ⌃⌘F | F11 |
| Quit Runner | ⌘Q | Close the last window |
| Quit and Stop Sessions | ⌥⌘Q | No default shortcut |
| Hide Runner | ⌘H | No default shortcut |
| Hide other apps | ⌥⌘H | No default shortcut |

App zoom changes the application controls. Set terminal text size separately in **Settings → Terminal**. Quit behavior follows **Settings → General → When Runner quits**, except the explicit **Quit and Stop Sessions** action.

## Customize

**Settings → Keyboard shortcuts** groups shortcuts into **Navigation and creation**, **Panes and sessions**, **Missions**, **Terminal** and **Zoom and window**; search filters the rows within each section and hides a section with no match. Select an editable shortcut and press the new combination. Runner checks conflicts within overlapping contexts. Fixed window and system bindings cannot be reassigned; the tab-selection group can be unbound as a group. Use **Reset all to defaults** to restore the standard map.

These tables follow Runner's shared keymap and its Windows/Linux Ctrl defaults. Agent commands such as slash commands and terminal editing keys belong to the agent itself.

In chat tabs, Stop and Resume shortcuts act only on the focused chat. Terminals have no lifecycle controls, and the drawer shortcut does nothing in chat or terminal tabs.
