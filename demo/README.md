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
mono; Simplified Chinese, Traditional Chinese and Japanese; Devanagari, Thai, Lao, Khmer and
Myanmar; Hebrew and Arabic, checks them against `fonts.sha256`, and stores the OFL licence
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
Phase 2's controls are there: the base direction (from the text, or forced
LTR or RTL), justification, the hang modes at each line edge, the autospace
profile and the trailing-whitespace mode. The bidi and CJK samples exercise
them, and the statistics count justified lines and hanging marks.
Hyphenation, clamping, vertical writing and the balanced/publication
breaking strategies are absent; the scope panel on the page says so.

## Locales and CJK faces

Every paragraph gets the same family stack: the chosen face, then Noto Sans,
Noto Serif, JetBrains Mono, Noto Sans SC, Noto Sans TC, Noto Sans JP and the
Indic and Southeast Asian faces. The page never reorders it by locale. The
engine profiles each face when it is registered and routes a CJK run to the
CJK face of its locale (spec SHP-12, SHP-25), so `zh-CN` Han comes from
Noto Sans SC, `zh-TW` or `zh-Hant` from Noto Sans TC and `ja` from Noto Sans
JP, and the Latin faces no longer serve the run's shared punctuation.

The engine does not guess Simplified from Traditional Han. Untagged text takes
the Locale field, which is read with `LocaleTag::parse_lenient` (so `jp`,
`cn`, `zh-tc` and the preset buttons all work). For a switch inside a
paragraph, tag the text:

```text
<zh-CN>令你甘心。骨肉分离。</zh-CN> <zh-TW>令你甘心。骨肉分離。</zh-TW> <ja>令。骨肉分離。</ja>
<lang=zh-Hant>…</lang>
```

`src/markup.rs` strips the tags, so the engine sees plain text (spec AX-1),
and passes each tagged range as a `StyleSpan` with `style.locale` set
(FE-25). A tag counts only when it parses as a locale and is closed by its own
closing tag; nested tags give each byte the innermost locale; anything else
stays in the text as typed. A tag cannot span two paragraphs, because the page
lays out each line of the text box as its own paragraph. The Locale policy
selector switches `FrontendOptions::locale_policy` between `ExplicitOnly` and
`ExperimentalHeuristics` (FE-16).
