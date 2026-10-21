# Keyboard shortcuts

These are Runner's default macOS, Windows and Linux shortcuts. Open **Settings → Keyboard shortcuts** to view, change or unbind supported bindings and reset them to defaults.

Shortcut names, descriptions and command-palette search follow **Settings → General → Language**. The command palette also accepts the original English command names. Changing languages preserves your bindings and saved overrides.

On macOS, `⌘` is Command, `⇧` is Shift, `⌥` is Option and `⌃` is Control. The action depends on focus: pane shortcuts act on the current pane, and mission navigation acts on the current mission. Text fields and the agent's own terminal interface can have additional bindings.

## Navigation and creation

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| New window | ⇧⌘N | Ctrl+Shift+N |
| New chat; fill the focused empty pane or create a tab | ⌘N | Ctrl+N |
| New terminal; fill the focused empty pane or create a tab | ⌘T | Ctrl+Shift+T |
| New mission | ⇧⌘M | Ctrl+Shift+M |
| Command palette | ⌘K | Ctrl+K |
| Toggle sidebar | ⌘S | Ctrl+S |
| Toggle terminal drawer | ⌥F12 | Alt+F12 |
| Open Settings | ⌘, | Ctrl+, |
| Previous recently viewed page | ⇧⌘[ | Ctrl+Shift+[ |
| Next recently viewed page | ⇧⌘] | Ctrl+Shift+] |
| Select visible sidebar row 1–9 | ⌘1–9 | Ctrl+1–9 |

In Projects, number shortcuts follow the visible tabs and missions. In Active, they follow tabs and missions from top to bottom across Chats, Missions and Terminals, skipping collapsed groups; a split tab opens on the pane that needs attention. Hold the primary modifier to reveal row numbers when the selection binding is enabled.

## Panes and sessions

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Previous chat pane | ⌘[ | Ctrl+[ |
| Next chat pane | ⌘] | Ctrl+] |
| Split right | ⌘D | Ctrl+D |
| Split down | ⇧⌘D | Ctrl+Shift+D |
| Hide focused drawer, close focused split pane, or close chat tab | ⌘W | Ctrl+Shift+W |
| Stop focused chat, terminal or mission slot | ⇧⌘X | Ctrl+Shift+X |
| Resume or restart focused stopped session | ⇧⌘R | Ctrl+Shift+R |
| Previous mission feed/session tab | ⌘[ | Ctrl+[ |
| Next mission feed/session tab | ⌘] | Ctrl+] |
| Copy terminal selection | ⌘C | Ctrl+C |
| Paste | ⌘V | Ctrl+V |

On Windows and Linux, **Ctrl+C** copies when terminal text is selected; otherwise it reaches the terminal as an interrupt. **Ctrl+W** remains available to terminal programs; Runner uses **Ctrl+Shift+W** to close a pane or tab.

## Dialogs

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Start the chat or mission in its launch dialog | ⌘Enter | Ctrl+Enter |
| Select Direct mode in Start a chat | ⌘1 | Ctrl+1 |
| Select Role mode in Start a chat | ⌘2 | Ctrl+2 |

These dialog shortcuts take precedence over tab selection while the launch dialog is open. They are fixed bindings.

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
