# Visual harness

A small page that runs `mutil-engine` in the browser and draws what it
returns. Paste text, change the container width, and look at the same glyph
ids and origins the engine validated — the canvas never re-shapes or
re-positions text, and it never asks the browser to lay text out.

## Run

```sh
cd demo
./fetch-fonts.sh      # full Noto faces, about 32 MB, cached in fonts/
./build.sh            # wasm32-unknown-unknown -> web/pkg/mutil_demo.wasm
./serve.sh            # http://127.0.0.1:8000/
```

Requirements: Rust 1.88 or later with the `wasm32-unknown-unknown` target
(`rustup target add wasm32-unknown-unknown`), Python 3 for the static server,
and a browser with `Path2D.addPath` (current Chrome, Firefox and Safari).

`fetch-fonts.sh` downloads the variable faces used for Latin sans, serif and
mono; Simplified and Japanese Chinese; Devanagari, Thai, Lao, Khmer and
Myanmar, checks them against `fonts.sha256`, and stores the OFL licence
beside them. The files are not committed.

## How it works

- `src/lib.rs` is a `cdylib` with a small C ABI. Through it the page registers
  font bytes, calls `compile_paragraph`, `layout_paragraph` and
  `ValidatedLayout::new`, and gets back JSON: per line, the slots, caret
  stops and every run's `font`, `font_size` and glyph ids with paragraph-local
  drawing origins.
- Outlines come from the same registered bytes through `skrifa`, in font
  units; the page applies only `size / units_per_em` and the engine's origin
  (spec HST-3 to HST-6). There is no hinting and no geometry adjustment.
- `web/app.js` splits the textarea into paragraphs (the engine takes one
  paragraph at a time), lays out each, and draws the results on a canvas.
  Line boxes, baselines and caret stops can be overlaid. The face, BCP 47
  locale and quotation policy (FE-13 or FE-14) are engine inputs, not page
  decoration: they are passed to `TextStyle` and `FrontendOptions` unchanged.

The harness deliberately offers no control the engine does not implement.
Justification, hanging punctuation, hyphenation, clamping, bidi, vertical
writing and the balanced/publication breaking strategies are absent; the
scope panel on the page says so. Pasted Arabic or Hebrew is rejected by the
frontend, and the page shows that error verbatim rather than drawing text in
the wrong order.

## Known engine gap exposed here

SHP-12 requires font fallback to use script and locale as the key and says
`Hani` skips Latin-only faces. `HarfRustShaper::face_for` in
`mutil-engine/src/shaping/harfrust.rs` currently decides by cmap coverage
alone. With a stack whose Latin face precedes the CJK faces, a Chinese
paragraph still forms one `Hani` run, but U+201C, U+2026, U+2014 and U+00B7
are served by the Latin face: for example `“` advances 6.5 units at size 18,
`…` 14.2 and `·` 4.8, where the same glyphs in Noto Sans SC advance 18.
The page shows the real result and names the gap; it does not reorder or
filter the stack to hide it. The fix belongs in the shaper.

Adding Noto Sans SC is a font-asset fix, not a shaper workaround: without a
Simplified Chinese face, common GB 2312 characters such as 总/结/验/发/现
fall through to glyph 0 and draw as boxes.

## Layout of the files

```
demo/
  Cargo.toml          harness crate, depends on ../mutil-engine
  src/lib.rs          FFI, JSON output, outline extraction
  web/index.html      page
  web/app.js          controls, paragraph loop, canvas drawing
  web/wasm.js         wasm ABI wrapper
  web/style.css       minimal styling
  web/pkg/            built wasm (ignored)
  fonts/              fetched faces (ignored)
  build.sh            cargo build + copy the module
  fetch-fonts.sh      download the Noto faces
  serve.sh            static server
```

## Licence

The harness is part of WisteriaSinensis, GPL-3.0-or-later. The fetched fonts
are under the SIL Open Font License 1.1; `fetch-fonts.sh` stores the licence
text in `fonts/OFL.txt`.
