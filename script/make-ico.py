#!/usr/bin/env python3
"""Pack 16, 32, 48, and 256 px RGBA PNGs into a Windows icon.

Regenerate from the repository root on macOS:
    icon_tmp=$(mktemp -d)
    for size in 16 32 48 256; do
        sips -z "$size" "$size" assets/icon.png --out "$icon_tmp/icon-$size.png"
    done
    python3 script/make-ico.py assets/icon.ico "$icon_tmp/icon-16.png" \\
        "$icon_tmp/icon-32.png" "$icon_tmp/icon-48.png" "$icon_tmp/icon-256.png"
"""

import argparse
from pathlib import Path
import struct


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("images", type=Path, nargs=4, metavar="PNG")
    args = parser.parse_args()

    images = [path.read_bytes() for path in args.images]
    directory = bytearray(struct.pack("<HHH", 0, 1, len(images)))
    offset = 6 + 16 * len(images)
    for size, png in zip((16, 32, 48, 256), images):
        if (
            png[:8] != b"\x89PNG\r\n\x1a\n"
            or png[12:16] != b"IHDR"
            or struct.unpack(">IIBB", png[16:26]) != (size, size, 8, 6)
        ):
            parser.error(f"expected an {size}x{size} 8-bit RGBA PNG")
        directory.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(png), offset))
        offset += len(png)

    args.output.write_bytes(directory + b"".join(images))


if __name__ == "__main__":
    main()
