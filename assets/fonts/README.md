# Native PWDS fonts

These are native TTF derivatives of the webfonts already shipped in this repository,
not downloaded system fonts. `scripts/prepare-native-fonts.py` converts their format
and renames the derived families to **PWDS Console Display** and **PWDS Console
Control**. The display source reserves the name NovaSquare; that name is not used
as the derivative's family/full/PostScript name.

| Native asset | Existing source | License |
| --- | --- | --- |
| `pwds-display.ttf` | `ui/fonts/nova-square-latin-400-normal.woff2` | `OFL-NovaSquare.txt` |
| `pwds-control.ttf` | `ui/fonts/rajdhani-latin-700-normal.woff2` | `OFL-Rajdhani.txt` |

Original copyright, glyph outlines, character mappings and advance/sidebearing
metrics are retained. The complete applicable OFL text is also embedded in each
TTF's license name record, so native embedding carries the copyright and license.
The fonts remain under SIL OFL 1.1; they are not relicensed as application code.
Named egui families retain the bundled proportional fallback for non-Latin titles.

Rebuild from the repository root with Python and `fonttools[woff]` installed:

```sh
python scripts/prepare-native-fonts.py
```

No conversion tool, Python interpreter, online font request, or OS font lookup is
used at application runtime. No source webfont was changed.
