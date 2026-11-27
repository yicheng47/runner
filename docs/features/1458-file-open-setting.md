# 1458 — Where ⌘-clicked files open

Tracking issue: [#1458](https://github.com/yicheng47/runner/issues/1458), milestone 0.20.

Design: `runner/desktop/specs/1458-file-open-setting.pen` in the design repo, boards `mlals` (A: the Settings choice), `Z12TDP` (B: a file outside the Files root), `Xayo4` (C: Changes without git), `yKQSi` (D: routing by file type and the cannot-preview placeholder) and `ffz9p` (the decisions). **Status: signed off by Jason on 2026-11-27**, with options A2, B1, C1 and D1; see [Decisions](#decisions). The rejected options are in the design file's history.

## Motivation

⌘-click (Ctrl-click on Windows and Linux) on a file path in a terminal opens it in Runner's side panel only when the panel and Files are on, the session is local, and the path resolves inside the Files root. Anything else opens in the editor saved by the Files viewer's **Open in** chevron, which defaults to **Default app**. Jason hits this often.

Since #634 m7 removed **Settings → General → Open file links in**, the chevron is the only place to choose that editor. With the panel or Files off, nothing in Settings says where ⌘-click goes; a new user finds the choice only after opening a file and noticing the chevron; and files outside the Files root (`/tmp/report.md`, `~/Downloads/data.json`, a sibling repository, anything cited in a chat with no project) always leave Runner, although the preview could show them. #1452's editor links (`vscode://file/…` and the like) follow the same rule, so this choice decides where those land too.

## Scope

### Settings (board A, option A2)

**Settings → General** gains a **File links** section after the Defaults card, with two rows:

- **Open file links in**: a segmented control, **Runner preview | Editor**, default **Runner preview**. Description: "Where ⌘-click on a file path in a terminal opens it. Files the preview can't show open in their default app." On Windows and Linux the description says Ctrl-click.
- **Editor**: a menu of the editors Runner finds on PATH (Zed, VS Code, Cursor) and **Default app**, with an icon for each. An editor whose command is missing stays listed but cannot be picked, with #458's hint (`code not on PATH`). Description: "Opens file links when Editor is chosen or the side panel is off. Also the viewer's Open in button." This is the existing `file_link_editor`: choosing an editor in **Open in** changes this row, and this row changes **Open in**.

When **Runner preview** is chosen but Settings turns off **Show the side panel** or **Files**, ⌘-click opens files in the Editor, and the **Open file links in** row adds an info line: "The side panel is off, so file links open in {editor}. Turn it on in Side panel → Panel." (or "Files is off, …"). ⌘-click never turns the panel back on. A panel that is only collapsed opens on Files, as it does today.

**Upgrading.** Everyone moves to **Runner preview**. The editor already saved by **Open in** becomes **Editor**, so **Open in** keeps working as before. A settings file without the new key reads as **Runner preview**.

### Routing by file type (board D)

⌘-click decides from the path's extension at click time, before any tab opens. With **Runner preview** chosen:

| File | ⌘-click opens | Hover tooltip |
| --- | --- | --- |
| Text, code and Markdown up to 10 MB | Runner preview, at the line | ⌘-click to open in side panel |
| Images: PNG, JPEG, GIF, WebP, SVG, BMP | Runner preview (#1460's image tab) | ⌘-click to open in side panel |
| HTML: `.html`, `.htm` | The default browser | ⌘-click to open in browser |
| Other binary: PDF, Office documents, archives, audio, video | **Default app**, whatever **Editor** names | ⌘-click to open in default app |
| Text over 10 MB | **Editor** | ⌘-click to open in {editor} |
| Any other extension | Runner preview; content the read finds binary opens in **Default app** instead of a tab | ⌘-click to open in side panel |

With **Editor** chosen, the first two rows open in the Editor at the line instead; the other rows route the same way. The size check for the over-10 MB row runs off the UI thread, as the root check does today. Remote sessions keep today's behaviour: their files are on another computer.

### A file outside the Files root (board B, option B1)

With **Runner preview** chosen, a local file outside the Files root opens in the panel too. A path that resolves inside the root opens its usual root tab, so one file never has two tabs.

- **The tab** is keyed by the file's absolute path. The subbar shows the name and its full folder, `~`-shortened (`report.md  /tmp`), then **Open in**. Markdown and SVG keep **Rendered | Source**; images open in #1460's image view.
- **The tree** stays on the project. Nothing in it is selected while an outside tab is active.
- **No Diff.** An outside tab shows Code or Rendered only, with no **Code | Diff** switch, whether or not its folder is in git. Changes stays on the Files root's checkout; to review another repository, choose it in the root picker.
- **Reading** uses `files_read` with the file's folder as `root` and its name as `path`, so `runnerd` needs no new request. An outside file is not under the root's watcher; it is checked with `if_unchanged` on the interval ignored files use (`IGNORED_REFRESH`), so a changed file reloads.
- **A chat without a project** has no Files root (`ChooseFolder`): every file is an outside file, the rail keeps **Choose a folder to show**, and the viewer shows the opened file beside it.

### Changes without git (board C, option C1)

When the Files root is not in a git checkout, including a chat without a project, the **Changes** icon in the activity bar stays in its place at 35% opacity, cannot be pressed, shows no dirty dot, and its tooltip reads "Changes · Not a git repository". It no longer opens the `CHANGES_NOT_GIT` screen; that screen stays only for a root that leaves git while Changes is already shown.

### Files the preview can't show, in a tab (board D, option D1)

Opening from the Files tree or the ⇧⌘O finder keeps today's behaviour: a tab opens and no other app launches (Jason, 2026-11-24). The placeholder keeps today's centred layout and gains a title and a reason:

- **Binary**: `eye-off` icon, "No preview for this file", "{name} is a {kind}, {size}." and "Runner previews text, code, Markdown and images.", with **Open in default app**. `{kind}` comes from the extension ("PDF document", "ZIP archive"), or "binary file" when unknown.
- **Over 10 MB**: `triangle-alert` icon, "Too large to preview", "{name} is {size}." and "Runner previews files up to 10 MB.", with **Open in {editor}**.

#1460's image states (too many pixels, cannot be decoded) are unchanged.

Not in this change: remote sessions; Diff for files outside the root; resolving shortened paths against the Files index (#1459); keyboard shortcuts for the panel (#1482).

## Design

- **Setting.** `AppSettings` gains `file_link_open_in: FileLinkOpenIn { Preview, Editor }`, defaulting to `Preview` when absent. (`FileLinkTarget` is already the clicked path, line and column in `file_links.rs`, so the setting takes another name.) `file_link_editor` stays as it is and backs both the **Editor** row and **Open in**.
- **One decision.** A pure function takes the target path, its extension, its size when known, whether the session is local, the setting, and the panel's state (shown, collapsed, or off in Settings), and returns where the file opens: preview, editor, default app or browser. The click handler, the hover tooltip and the Settings info line all use it, so they cannot disagree. `file_links::link_tooltip` and `FilesView::open_terminal_link` / `link_previews` (`surfaces/files/links.rs`) move onto it.
- **Outside tabs.** The viewer's tab key gains an absolute-path variant beside today's root-relative one (`relative_in` in `surfaces/files/links.rs`). Reads, reload and close follow the variant; find, Rendered | Source and **Open in** work unchanged.
- **Changes disabled.** `activity_bar` (`surfaces/side_panel.rs`) takes whether the root is a git checkout, which the Files view already knows when it resolves the root.

## Implementation steps

1. **Setting and decision.** `file_link_open_in` with its default and migration, the decision function and its tests, and the click and tooltip paths moved onto it, including the panel-off fallback.
2. **Settings UI.** The File links section in General, the Editor menu shared with **Open in**, and the panel-off info line, in English and Chinese.
3. **Outside tabs.** The absolute-path tab key, reading through the file's folder, reload, the subbar folder, and no Diff.
4. **Changes and placeholders.** The disabled Changes icon and the new placeholder copy.
5. **Guide and tests.** The guide pages below and the tests in Verification.

## Guide pages

- `docs/guide/guides/chats-and-windows.md` and `docs/guide/zh-CN/guides/chats-and-windows.md`: the ⌘-click paragraph in "Read a file in the side panel", outside tabs, the placeholder paragraph, and Changes without git.
- `docs/guide/concepts/sessions.md` and `docs/guide/zh-CN/concepts/sessions.md`: where file links open.
- `docs/guide/guides/appearance.md` and `docs/guide/zh-CN/guides/appearance.md`, or whichever page owns **Settings → General** by then: the File links section.

## Verification

### Automated

- Settings: a file without `file_link_open_in` reads as Preview and keeps its `file_link_editor`; both round-trip.
- Decision: every row of the routing table for both settings; the panel shown, collapsed, and off in Settings; Files off; a remote session; an unknown extension; a missing editor command.
- Tooltip: the text for each decision, on macOS and on Windows and Linux.
- Outside tabs: a path inside the root opens the root tab, not an outside one; an outside file reads through its folder, reloads when changed, has no Diff, and keeps one tab per absolute path; a chat with no project opens an outside tab.
- Changes: disabled without git, enabled with git, and the dirty dot hidden while disabled.
- Placeholders: binary and over-10 MB copy, including an unknown kind.
- Gates: `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check` and `python3 script/check-guide-translations.py`, plus a 20-run nextest stress loop for any new test that waits on background work, a watcher or a timer.

### Live checks

On the macOS development app, by `qa` holding `script/qa-lock`, with an owned fixture repository under `/tmp` as the regression suite describes, plus a scratch folder outside it holding a Markdown file, a PNG, a PDF, an `.html` file, a text file over 10 MB and a file with an unknown extension and binary content, and a second git repository beside the fixture:

1. **Settings** (with `SET-PANEL-03`): the File links section in English and Chinese; the Editor menu with one editor's command missing; changing **Editor** changes **Open in** and back; the info line with **Show the side panel** off, then with Files off. Reason: new Settings UI and its link to **Open in**.
2. **Routing with Runner preview** (with `WS-FILES-04`): ⌘-click each scratch file and a fixture path at a line, with the panel shown and collapsed; hover each first and read the tooltip. Each opens where the routing table says. Reason: the routing changes for every type outside the root and for binary types inside it.
3. **Outside tabs**: a scratch file, a file in the second repository, and the same fixture file by absolute path. The tree stays on the fixture, the subbar shows the full folder, there is no Diff, editing the scratch file on disk reloads its tab, and the fixture file opens its usual tab. Reason: outside tabs are new.
4. **Editor and panel off**: choose **Editor**, ⌘-click a text file and a PNG (both open in the editor at the line) and a PDF (default app); then choose **Runner preview** with the side panel off and ⌘-click a text file (editor). Reason: the setting and its fallback.
5. **No project and Changes**: in a chat without a project, ⌘-click a scratch file; the rail shows **Choose a folder to show**, Changes is dimmed with its tooltip and cannot be pressed; choose a non-git folder as the root, then the fixture, and Changes turns back on. Reason: the disabled Changes state is new.
6. **Placeholders** (with `WS-FILES-03` and `WS-CHANGES-08`): open the PDF and the over-10 MB file from the tree and from ⇧⌘O. The new copy shows and no other app launches; Changes' own placeholders are unchanged. Reason: new placeholder copy.
7. **Upgrading**: start the development app from a settings file that has `file_link_editor` set to VS Code and no `file_link_open_in`. Settings shows **Runner preview** and **Editor** VS Code, and **Open in** still shows VS Code. Reason: the migration.
8. **Windows** (with `TERM-LINK-02`): repeat checks 2 and 4 with Ctrl-click, including drive-letter paths and a file outside the root. Reason: routing is platform-independent code, but default-app and browser launches differ on Windows.

Selected regression cases: `SET-PANEL-03`, `WS-FILES-03`, `WS-FILES-04`, `WS-CHANGES-08`, `TERM-LINK-02`.

## Decisions

Signed off by Jason on 2026-11-27, answering each open question in the issue. The decisions board is `ffz9p`.

- **Placement and number of settings: A2.** Two rows in a **File links** section in **General**, which applies with the side panel off. Rejected: A1, one **Open file links in** menu with Runner preview at the top, which leaves no editor in Settings while Runner preview is chosen; A3, the same rows under **Side panel → Files**, where people looking in General won't find them.
- **Runner preview with the panel off.** A collapsed panel opens on Files; a panel or Files turned off in Settings sends ⌘-click to the Editor, and the row says so. ⌘-click never turns the panel on.
- **The tree for an outside file: B1.** The tree stays on the project and an outside tab has no Diff. Rejected: B2, moving the panel's root to the file's folder or checkout; B3, an **Outside** group at the top of the tree.
- **Changes without git: C1.** Dimmed in place with a tooltip. Rejected: C2, hiding the icon.
- **The placeholder: D1.** Today's centred layout with a title and a reason. Rejected: D2, a left-aligned file card with **Show in Finder**.
- **Upgrading.** Everyone moves to Runner preview; the editor picked in **Open in** becomes **Editor**.
