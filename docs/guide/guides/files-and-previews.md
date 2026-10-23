# Files and read-only previews

Use the right side panel to inspect the workspace without leaving the conversation. Direct chats keep their **Info** view; Teams keep **Roles** and **Details**. These panel content tabs do not create a new chat tab or terminal pane.

## Browse a workspace

Choose **Files**. A chat uses its working directory, then its project directory, then the default working directory; a Team uses its mission directory with the same project/default fallbacks. If no directory is set, the panel says so instead of browsing an unrelated folder.

Expand folders to load their immediate children. `.git` is hidden. Ignored entries are dimmed when Git can identify them. The tree is read-only: it does not create, rename, move or delete files. Paths and symlinks that resolve outside the workspace cannot be opened here.

Click a file to open a panel tab. Opening the same canonical file again selects its existing tab; files in different folders may have the same name. Hover a file tab to see its path. Scroll the tab strip when it overflows. Close a tab with its ×; closing the active tab selects a neighbour, and closing the last tab shows the new-tab page. The **+** page can reopen the built-in views or show the existing terminal drawer.

Tabs and preview positions belong to the chat or Team within the current window. Switching conversations does not reuse another conversation's workspace. They are not saved across an app restart.

## Read a file

- **Markdown:** 16-DIP horizontal reading margins with rendered headings, lists, quotations, code blocks and tables. Standalone relative image lines can display local raster images inside the workspace; remote images are not fetched.
- **Text, source code and JSON:** read-only text with line numbers and selection/copy. The viewer does not offer editing or syntax highlighting.
- **PNG, JPEG, GIF, WebP and BMP:** raster preview with zoom and fit controls. Animation playback is not promised.
- **PDF, Office, video, binary and other unsupported files:** an explicit fallback rather than a blank or misleading editor. Use the external-open button to open them with the configured editor or system application.

Text previews are limited to 2 MiB and raster images to 20 MiB. Oversized, inaccessible, deleted and unsupported files show an explanation. No file is uploaded or changed by previewing it.

Expanded directories and the selected file refresh in the background while their panel is visible; use **Refresh** to request a fresh read. Background tabs retain their view and refresh when selected. Closing a preview tab does not close or stop its chat session.

Drag the panel divider for more space, or use **Expand** and **Restore**. The rendered panel width is bounded to the available window space so the conversation remains reachable. Collapsing the panel keeps its tabs. Git diffs, review comments, file editing and an embedded browser are not part of this view.
