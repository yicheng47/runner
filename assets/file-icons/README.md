# File icons

The Files view's icon themes, in Zed's icon-theme format, built by `script/file-icon-themes.py` from the repositories below. Each `*.json.z` is a zlib-compressed JSON document holding one theme and only the SVGs it maps; the commit it was built from is its `source` field. Each bundle also embeds the text of its licence files in its `licenses` field, so the notices ship inside the app on every platform; the Windows installer also installs the `LICENSE-*` files under `file-icons\`.

| Bundle | Theme | Source | Licence |
| --- | --- | --- | --- |
| `catppuccin.json.z` | Catppuccin Latte, with Mocha as a colour map for dark appearances | [catppuccin/zed-icons](https://github.com/catppuccin/zed-icons) | MIT, `LICENSE-catppuccin` |
| `material.json.z` | Material Icon Theme | [zed-extensions/material-icon-theme](https://github.com/zed-extensions/material-icon-theme), whose icons come from the MIT-licensed [Material Icon Theme](https://github.com/material-extensions/vscode-material-icon-theme) | Apache-2.0, `LICENSE-material`; the icons MIT, `LICENSE-material-icons` |
| `minimal.json.z` | Lucide Light, drawn in the tree's text colour | [kjvdven/lucide-zed-icons](https://github.com/kjvdven/lucide-zed-icons), with icons from [Lucide](https://lucide.dev) | MIT for the theme and ISC for Lucide's icons, both in `LICENSE-minimal` |
