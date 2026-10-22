# Appearance

Choose your personal avatar and how Runner follows light and dark mode, then set the app and terminal palettes independently. Text size and window material have their own controls.

## Display language

Open **Settings → General** and use the **Language** dropdown to choose **System**, **English**, or **简体中文**. Changes apply immediately to all open Runner windows, including menus and search labels. Drafts, terminal contents and running sessions are preserved. Runner remembers your choice after relaunch.

**System** is the default for new and existing settings. It follows the first preferred operating-system UI language: Chinese resolves to Simplified Chinese; other or unavailable languages resolve to English. Runner checks this at startup, when you select System, and when a window becomes active. System appears as **跟随系统** in Chinese; the two explicit choices retain their own language names.

The setting covers Runner's interface and date displays. Agent output, user-authored names and prompts, backend error details, CLI output and documentation keep their original text. Native operating-system dialogs follow the operating system's language.

## My avatar

Open **Settings → Appearance → My avatar** to edit the picture used for your human-authored mission messages, labelled **You**. This is your personal appearance, not an AI role: changing it does not change a role, crew slot, event author or agent configuration.

Choose **Initials** or **Pixel art** and a theme color. Automatic uses the existing human color; the named color presets and a custom `#RRGGBB` value let you choose your own. The selected local style and color also provide the fallback while an image loads or cannot be read.

You can import a PNG image up to 8 MiB and 2048 × 2048 pixels. Runner copies it into its personal-avatar storage, leaving your original file untouched; moving the original later does not remove the imported avatar. Choosing a local style clears the image selection. Runner removes superseded imported copies after the new choice saves successfully. Your choice is saved with the app settings and shared by open Runner windows.

## Theme and palettes

Open **Settings → Appearance**. **Theme** follows the operating system or pins Runner to light or dark mode. Each mode has two selections:

- **App palette** colors the sidebar, panels and controls. Light mode offers Runner Light and Catppuccin Latte; dark mode offers Runner and Catppuccin Mocha.
- **Terminal palette** colors the terminal grid. Runner, Catppuccin Mocha and Rosé Pine Dawn are available independently of the app palette.

Choose both modes once, then switch between them without rebuilding the pairing. Claude Code follows Runner's light/dark switch live; other agents may use their own theme controls for colors they draw themselves.

## Window material

On macOS, **Window material** offers **Glass** and **Solid**. Glass adds depth to the sidebar and window chrome. Runner respects macOS **Reduce Transparency**, which makes Glass use the solid appearance.

Windows uses Solid and does not show the macOS material selector.

## Size and terminal text

**Settings → General → App zoom** scales the application controls. Use **⌘+** / **Ctrl++** to zoom in, **⌘−** / **Ctrl+−** to zoom out, and **⌘0** / **Ctrl+0** to reset.

The terminal canvas has a separate text size. Open **Settings → Terminal** to choose its font, font size and cursor style. If the sidebar is comfortable but the agent text is too small, change the terminal font size instead of app zoom.
