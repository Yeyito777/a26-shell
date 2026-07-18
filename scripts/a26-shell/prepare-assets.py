#!/usr/bin/env python3
"""Generate Xorg-native BGRX app-icon bytes from the committed PNG source."""

from pathlib import Path

from PIL import Image


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SOURCE = PROJECT_ROOT / "src/a26-shell/assets/system-app.png"
OUTPUT = PROJECT_ROOT / "src/a26-shell/assets/system-app.bgrx"
SIZE = (220, 220)

image = Image.open(SOURCE).convert("RGB").resize(SIZE, Image.Resampling.LANCZOS)
rgb = image.tobytes()
bgrx = bytearray()
for index in range(0, len(rgb), 3):
    red, green, blue = rgb[index : index + 3]
    bgrx.extend((blue, green, red, 0))

OUTPUT.write_bytes(bgrx)
if OUTPUT.stat().st_size != SIZE[0] * SIZE[1] * 4:
    raise SystemExit("unexpected BGRX icon size")
