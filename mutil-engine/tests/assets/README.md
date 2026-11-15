# Test assets

## `JetBrainsMono-Subset.ttf`

A subset of JetBrains Mono Regular, version 2.304, used by the pipeline tests
(`tests/pipeline/`) as the one real face the HarfRust backend shapes with.

- **Source:** `JetBrainsMono-Regular.ttf` from the Debian package
  `fonts-jetbrains-mono` (upstream <https://github.com/JetBrains/JetBrainsMono>),
  SHA-256 `bb03b32b13f66bb799975d478d3725fbed9101728d46ffda995e87d7847fda2d`.
- **Licence:** SIL Open Font License 1.1, in `OFL.txt` beside this file. The
  font declares no Reserved Font Name, so the subset keeps its name.
- **Subset:** made with fontTools 4.68.0:

  ```bash
  pyftsubset JetBrainsMono-Regular.ttf \
    --unicodes="U+0020-007E,U+00A0,U+00E9,U+0301,U+2026" \
    --layout-features='ccmp,locl,mark,mkmk,calt' \
    --no-hinting --name-IDs='*' --drop-tables+=DSIG \
    --output-file=JetBrainsMono-Subset.ttf
  ```

  SHA-256 of the result:
  `ba601e58ee9ae0980fb3c157f5ecbc8fee5da728059cff8b3f324c257ce30c19`.

What the tests rely on: 1000 units per em; every spacing glyph, the space
included, advances 600 units; U+0301 is a zero-advance mark that `mark`
positions over its base; OS/2 `fsSelection` bit 7 (`USE_TYPO_METRICS`) is set,
with typo ascender 1020, descender -300 and line gap 0; `post`
`underlinePosition` is -155 and OS/2 `yStrikeoutPosition` 320. There is no
`BASE`, `vhea` or GDEF ligature caret table, so those metrics take the
backend's documented fallbacks.

Fonts for other scripts and the Unicode conformance files arrive with Step B.
