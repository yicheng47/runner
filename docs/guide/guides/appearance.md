# Appearance

Choose how Runner follows light and dark mode, then set the app and terminal palettes independently: the app palette in Appearance, the terminal palette with the terminal's other settings. Text size and window material have their own controls.

## Display language

Open **Settings → General** and use the **Language** dropdown to choose **System**, **English**, or **简体中文**. Changes apply immediately to all open Runner windows, including menus and search labels. Drafts, terminal contents and running sessions are preserved. Runner remembers your choice after relaunch.

**System** is the default for new and existing settings. It follows the first preferred operating-system UI language: Chinese resolves to Simplified Chinese; other or unavailable languages resolve to English. Runner checks this at startup, when you select System, and when a window becomes active. System appears as **跟随系统** in Chinese; the two explicit choices retain their own language names.

The setting covers Runner's interface and date displays. Agent output, user-authored names and prompts, backend error details, CLI output and documentation keep their original text. Native operating-system dialogs follow the operating system's language.

## Theme and palettes

Open **Settings → Appearance**. **Theme** follows the operating system or pins Runner to light or dark mode. Each mode has two palettes:

- **App palette**, in **Settings → Appearance**, colors the sidebar, panels and controls. Light mode offers Runner Light and Catppuccin Latte; dark mode offers Runner and Catppuccin Mocha. Runner Light uses cool gray surfaces with stronger borders.
- **Terminal palette**, chosen as **Light mode palette** and **Dark mode palette** under **Settings → Terminal → Colors**, colors the terminal grid. Runner, Catppuccin Mocha and Rosé Pine Dawn are available independently of the app palette. Rosé Pine Dawn uses a muted cream background in Runner. The Appearance preview shows each mode with both of its palettes.

Choose both modes once, then switch between them without rebuilding the pairing. Claude Code follows Runner's light/dark switch live; other agents may use their own theme controls for colors they draw themselves.

Agents that draw their own colors, such as Codex, choose light or dark styling once, when they start, from the terminal palette. Chats that resume when Runner starts get the terminal palette you last used. If **Theme** follows the operating system and it switched between light and dark while Runner was closed, stop and resume a chat whose colors look mismatched. Terminals on another computer use that computer’s terminal palette; see [Remote access](remote-access.md#pair-and-open-another-computer-in-the-app).

## Window material

On macOS, **Window material** offers **Glass** and **Solid**. Glass adds depth to the sidebar and window chrome. Runner respects macOS **Reduce Transparency**, which makes Glass use the solid appearance.

Windows uses Solid and does not show the macOS material selector.

## Size and terminal text

**Settings → General → App zoom** scales the application controls. Use **⌘+** / **Ctrl++** to zoom in, **⌘−** / **Ctrl+−** to zoom out, and **⌘0** / **Ctrl+0** to reset.

The terminal canvas has a separate text size. Open **Settings → Terminal**, grouped into **Text**, **Colors**, **Cursor**, **Keyboard** (macOS only) and **Scrollback & selection**. **Font size** runs from 8 to 32 px, and **Line height** sets row spacing from 1.0 to 1.6 times the font size; open terminals lay out again and the shell's row count follows. The searchable **Font family** list groups JetBrains Mono under **Bundled** and the monospace fonts on your computer under **Installed**. A missing saved font is marked **Not installed** and uses JetBrains Mono until you reinstall it or choose another font. Font changes apply to open terminals and the Appearance preview. If the sidebar is comfortable but the agent text is too small, change the terminal font size instead of app zoom.

**Blink cursor**, off by default, blinks the cursor of the focused terminal; a program that sets its own cursor style still decides for itself. **Scrollback** keeps 1,000, 5,000, 10,000 (the default), 25,000 or 50,000 lines above the screen in every terminal. More lines use more memory in every open session, and lowering it drops the oldest lines of open terminals. **Copy on select**, off by default, copies text to the clipboard as soon as you finish selecting it with the mouse, the same text ⌘C or Ctrl+C would copy, and leaves the selection in place.

On macOS, the **Keyboard** section's **Option as Meta key** controls whether Option sends shell shortcuts or types your keyboard layout's characters. **Automatic**, the default, uses Meta on U.S., U.S. International and ABC layouts and types characters on other layouts. Choose **Both Option keys**, **Left Option only**, **Right Option only** or **Off** to override it. A layout switch takes effect on the next keypress.
