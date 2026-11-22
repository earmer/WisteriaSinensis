import { Engine } from './wasm.js';
import { PAGE, createRenderer } from './render.js';
import { motionFor, orderedSelection, position, step } from './selection.js';

const FONTS = [
  ['Noto Sans', 'NotoSans[wdth,wght].ttf'],
  ['Noto Serif', 'NotoSerif[wdth,wght].ttf'],
  ['JetBrains Mono', 'JetBrainsMono[wght].ttf'],
  ['Noto Sans SC', 'NotoSansSC[wght].ttf'],
  ['Noto Sans TC', 'NotoSansTC[wght].ttf'],
  ['Noto Sans JP', 'NotoSansJP[wght].ttf'],
  ['Noto Sans Devanagari', 'NotoSansDevanagari[wdth,wght].ttf'],
  ['Noto Sans Thai', 'NotoSansThai[wdth,wght].ttf'],
  ['Noto Sans Lao', 'NotoSansLao[wdth,wght].ttf'],
  ['Noto Sans Khmer', 'NotoSansKhmer[wdth,wght].ttf'],
  ['Noto Sans Myanmar', 'NotoSansMyanmar[wdth,wght].ttf'],
  ['Noto Sans Hebrew', 'NotoSansHebrew[wdth,wght].ttf'],
  ['Noto Naskh Arabic', 'NotoNaskhArabic[wght].ttf'],
];

const SAMPLES = {
  mixed: [
    'Typesetting is the arrangement of glyphs, not of characters. The engine shapes each run with HarfRust, then breaks lines greedily at the opportunities UAX #14 allows.',
    '日本語の組版では、行の折り返し位置と約物の扱いが読みやすさを決めます。この見本は、エンジンが返したグリフの位置をそのまま描いています。',
    'देवनागरी में मात्राएँ और संयुक्ताक्षर एक ही साँचे में ढलते हैं। ไทยและลาวมีสระบนล่างที่ต้องวางให้ถูกตำแหน่ง',
  ].join('\n'),
  latin:
    'ﬁne office affluence · café naïve Ångström · Straße · “quoted text” — an em dash… and a soft­hyphen that is not a break opportunity yet.',
  japanese:
    '組版とは、文字ではなくグリフを並べることです。このページは、エンジンが返した検証済みのグリフ位置をキャンバスに描くだけで、ブラウザの文字組版には一切任せていません。行頭・行末の禁則処理、字間の調整、行の最適化はまだ実装されていません。',
  indic:
    'देवनागरी में मात्राएँ और संयुक्ताक्षर एक ही साँचे में ढलते हैं।\nการเรียงพิมพ์ภาษาไทยต้องวางสระบนและสระล่างให้ถูกตำแหน่ง\nການຈັດລຽງຕົວອັກສອນລາວ\nការរៀបចំអក្សរខ្មែរ\nမြန်မာစာ စာလုံးစီရာတွင်',
  zhHans:
    '《西江月》词牌：简体中文的标点“省略……”和破折号——、间隔号·都在这里。总结经验、发现问题、链接导航和时间校验，确保常用汉字不出现方框。',
  regional:
    '<zh-CN>令你甘心。骨肉分离。</zh-CN> <zh-TW>令你甘心。骨肉分離。</zh-TW> <ja>令。骨肉分離。</ja>\n' +
    '<lang=zh-Hant>同一個字型堆疊，三種地區字形：「令」「骨」與置中的句號。標記只說明語言，字型由引擎依文字與語言挑選；未標記的漢字不作繁簡猜測。</lang>',
  rtl:
    'هذا النص مكتوب من اليمين إلى اليسار، وفيه كلمات English وأرقام 2024 وأقواس (مرآة) «وعلامات» تتبع الخوارزمية الثنائية الاتجاه.\n' +
    'עברית נכתבת מימין לשמאל: המנוע ממיין שורה אחר שורה, כולל קטע ABC 123 משמאל לימין בתוך פסקה עברית.\n' +
    'Mixed English paragraph with עברית בתוך שורה and العربية too, wrapped and reordered per line.',
  cjkJustify:
    '日本語とEnglishが混ざる文章では、和欧間に自動でアキが入ります。「括弧」や句読点、句点。行末の約物はぶら下げ組みもできます。両端揃えでは、まず単語間、次に和欧間、それから字間の順に調整します。\n' +
    '中文和English混排时，引擎会在中西文之间插入间距。标点挤压：“引号”（括号）。两端对齐按优先级逐级分配剩余空间，最后一行不对齐。',
  richText:
    'Rich text with <pill>cloned pills that wrap cleanly across lines</pill>, <mark>sliced highlights</mark>, <u>underlined</u> and <s>struck</s> runs, plus per-glyph paint attribution across <color=#c0392b>fi</color><color=#1d4ed8>ne</color> <color=#0f766e>office</color> ligatures.\n' +
    'A multi-line <ribbon>continuous gradient ribbon whose shader coordinate advances seamlessly across line wraps and mixed עברית Bidi fragments</ribbon> without breaking shaping.',
};

const state = {
  engine: null,
  fonts: new Map(), // font id -> { upem, family }
  missing: [],
  // The last successful layout, or null: `text` is the textarea value it was
  // laid out from, `results` the engine's paragraphs each extended with
  // { y, rawText, utf16Start, utf16End }.
  view: null,
  selection: {
    anchor: null, // { paraId, byteOffset, affinity }
    focus: null, // { paraId, byteOffset, affinity }
    preferredInline: null,
    dragging: false,
  },
  ime: {
    active: false,
    startUtf16: 0,
    text: '',
  },
  lastError: null,
};

const utf8Encoder = new TextEncoder();

const $ = (id) => document.getElementById(id);
const canvas = $('canvas');
const renderer = createRenderer(canvas);
const textarea = $('text');
const widthInput = $('width');
const sizeInput = $('font-size');
const lineHeightInput = $('line-height');
const localeInput = $('locale');
const quotesInput = $('quotes');
const policyInput = $('locale-policy');
const localePresets = $('locale-presets');
const faceInput = $('face');
const lineBreakInput = $('line-break');
const wordBreakInput = $('word-break');
const letterSpacingInput = $('letter-spacing');
const wordSpacingInput = $('word-spacing');
const firstIndentInput = $('first-indent');
const hangingIndentInput = $('hanging-indent');
const selectionHeightInput = $('selection-height');
const cursorStatus = $('cursor-status');
const showBoxes = $('show-boxes');
const showBaselines = $('show-baselines');
const showCarets = $('show-carets');
const errorBox = $('error');
const hint = $('hint');
const stats = $('stats');
const gateChip = $('gate-chip');
const fontChip = $('font-chip');
const engineStatus = $('engine-status');
const fontNote = $('font-note');
const samples = $('samples');

let textTimer = 0;
let localeTimer = 0;
let frame = 0;

/** Samples whose quotation marks stay Latin; the others use CJK-aware quotes. */
const STANDARD_QUOTES = ['latin', 'indic', 'rtl'];
const SAMPLE_LOCALES = { latin: 'en', japanese: 'ja-JP', zhHans: 'zh-CN' };

function applySample(key) {
  textarea.value = SAMPLES[key] ?? '';
  localeInput.value = SAMPLE_LOCALES[key] ?? '';
  quotesInput.value = STANDARD_QUOTES.includes(key) ? 'standard' : 'cjk-aware';
  faceInput.value = 'Noto Sans';
  syncLocalePresets();
}

function syncLocalePresets() {
  const current = localeInput.value.trim().toLowerCase() || 'und';
  for (const button of localePresets.querySelectorAll('button[data-locale]')) {
    const active = button.dataset.locale.toLowerCase() === current;
    button.setAttribute('aria-pressed', active ? 'true' : 'false');
  }
}

async function boot() {
  applySample(samples.value);
  hint.hidden = false;
  hint.textContent = 'Loading engine and fonts…';
  renderEngineStatus(null);
  try {
    state.engine = await Engine.load(new URL('./pkg/mutil_demo.wasm', import.meta.url));
  } catch (error) {
    showFatal(
      'The wasm module did not load.',
      `Run <code>./build.sh</code> in the <code>demo/</code> directory, then serve this folder over HTTP. (${error.message})`,
    );
    return;
  }
  renderEngineStatus(state.engine.conformance());
  await loadFonts();
  relayout();
}

async function loadFonts() {
  for (const [index, [family, file]] of FONTS.entries()) {
    fontNote.textContent = `loading face ${index + 1} of ${FONTS.length}…`;
    try {
      const url = new URL(`../fonts/${encodeURIComponent(file)}`, import.meta.url);
      const response = await fetch(url);
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }
      const bytes = new Uint8Array(await response.arrayBuffer());
      const info = state.engine.addFont(family, bytes);
      if (!info || info.id === null) {
        throw new Error('not a readable font');
      }
      state.fonts.set(info.id, { upem: info.upem, family });
    } catch (error) {
      state.missing.push(`${family} (${error.message})`);
    }
  }
  const loaded = state.fonts.size;
  fontChip.textContent = `${loaded} of ${FONTS.length} faces`;
  fontChip.className = `chip ${loaded === FONTS.length ? 'good' : 'warn'}`;
  fontNote.textContent =
    loaded === FONTS.length
      ? 'Font files are registered with the engine before layout; the canvas' +
        ' rasterises their outlines, it does not run the browser shaper.'
      : `Missing: ${state.missing.join(', ')}. Run ./fetch-fonts.sh.`;
  fontNote.classList.toggle('bad', loaded === 0);
}

function renderEngineStatus(report) {
  if (!report) {
    engineStatus.innerHTML = '<li class="muted">waiting for the wasm module…</li>';
    return;
  }
  const components = [
    ['grapheme clusters', report.grapheme],
    ['line break opportunities', report.lineBreak],
    ['word boundaries', report.word],
    ['scripts & script extensions', report.script],
    ['emoji sequences', report.emoji],
    ['bidi resolution', report.bidi],
  ];
  engineStatus.innerHTML = components
    .map(([name, version]) =>
      version
        ? `<li><span class="dot on"></span>${name}<span class="version">${version}</span></li>`
        : `<li><span class="dot off"></span>${name}<span class="version off">not implemented</span></li>`,
    )
    .join('');
  $('unicode-version').textContent = `Unicode data ${report.unicode}`;
}

function splitParagraphs(text) {
  return [...text.matchAll(/[^\r\n\u2029]+/g)]
    .filter((match) => match[0].trim() !== '')
    .map((match) => ({
      rawText: match[0],
      utf16Start: match.index,
      utf16End: match.index + match[0].length,
    }));
}

/** The Phase 2 options packed as the demo's C ABI expects (`Phase2`). */
function phase2Bits() {
  const direction = { auto: 0, ltr: 1, rtl: 2 }[$('direction').value] ?? 0;
  const start = { none: 0, first: 1, all: 2 }[$('hang-start').value] ?? 0;
  const end = { none: 0, allow: 1, force: 2 }[$('hang-end').value] ?? 0;
  const punctTrim = { auto: 0, 'space-first': 1, 'trim-start': 2, 'space-all': 3 }[$('cjk-punct-trim').value] ?? 0;
  const minHScale = Number($('min-h-scale').value) || 0;
  return direction
    | ($('justify').checked ? 1 << 2 : 0)
    | (start << 3)
    | (end << 5)
    | ($('autospace').value === 'css' ? 1 << 7 : 0)
    | ($('whitespace').value === 'preserved' ? 1 << 8 : 0)
    | ($('compress-last').checked ? 1 << 9 : 0)
    | (punctTrim << 10)
    | ($('shape-fit').checked ? 1 << 12 : 0)
    | (minHScale << 13);
}

function currentSettings() {
  return {
    width: Number(widthInput.value),
    fontSize: Number(sizeInput.value),
    lineHeight: lineHeightInput.value === 'normal' ? 0 : Number(lineHeightInput.value),
    locale: localeInput.value.trim(),
    cjkQuotes: quotesInput.value === 'cjk-aware',
    heuristics: policyInput.value === 'heuristics',
    phase2: phase2Bits(),
    primary: faceInput.value,
    lineBreak: Number(lineBreakInput.value),
    wordBreak: Number(wordBreakInput.value),
    letterSpacing: Number(letterSpacingInput.value),
    wordSpacing: Number(wordSpacingInput.value),
    firstLineIndent: Number(firstIndentInput.value),
    hangingIndent: Number(hangingIndentInput.value),
  };
}

/** Drops the canvas selection; `relayout` calls it when the text changed. */
function clearSelection() {
  state.selection.anchor = null;
  state.selection.focus = null;
  state.selection.preferredInline = null;
}

function relayout() {
  if (!state.engine) {
    return;
  }
  if (state.fonts.size === 0) {
    showFatal('No fonts are registered.', 'Run <code>./fetch-fonts.sh</code> in <code>demo/</code>, then reload.');
    return;
  }
  const settings = currentSettings();
  const text = textarea.value;
  const paragraphs = splitParagraphs(text);
  // One rule for every way the text can change (typing, IME, a new sample):
  // a selection holds offsets into the text it was made in, so it survives
  // only a relayout of that same text. A click during the input debounce
  // selects in the old layout; the relayout of the new text drops it.
  if (state.view?.text !== text) {
    clearSelection();
  }
  state.lastError = null;
  hideError();

  // Geometry changes with every layout: a remembered column is stale.
  state.selection.preferredInline = null;
  state.engine.clearParagraphs();
  state.engine.setOptions(settings);

  const started = performance.now();
  const results = [];
  let y = PAGE.pad;
  let lines = 0;
  let glyphs = 0;
  let justified = 0;
  let hanging = 0;
  for (const [index, para] of paragraphs.entries()) {
    const result = state.engine.layout(
      para.rawText,
      settings.width,
      settings.fontSize,
      settings.lineHeight,
      settings.locale || null,
      settings.cjkQuotes,
      settings.primary || null,
      settings.heuristics,
      settings.phase2,
    );
    if (!result.ok) {
      showEngineError(result, index, paragraphs.length);
      state.view = null;
      clearSelection();
      gateChip.textContent = 'engine rejected the input';
      gateChip.className = 'chip bad';
      stats.replaceChildren();
      drawPlaceholder(settings.width, 'Layout rejected — see message above.');
      updateCursorStatus();
      return;
    }
    results.push(Object.assign(result, para, { y }));
    y += result.blockSize + PAGE.paragraphGap;
    lines += result.lines.length;
    for (const line of result.lines) {
      justified += line.justified ? 1 : 0;
      hanging += (line.hang[0] > 0 ? 1 : 0) + (line.hang[1] > 0 ? 1 : 0);
      for (const run of line.runs) {
        glyphs += run.glyphs.length;
      }
    }
  }
  const elapsed = performance.now() - started;
  state.view = { text, width: settings.width, results };

  gateChip.textContent = results.length === 0 ? 'nothing to draw' : 'output gate passed';
  gateChip.className = `chip ${results.length === 0 ? 'muted' : 'good'}`;
  const figures = [
    [paragraphs.length, paragraphs.length === 1 ? 'paragraph' : 'paragraphs'],
    [lines, 'lines'],
    [justified, 'justified'],
    [hanging, 'hanging marks'],
    [glyphs.toLocaleString(), 'glyphs'],
    [elapsed.toFixed(1), 'ms'],
  ];
  stats.replaceChildren(...(results.length === 0 ? [] : figures.map(([value, label]) => {
    const item = document.createElement('span');
    const strong = document.createElement('strong');
    strong.textContent = String(value);
    item.append(strong, ` ${label}`);
    return item;
  })));
  draw();
}

function draw() {
  const view = state.view;
  if (!view) {
    return;
  }
  if (view.results.length === 0) {
    renderer.drawPage(view);
    hint.textContent = 'Paste text to lay it out.';
    hint.hidden = false;
    updateCursorStatus();
    return;
  }
  hint.hidden = true;
  const selBounds = orderedSelection(state.selection);
  const imeRange = resolveImeParagraphRange();
  renderer.drawPage(view, {
    engine: state.engine,
    fonts: state.fonts,
    selBounds,
    focus: state.selection.focus,
    imeRange,
    guides: { boxes: showBoxes.checked, baselines: showBaselines.checked, carets: showCarets.checked },
    selectionHeight: Number(selectionHeightInput.value),
  });
  updateCursorStatus(selBounds, imeRange);
}

function drawPlaceholder(width, message) {
  renderer.drawPlaceholder(width, message);
  hint.hidden = true;
}

function resolveImeParagraphRange() {
  if (!state.ime.active || state.ime.text.length === 0) {
    return null;
  }
  const startUtf16 = state.ime.startUtf16;
  const endUtf16 = startUtf16 + state.ime.text.length;
  const para = state.view.results.find(
    (p) => startUtf16 >= p.utf16Start && startUtf16 <= p.utf16End,
  );
  if (!para) {
    return null;
  }
  const relStart = Math.max(0, startUtf16 - para.utf16Start);
  const relEnd = Math.min(para.rawText.length, endUtf16 - para.utf16Start);
  if (relEnd <= relStart) {
    return null;
  }
  // The layout holds the text without locale tags: map the source bytes past
  // them.
  const toText = (rel) =>
    state.engine.textOffset(para.id, utf8Encoder.encode(para.rawText.slice(0, rel)).length);
  const startByte = toText(relStart);
  const endByte = toText(relEnd);
  if (startByte === null || endByte === null || endByte <= startByte) {
    return null;
  }
  return { paraId: para.id, startByte, endByte };
}

function updateCursorStatus(selBounds, imeRange) {
  const parts = [];
  if (state.selection.focus) {
    const f = state.selection.focus;
    parts.push(`Caret: p${f.paraId} · byte ${f.byteOffset} (${f.affinity})`);
  }
  if (selBounds) {
    const heightLabel = ['Tight', 'Max', 'Strut'][Number(selectionHeightInput.value)] ?? 'Tight';
    parts.push(
      `Sel: p${selBounds.start.paraId}:${selBounds.start.byteOffset}–p${selBounds.end.paraId}:${selBounds.end.byteOffset} (${heightLabel})`,
    );
  }
  if (imeRange) {
    parts.push(`IME: p${imeRange.paraId} bytes ${imeRange.startByte}..${imeRange.endByte}`);
  }
  cursorStatus.textContent =
    parts.length > 0
      ? parts.join('  |  ')
      : 'Click or drag on canvas · Arrow keys, [ ] logical (+ Shift) · IME in Text';
}

function showEngineError(result, index, total) {
  const where = total > 1 ? ` paragraph ${index + 1}` : ' this text';
  const stage = { compile: 'frontend', layout: 'layout', validate: 'output gate', input: 'input' }[result.stage] ?? result.stage;
  state.lastError = result;
  errorBox.innerHTML = `
    <strong>The ${stage} rejected${where}.</strong>
    <span>${escapeHtml(result.error)}</span>
    <small>This is the engine's real answer, not a drawing fallback. The
    <a href="#scope">scope panel</a> lists what Phase 1 supports.</small>`;
  errorBox.hidden = false;
}

function showFatal(title, detail) {
  errorBox.innerHTML = `<strong>${escapeHtml(title)}</strong><span>${detail}</span>`;
  errorBox.hidden = false;
  hint.hidden = true;
  gateChip.textContent = 'not running';
  gateChip.className = 'chip bad';
}

function hideError() {
  errorBox.hidden = true;
  errorBox.innerHTML = '';
}

function escapeHtml(text) {
  return text.replace(/[&<>"']/g, (c) => ({
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#39;',
  })[c]);
}

function schedule() {
  if (frame) {
    return;
  }
  frame = requestAnimationFrame(() => {
    frame = 0;
    relayout();
  });
}

textarea.addEventListener('compositionstart', (event) => {
  state.ime.active = true;
  state.ime.startUtf16 = textarea.selectionStart;
  state.ime.text = event.data ?? '';
});

textarea.addEventListener('compositionupdate', (event) => {
  state.ime.active = true;
  state.ime.text = event.data ?? '';
});

textarea.addEventListener('compositionend', () => {
  state.ime.active = false;
  state.ime.text = '';
  clearTimeout(textTimer);
  relayout();
});

textarea.addEventListener('input', (event) => {
  clearTimeout(textTimer);
  if (event.isComposing || state.ime.active) {
    relayout();
  } else {
    textTimer = setTimeout(relayout, 140);
  }
});

const phase2Inputs = ['direction', 'justify', 'compress-last', 'hang-start', 'hang-end', 'cjk-punct-trim', 'autospace', 'whitespace', 'min-h-scale', 'shape-fit']
  .map((id) => $(id));

const layoutInputs = [
  widthInput,
  sizeInput,
  lineHeightInput,
  quotesInput,
  faceInput,
  policyInput,
  lineBreakInput,
  wordBreakInput,
  ...phase2Inputs,
  letterSpacingInput,
  wordSpacingInput,
  firstIndentInput,
  hangingIndentInput,
];

function showValue(input) {
  const output = $(`${input.id}-value`);
  if (output) {
    output.textContent = `${input.value} px`;
  }
}

for (const input of layoutInputs) {
  input.addEventListener('input', () => {
    showValue(input);
    schedule();
  });
  showValue(input);
}

/** True when there is a layout with at least one paragraph to point at. */
function hasParagraphs() {
  return (state.view?.results.length ?? 0) > 0;
}

function canvasPointToParagraph(clientX, clientY) {
  const { results } = state.view;
  const rect = canvas.getBoundingClientRect();
  const cx = clientX - rect.left;
  const cy = clientY - rect.top;
  const target =
    results.find((para) => cy < para.y + para.blockSize + PAGE.paragraphGap * 0.5) ?? results.at(-1);
  return { paraId: target.id, x: cx - PAGE.pad, y: cy - target.y };
}

/** The caret position under a pointer event, or `null` when the engine finds none. */
function hitPosition(event) {
  const pt = canvasPointToParagraph(event.clientX, event.clientY);
  const hit = state.engine.hitTest(pt.paraId, pt.x, pt.y);
  return hit && position(pt.paraId, hit);
}

canvas.addEventListener('pointerdown', (event) => {
  if (event.button !== 0 || !hasParagraphs()) {
    return;
  }
  canvas.focus({ preventScroll: true });
  canvas.setPointerCapture(event.pointerId);
  const pos = hitPosition(event);
  if (!pos) {
    return;
  }
  state.selection.anchor = event.shiftKey && state.selection.anchor ? state.selection.anchor : pos;
  state.selection.focus = pos;
  state.selection.preferredInline = null;
  state.selection.dragging = true;
  draw();
});

canvas.addEventListener('pointermove', (event) => {
  if (!state.selection.dragging || !hasParagraphs()) {
    return;
  }
  const pos = hitPosition(event);
  if (pos) {
    state.selection.focus = pos;
    draw();
  }
});

for (const evName of ['pointerup', 'pointercancel']) {
  canvas.addEventListener(evName, () => {
    state.selection.dragging = false;
  });
}

canvas.addEventListener('keydown', (event) => {
  const motion = motionFor(event);
  if (!hasParagraphs() || motion === undefined) {
    return;
  }
  event.preventDefault();
  const context = {
    engine: state.engine,
    paraCount: state.view.results.length,
    isRtl: (paraId) => state.view.results[paraId]?.direction === 'rtl',
  };
  Object.assign(state.selection, step(context, state.selection, motion, event.shiftKey));
  draw();
});

for (const evName of ['focus', 'blur']) {
  canvas.addEventListener(evName, () => {
    if (state.selection.focus) {
      draw();
    }
  });
}

localeInput.addEventListener('input', () => {
  syncLocalePresets();
  clearTimeout(localeTimer);
  localeTimer = setTimeout(relayout, 200);
});
localeInput.addEventListener('change', () => {
  clearTimeout(localeTimer);
  syncLocalePresets();
  relayout();
});

// Each preset only fills the locale field; the engine reads it with
// `LocaleTag::try_from_str_lenient`, so shorthand such as `jp` or `zh-tc` works too.
localePresets.addEventListener('click', (event) => {
  const button = event.target.closest('button[data-locale]');
  if (!button) {
    return;
  }
  clearTimeout(localeTimer);
  localeInput.value = button.dataset.locale === 'und' ? '' : button.dataset.locale;
  syncLocalePresets();
  relayout();
});

// The overlays and the selection height are read from these where they are
// drawn, so a change only needs a redraw.
for (const input of [showBoxes, showBaselines, showCarets, selectionHeightInput]) {
  input.addEventListener('change', draw);
}

samples.addEventListener('change', () => {
  applySample(samples.value);
  relayout();
});

void boot();
