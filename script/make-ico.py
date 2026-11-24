#!/usr/bin/env python3
"""Render and pack 16, 24, 32, 48, and 256 px RGBA PNGs into a Windows icon.

`assets/icon.ico` uses full-bleed Windows tile framing derived from
`runner/desktop/app-icon.svg` (the 824x824 `#0E0E10` tile cropped onto a full
1024x1024 canvas with `rx = 232` = `186.6875 * 1024 / 824`, and the 72-unit
Trail chevrons scaled to `unit = 9.25` = 48.1% of the tile width in `#00FF9C`
at opacities 0.28, 0.55, and 1.0), rendered directly at each target size so
Windows taskbar icons stay sharp and match neighboring apps at 16-48 px.

Regenerate from scratch on macOS:
    python3 script/make-ico.py --render assets/icon.ico

Or pack five pre-rendered 8-bit RGBA PNGs (16, 24, 32, 48, 256 px):
    python3 script/make-ico.py assets/icon.ico "$icon_tmp/icon-16.png" \\
        "$icon_tmp/icon-24.png" "$icon_tmp/icon-32.png" \\
        "$icon_tmp/icon-48.png" "$icon_tmp/icon-256.png"
"""

import argparse
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

SIZES = (16, 24, 32, 48, 256)

SWIFT_RENDERER = """\
import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

func renderWindowsIcon(size: Int) -> CGImage {
    let cs = CGColorSpace(name: CGColorSpace.sRGB)!
    let ctx = CGContext(
        data: nil, width: size, height: size,
        bitsPerComponent: 8, bytesPerRow: size * 4,
        space: cs,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    ctx.setAllowsAntialiasing(true)
    ctx.setShouldAntialias(true)
    ctx.translateBy(x: 0, y: CGFloat(size))
    ctx.scaleBy(x: CGFloat(size) / 1024.0, y: -CGFloat(size) / 1024.0)

    let tilePath = CGPath(
        roundedRect: CGRect(x: 0, y: 0, width: 1024, height: 1024),
        cornerWidth: 232, cornerHeight: 232,
        transform: nil
    )
    ctx.setFillColor(CGColor(colorSpace: cs, components: [14.0/255.0, 14.0/255.0, 16.0/255.0, 1.0])!)
    ctx.addPath(tilePath)
    ctx.fillPath()

    let chevrons: [(x0: CGFloat, sw: CGFloat, alpha: CGFloat)] = [
        (308.5,  83.25, 0.28),
        (447.25, 83.25, 0.55),
        (586.0,  88.8,  1.00),
    ]
    ctx.setLineCap(.round)
    ctx.setLineJoin(.round)
    for c in chevrons {
        let p = CGMutablePath()
        p.move(to: CGPoint(x: c.x0, y: 382.5))
        p.addLine(to: CGPoint(x: c.x0 + 129.5, y: 512.0))
        p.addLine(to: CGPoint(x: c.x0, y: 641.5))
        ctx.addPath(p)
        ctx.setLineWidth(c.sw)
        ctx.setStrokeColor(CGColor(colorSpace: cs, components: [0.0, 1.0, 156.0/255.0, c.alpha])!)
        ctx.strokePath()
    }
    return ctx.makeImage()!
}

let outDir = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
for arg in CommandLine.arguments.dropFirst(2) {
    let size = Int(arg)!
    let img = renderWindowsIcon(size: size)
    let url = outDir.appendingPathComponent("icon-\\(size).png")
    let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, img, nil)
    precondition(CGImageDestinationFinalize(dest), "failed to write \\(url.path)")
}
"""


def render_pngs(parser: argparse.ArgumentParser, out_dir: Path) -> list[Path]:
    if sys.platform != "darwin" or shutil.which("swift") is None:
        parser.error("--render requires macOS with swift installed")
    script = out_dir / "render.swift"
    script.write_text(SWIFT_RENDERER, encoding="utf-8")
    subprocess.run(
        ["swift", str(script), str(out_dir), *(str(size) for size in SIZES)],
        check=True,
    )
    return [out_dir / f"icon-{size}.png" for size in SIZES]


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
    parser.add_argument(
        "--render",
        action="store_true",
        help="render the Windows Trail icon directly at 16, 24, 32, 48, and 256 px (macOS) before packing",
    )
    parser.add_argument("output", type=Path)
    parser.add_argument("images", type=Path, nargs="*", metavar="PNG")
    args = parser.parse_args()

    if args.render:
        if args.images:
            parser.error("--render does not accept PNG inputs")
        with tempfile.TemporaryDirectory(prefix="runner-ico-") as tmp:
            pack_ico(parser, args.output, render_pngs(parser, Path(tmp)))
        return

    if len(args.images) != len(SIZES):
        parser.error(f"expected {len(SIZES)} PNGs ({', '.join(map(str, SIZES))}) or --render")
    pack_ico(parser, args.output, args.images)


if __name__ == "__main__":
    main()
