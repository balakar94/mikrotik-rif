#!/usr/bin/env python3
"""Generate the application icon from the master vector SVG.

Produces a 1024 px PNG plus the macOS (.icns) and Windows (.ico) variants that
cargo-packager expects. Run it from the repository root:

    python3 tools/make_icon.py

Requires assets/icon/icon.svg as the source vector.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

from PIL import Image

SIZE = 1024
OUT = Path("assets/icon")
SVG_SOURCE = OUT / "icon.svg"


def render_svg_to_png(svg_path: Path, png_path: Path, size: int = SIZE) -> None:
    """Render an SVG file to a PNG using AppKit via swift on macOS."""
    swift_code = f"""import AppKit
let url = URL(fileURLWithPath: "{svg_path.resolve()}")
guard let img = NSImage(contentsOf: url) else {{
    fputs("Failed to load SVG\\n", stderr)
    exit(1)
}}
let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: {size},
    pixelsHigh: {size},
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
img.draw(in: NSRect(x: 0, y: 0, width: {size}, height: {size}))
NSGraphicsContext.restoreGraphicsState()
guard let data = rep.representation(using: .png, properties: [:]) else {{
    fputs("Failed to encode PNG\\n", stderr)
    exit(1)
}}
do {{
    try data.write(to: URL(fileURLWithPath: "{png_path.resolve()}"))
}} catch {{
    fputs("Failed to write PNG: \\(error)\\n", stderr)
    exit(1)
}}
"""
    res = subprocess.run(["swift", "-e", swift_code], capture_output=True, text=True)
    if res.returncode != 0:
        raise RuntimeError(f"Failed to render SVG: {res.stderr}\n{res.stdout}")


def write_icns(icon: Image.Image, dest_dir: Path = OUT) -> None:
    iconset = dest_dir / "icon.iconset"
    if iconset.exists():
        shutil.rmtree(iconset)
    iconset.mkdir(parents=True)

    for size in (16, 32, 128, 256, 512):
        icon.resize((size, size), Image.Resampling.LANCZOS).save(iconset / f"icon_{size}x{size}.png")
        icon.resize((size * 2, size * 2), Image.Resampling.LANCZOS).save(
            iconset / f"icon_{size}x{size}@2x.png"
        )

    if shutil.which("iconutil"):
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(dest_dir / "icon.icns")],
            check=True,
        )
        shutil.rmtree(iconset)
    else:
        print("iconutil not found: skipping icon.icns", file=sys.stderr)


def build_icons_for_dir(target_dir: Path, svg_name: str = "icon.svg") -> None:
    svg_file = target_dir / svg_name
    png_out = target_dir / "icon.png"
    if not svg_file.exists():
        return
    print(f"Rendering {svg_file} -> {png_out} ({SIZE}x{SIZE})...")
    render_svg_to_png(svg_file, png_out, SIZE)
    icon = Image.open(png_out)
    icon.save(
        target_dir / "icon.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    write_icns(icon, dest_dir=target_dir)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    build_icons_for_dir(OUT)
    
    # Also ensure posible folder has full .ico and .icns
    posible = OUT / "posible"
    if posible.exists() and (posible / "icon.svg").exists():
        build_icons_for_dir(posible)

    print("assets/icon contents:", ", ".join(sorted(p.name for p in OUT.iterdir() if not p.name.startswith('.'))))
    if posible.exists():
        print("assets/icon/posible contents:", ", ".join(sorted(p.name for p in posible.iterdir() if not p.name.startswith('.'))))


if __name__ == "__main__":
    main()
