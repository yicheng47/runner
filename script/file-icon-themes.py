#!/usr/bin/env python3
"""Builds the Files view's bundled icon themes in assets/file-icons/.

Each bundle is a zlib-compressed JSON document holding one theme in Zed's
icon-theme format, unchanged, and only the SVG files that theme maps, keyed
by the path the theme names. Colour icons have their root size raised so
GPUI rasterizes them sharply; Catppuccin's Mocha icons are kept as a colour
map over Latte's, since the two sets differ only in colour.

Each theme's licence is copied beside its bundle as LICENSE-<name> and its
text embedded in the bundle's `licenses` field, so it ships inside the app
on every platform. LICENSE-material-icons, the MIT licence of the Material
Icon Theme the Zed port's icons come from, is kept by hand and embedded with
Material's own.

Usage, with each repository cloned at the commit recorded in the bundle:

    script/file-icon-themes.py <catppuccin/zed-icons> \
        <zed-extensions/material-icon-theme> <kjvdven/lucide-zed-icons>
"""

import json
import re
import shutil
import subprocess
import sys
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "assets" / "file-icons"
RASTER_SIZE = 48
HEX = re.compile(r"#[0-9a-fA-F]{6}\b")


def theme_paths(theme):
    paths = set()

    def walk(value):
        if isinstance(value, dict):
            for item in value.values():
                walk(item)
        elif isinstance(value, str) and value.endswith(".svg"):
            paths.add(value)

    walk(theme)
    return sorted(paths)


def raster_size(svg):
    """Sets the root element's width and height, keeping its viewBox."""
    match = re.search(r"<svg\b[^>]*>", svg)
    tag = match.group(0)
    attrs = dict(re.findall(r'\s([\w:-]+)="([^"]*)"', tag))
    if "viewBox" not in attrs:
        width = attrs.get("width", "16").rstrip("px")
        height = attrs.get("height", "16").rstrip("px")
        tag = tag.replace("<svg", f'<svg viewBox="0 0 {width} {height}"', 1)
    tag = re.sub(r'\s(width|height)="[^"]*"', "", tag)
    tag = tag.replace("<svg", f'<svg width="{RASTER_SIZE}" height="{RASTER_SIZE}"', 1)
    return svg[: match.start()] + tag + svg[match.end() :]


def minify(svg):
    svg = re.sub(r"<\?xml[^>]*\?>", "", svg)
    svg = re.sub(r"<!--.*?-->", "", svg, flags=re.S)
    return re.sub(r">\s+<", "><", svg).strip()


def source(repo, url):
    commit = subprocess.check_output(["git", "-C", repo, "rev-parse", "HEAD"], text=True)
    return f"{url} @ {commit.strip()}"


def with_default(theme, path):
    """Zed falls back to its own theme's file icon when a theme has no
    `default`; Runner has no theme behind, so the set's generic file stands in."""
    theme["file_icons"].setdefault("default", {"path": path})
    return theme


def icons(repo, theme, color):
    loaded = {}
    for path in theme_paths(theme):
        svg = minify((repo / path).read_text())
        loaded[path] = raster_size(svg) if color else svg
    return loaded


def write(name, bundle, licence):
    OUT.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(licence, OUT / f"LICENSE-{name}")
    bundle["licenses"] = [
        path.read_text() for path in sorted(OUT.glob(f"LICENSE-{name}*"))
    ]
    data = json.dumps(bundle, separators=(",", ":"), sort_keys=True).encode()
    (OUT / f"{name}.json.z").write_bytes(zlib.compress(data, 9))
    print(f"{name}: {len(bundle['icons'])} icons, {len(data)} bytes, {len(zlib.compress(data, 9))} compressed")


def catppuccin(repo):
    family = json.loads((repo / "icon_themes/catppuccin-icons.json").read_text())
    themes = {theme["name"]: theme for theme in family["themes"]}
    latte = with_default(themes["Catppuccin Latte"], "./icons/latte/_file.svg")
    mocha = with_default(themes["Catppuccin Mocha"], "./icons/mocha/_file.svg")
    loaded = icons(repo, latte, True)
    colors = {}
    for path in loaded:
        light = (repo / path).read_text()
        dark = (repo / path.replace("/latte/", "/mocha/")).read_text()
        for a, b in zip(HEX.findall(light), HEX.findall(dark)):
            colors.setdefault(a.lower(), {}).setdefault(b.lower(), 0)
            colors[a.lower()][b.lower()] += 1
    dark_colors = {a: max(b, key=b.get) for a, b in colors.items()}
    dark_icons = {}
    for path, svg in loaded.items():
        recoloured = HEX.sub(lambda m: dark_colors.get(m.group(0).lower(), m.group(0)), svg)
        expected = raster_size(minify((repo / path.replace("/latte/", "/mocha/")).read_text()))
        if recoloured.lower() != expected.lower():
            dark_icons[path] = expected
    assert theme_paths(mocha) == [p.replace("/latte/", "/mocha/") for p in theme_paths(latte)]
    write(
        "catppuccin",
        {
            "source": source(repo, "https://github.com/catppuccin/zed-icons"),
            "theme": latte,
            "icons": loaded,
            "dark_colors": dark_colors,
            "dark_icons": dark_icons,
        },
        repo / "LICENSE",
    )


def material(repo):
    family = json.loads((repo / "icon_themes/material-icon-theme.json").read_text())
    theme = family["themes"][0]
    with_default(theme, theme["file_icons"]["file"]["path"])
    write(
        "material",
        {
            "source": source(repo, "https://github.com/zed-extensions/material-icon-theme"),
            "theme": theme,
            "icons": icons(repo, theme, True),
        },
        repo / "LICENSE",
    )


def minimal(repo):
    family = json.loads((repo / "icon_themes/lucide.json").read_text())
    theme = next(theme for theme in family["themes"] if theme["name"] == "Lucide Light")
    write(
        "minimal",
        {
            "source": source(repo, "https://github.com/kjvdven/lucide-zed-icons"),
            "theme": theme,
            "icons": icons(repo, theme, False),
            "monochrome": True,
        },
        repo / "LICENSE",
    )


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    catppuccin(Path(sys.argv[1]))
    material(Path(sys.argv[2]))
    minimal(Path(sys.argv[3]))
