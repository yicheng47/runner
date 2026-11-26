#!/usr/bin/env python3
"""Pack 16, 24, 32, 48, and 256 px RGBA PNGs into a Windows icon.

`assets/icon.ico` uses full-bleed Windows tile framing (no macOS Dock gutter)
so taskbar icons match neighboring apps at 16-48 px. The PNGs are exported at
1x from the `Export · Tile <size>` frames of the `R7 · Production Exports`
board in `runner/desktop/specs/app-icon-upgrade.pen` (yeelinks_design), where
the 16, 24, and 32 px tiles carry their own small-size tuning:

    python3 script/make-ico.py assets/icon.ico "$icon_tmp/icon-16.png" \\
        "$icon_tmp/icon-24.png" "$icon_tmp/icon-32.png" \\
        "$icon_tmp/icon-48.png" "$icon_tmp/icon-256.png"
"""

import argparse
from pathlib import Path
import struct

SIZES = (16, 24, 32, 48, 256)


def pack_ico(parser: argparse.ArgumentParser, output: Path, image_paths: list[Path]) -> None:
    images = [path.read_bytes() for path in image_paths]
    directory = bytearray(struct.pack("<HHH", 0, 1, len(images)))
    offset = 6 + 16 * len(images)
    for size, png in zip(SIZES, images):
        if (
            png[:8] != b"\x89PNG\r\n\x1a\n"
            or png[12:16] != b"IHDR"
            or struct.unpack(">IIBB", png[16:26]) != (size, size, 8, 6)
        ):
            parser.error(f"expected an {size}x{size} 8-bit RGBA PNG")
        directory.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(png), offset))
        offset += len(png)

    output.write_bytes(directory + b"".join(images))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output", type=Path)
    parser.add_argument("images", type=Path, nargs="+", metavar="PNG")
    args = parser.parse_args()

    if len(args.images) != len(SIZES):
        parser.error(f"expected {len(SIZES)} PNGs ({', '.join(map(str, SIZES))})")
    pack_ico(parser, args.output, args.images)


if __name__ == "__main__":
    main()
