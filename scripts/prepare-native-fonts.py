#!/usr/bin/env python3
"""Rebuild native font assets from the repository's OFL webfonts.

One-time maintainer tool: requires fonttools[woff]. The application has no Python
or system-font dependency. Glyph outlines/metrics are unchanged; derived font
names are distinct to respect the source's Reserved Font Name.
"""
from pathlib import Path
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[1]
SOURCES = (
    ("nova-square-latin-400-normal", "pwds-display", "PWDS Console Display", "NovaSquare"),
    ("rajdhani-latin-700-normal", "pwds-control", "PWDS Console Control", "Rajdhani"),
)


def main():
    destination = ROOT / "assets" / "fonts"
    destination.mkdir(parents=True, exist_ok=True)
    for source, filename, family, license_name in SOURCES:
        font = TTFont(
            ROOT / "ui" / "fonts" / f"{source}.woff2",
            recalcBBoxes=False,
            recalcTimestamp=False,
        )
        font.flavor = None
        for name in font["name"].names:
            if name.nameID in (1, 3, 4, 6, 16, 18, 21):
                value = family.replace(" ", "") if name.nameID == 6 else family
                name.string = value.encode(name.getEncoding())
        license_text = (ROOT / "ui" / "fonts" / f"OFL-{license_name}.txt").read_text(encoding="utf-8")
        font["name"].setName(license_text, 13, 3, 1, 0x409)
        font["name"].setName("https://openfontlicense.org/", 14, 3, 1, 0x409)
        font.save(destination / f"{filename}.ttf")


if __name__ == "__main__":
    main()
