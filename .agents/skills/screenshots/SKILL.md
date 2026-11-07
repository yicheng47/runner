---
name: screenshots
description: Take README or release screenshots of the Runner development build on macOS — wrap the debug build in a temporary .app so computer use can drive it, set up the scene in the development data, and capture the window at Retina size with the macOS screenshot keys. Use when Jason asks for a new or refreshed product screenshot.
---

# Screenshots

Product screenshots come from the development build, never the installed Runner, so they show the code on `main` and leave Jason's own chats alone. Computer use rejects the bare `target/debug/Runner` executable, so [`script/screenshot-app`](../../../script/screenshot-app) wraps it in a temporary `Runner Screenshot Dev.app` with its own bundle ID and launches it against the development data. Computer use then sets up the scene and presses the macOS screenshot keys, which save the window at full Retina resolution; computer use's own screenshots only come back to the agent, scaled down. This is the same wrapper the [full smoke procedure](../../../docs/tests/full-smoke-test.md#macos-native-control-of-the-debug-executable) uses for QA.

A screenshot request authorizes launching the development app and starting chats or missions for the scene. It does not authorize Git writes, which follow `AGENTS.md`.

## Take a screenshot

1. **Check the development app is free.** `script/qa-lock status` must say free, and no `target/debug/Runner` or wrapper may be running. If a mission's qa holds the lock or another development app is open, wait or ask Jason; never stop someone else's app.
2. **Build** in a checkout of the code to show, usually the root checkout on `main`: `cargo build -p runner-cli -p runner-app`. Work on the README or the guide itself happens in its own branch worktree; point the script at the build with `CARGO_TARGET_DIR=<checkout>/target`.
3. **Bundle and launch** into a fresh scratch folder:

   ```sh
   shot=$(mktemp -d)
   CARGO_TARGET_DIR=<checkout>/target script/screenshot-app bundle "$shot"
   script/screenshot-app launch "$shot"
   ```

   The launch starts the development daemon if none is running, and the saved development chats resume. It strips the calling agent's `CLAUDE*`, `CODEX*`, `RUNNER_*` and `ANTHROPIC_*` variables, which would otherwise reach the daemon and every chat it starts: `CLAUDE_CONFIG_DIR` puts the chat on the agent's own account, and `CLAUDE_CODE_CHILD_SESSION` turns off its transcript, so it shows a warning and cannot resume. If a development daemon was already running from such a launch, quit the app, run `runner-dev daemon stop` and launch again. Emptying the environment entirely with `env -i` does not work: Claude Code launches then hang.
4. **Grant computer use** for the bundle ID `com.wycstudios.runner.screenshot-dev` with `request_access` and `systemKeyCombos`, which the screenshot keys need, then take a screenshot to find the window.
5. **Set up the scene.**
   - Create what the picture needs yourself. `runner-dev chat start --runtime <runtime> --cwd <folder>` starts a chat without driving the launcher; select it in the sidebar afterwards. Never type into, archive or rearrange a development chat or mission you did not create.
   - Keep the sidebar clean with the Active view, the list-filter button beside Search, which shows only running work. Leftover QA projects and other people's chats then stay out of the picture.
   - Show real state. A runtime-only chat started from the CLI sits idle at its welcome screen, which is enough; send no prompts. To show git status, run the chat in a branch worktree that has real uncommitted changes, such as the one holding the README edit the screenshot is for.
   - In the Files view, a source file shows the reader best. Markdown that is mostly HTML, like the README, renders as written, and a file with YAML front matter, like a skill, shows it as a paragraph.
   - Dark theme and the Solid material, the defaults, match the existing images. Hide anything personal: account names in pane headers are fine, private repository names and paths outside the project are not.
6. **Size the window** so every pane has room. The README images are 1684 to 2280 points wide (3368 to 4560 pixels): a single chat fits the smaller size, but the Files view needs about 2280 × 1245, or the chat is squeezed to its minimum. Computer use cannot drag the window's corner outward, because the drag would end on the desktop, so ask Jason to size the window when it has to grow.
7. **Capture** the window without its shadow. Click empty sidebar space so the window is in front, then press ⌘⇧4, then Space, and click the window holding `option`; the modifier must be named `option`, since `alt` does not register and the capture then keeps the shadow margin. The file lands in the folder `defaults read com.apple.screencapture location` names, the Desktop by default, and its name has a narrow no-break space before AM or PM, so find it with `find ~/Desktop -name 'Screenshot*<time>*'`. Check the pixel size with `sips -g pixelWidth -g pixelHeight` and look at the image before copying it to `assets/<name>.png`; capture again rather than editing pixels. Move every capture you made off the Desktop.
8. **Clean up.** Archive the chats you created with `runner-dev session archive <id>`, after `runner-dev session show <id>` confirms each is yours, and any missions with `runner-dev mission archive`. Then `script/screenshot-app quit` and delete the scratch folder. Leave the development daemon running.

## Where screenshots go

- **README:** `assets/<snake_name>.png`, referenced from a feature row in both `README.md` and `README.zh-CN.md`, with alt text in each language that says what the picture shows. A refreshed image keeps its file name so links stay stable.
- **Release notes:** drag the image into the release editor on GitHub. Do not attach it as a release asset, which would add to the files the update feeds and download page list.

A refreshed README image is a doc-only change and goes straight to `main` from the root checkout; a change to the script or this skill goes through a pull request.
