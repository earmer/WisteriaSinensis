import { Engine } from './wasm.js';

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
};

const state = {
  engine: null,
  fonts: new Map(), // font id -> { upem, family }
  missing: [],
  glyphs: new Map(), // "font:gid" -> Path2D | null
  layout: null, // last successful results
  paraLayouts: [], // [{ id, y, blockSize, utf16Start, utf16End, rawText }]
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
const ctx = canvas.getContext('2d');
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

function applySample(key) {
  const preset = PRESETS[key];
  textarea.value = SAMPLES[key] ?? '';
  if (preset) {
    localeInput.value = preset.locale;
    quotesInput.value = preset.quotes;
    faceInput.value = preset.face;
  }
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
  const re = /\r\n|[\r\n\u2029]/g;
  const out = [];
  let start = 0;
  let match;
  while ((match = re.exec(text)) !== null) {
    const slice = text.slice(start, match.index);
    if (slice.trim() !== '') {
      out.push({ rawText: slice, utf16Start: start, utf16End: match.index });
    }
    start = match.index + match[0].length;
  }
  const tail = text.slice(start);
  if (tail.trim() !== '') {
    out.push({ rawText: tail, utf16Start: start, utf16End: text.length });
  }
  return out;
}

/** The Phase 2 options packed as the demo's C ABI expects (`Phase2`). */
function phase2Bits() {
  const direction = { auto: 0, ltr: 1, rtl: 2 }[$('direction').value] ?? 0;
  const start = { none: 0, first: 1, all: 2 }[$('hang-start').value] ?? 0;
  const end = { none: 0, allow: 1, force: 2 }[$('hang-end').value] ?? 0;
  return direction
    | ($('justify').checked ? 1 << 2 : 0)
    | (start << 3)
    | (end << 5)
    | ($('autospace').value === 'css' ? 1 << 7 : 0)
    | ($('whitespace').value === 'preserved' ? 1 << 8 : 0)
    | ($('compress-last').checked ? 1 << 9 : 0);
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
    selectionHeight: Number(selectionHeightInput.value),
    showBoxes: showBoxes.checked,
    showBaselines: showBaselines.checked,
    showCarets: showCarets.checked,
  };
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
  const paragraphs = splitParagraphs(textarea.value);
  state.lastError = null;
  hideError();

  state.engine.clearParagraphs();
  state.engine.setOptions({
    lineBreak: settings.lineBreak,
    wordBreak: settings.wordBreak,
    letterSpacing: settings.letterSpacing,
    wordSpacing: settings.wordSpacing,
    firstLineIndent: settings.firstLineIndent,
    hangingIndent: settings.hangingIndent,
  });

  const started = performance.now();
  const results = [];
  const paraLayouts = [];
  const pad = 32;
  const paragraphGap = 14;
  let y = pad;
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
      state.layout = null;
      state.paraLayouts = [];
      state.selection.anchor = null;
      state.selection.focus = null;
      gateChip.textContent = 'engine rejected the input';
      gateChip.className = 'chip bad';
      stats.replaceChildren();
      drawPlaceholder(settings.width, 'Layout rejected — see message above.');
      updateCursorStatus();
      return;
    }
    results.push(result);
    paraLayouts.push({
      id: result.id,
      y,
      blockSize: result.blockSize,
      utf16Start: para.utf16Start,
      utf16End: para.utf16End,
      rawText: para.rawText,
    });
    y += result.blockSize + paragraphGap;
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
  state.layout = { results, settings };
  state.paraLayouts = paraLayouts;

  if (state.selection.focus && state.selection.focus.paraId >= results.length) {
    state.selection.anchor = null;
    state.selection.focus = null;
  }

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
  draw(results, settings);
}

function draw(results, settings) {
  const pad = 32;
  const paragraphGap = 14;
  const block = results.reduce((sum, result) => sum + result.blockSize, 0);
  const gaps = Math.max(0, results.length - 1) * paragraphGap;
  const cssWidth = settings.width + pad * 2;
  const cssHeight = Math.max(block + gaps + pad * 2, 200);
  const dpr = Math.min(window.devicePixelRatio || 1, 2.5);

  canvas.width = Math.round(cssWidth * dpr);
  canvas.height = Math.round(cssHeight * dpr);
  canvas.style.width = `${cssWidth}px`;
  canvas.style.height = `${cssHeight}px`;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, cssWidth, cssHeight);

  ctx.fillStyle = '#fffdf7';
  ctx.fillRect(0, 0, cssWidth, cssHeight);

  if (results.length === 0) {
    hint.textContent = 'Paste text to lay it out.';
    hint.hidden = false;
    updateCursorStatus();
    return;
  }
  hint.hidden = true;

  const selBounds = orderedSelection();
  const imeRange = resolveImeParagraphRange();

  let y = pad;
  for (const paragraph of results) {
    ctx.save();
    ctx.translate(pad, y);
    drawLineGuides(paragraph, settings);
    if (selBounds) {
      drawSelection(paragraph.id, selBounds, settings.selectionHeight);
    }
    drawGlyphs(paragraph);
    drawCaretStops(paragraph, settings);
    if (imeRange && imeRange.paraId === paragraph.id) {
      drawImeOverlay(paragraph.id, imeRange.startByte, imeRange.endByte);
    }
    if (state.selection.focus && state.selection.focus.paraId === paragraph.id) {
      drawActiveCaret(state.selection.focus);
    }
    ctx.restore();
    y += paragraph.blockSize + paragraphGap;
  }
  updateCursorStatus(selBounds, imeRange);
}

function drawGlyphs(paragraph) {
  const path = new Path2D();
  for (const line of paragraph.lines) {
    for (const run of line.runs) {
      const info = state.fonts.get(run.font);
      if (!info) {
        continue;
      }
      // Outline paths are in font units; the run's size over the face's
      // units-per-em is the only scale applied (spec HST-5: geometry is
      // never adjusted at draw time).
      const scale = run.size / info.upem;
      for (const [glyphId, x, originY] of run.glyphs) {
        const outline = glyphPath(run.font, glyphId);
        if (outline) {
          path.addPath(outline, new DOMMatrix([scale, 0, 0, scale, x, originY]));
        }
      }
    }
  }
  ctx.fillStyle = '#1b1a17';
  ctx.fill(path);
}

function glyphPath(fontId, glyphId) {
  const key = `${fontId}:${glyphId}`;
  if (state.glyphs.has(key)) {
    return state.glyphs.get(key);
  }
  const data = state.engine.outline(fontId, glyphId);
  const path = data ? new Path2D(data) : null;
  state.glyphs.set(key, path);
  return path;
}

function drawLineGuides(paragraph, settings) {
  if (!settings.showBoxes && !settings.showBaselines) {
    return;
  }
  for (const line of paragraph.lines) {
    for (const [start, end] of line.slots) {
      if (settings.showBoxes) {
        ctx.fillStyle = 'rgba(79, 112, 189, 0.06)';
        ctx.fillRect(start, line.top, end - start, line.height);
        ctx.strokeStyle = 'rgba(79, 112, 189, 0.35)';
        ctx.lineWidth = 1;
        ctx.strokeRect(start + 0.5, line.top + 0.5, end - start - 1, line.height - 1);
      }
      if (settings.showBaselines) {
        ctx.save();
        ctx.setLineDash([3, 3]);
        ctx.strokeStyle = 'rgba(79, 112, 189, 0.5)';
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(start, Math.round(line.baseline) + 0.5);
        ctx.lineTo(end, Math.round(line.baseline) + 0.5);
        ctx.stroke();
        ctx.restore();
      }
    }
  }
}

function drawCaretStops(paragraph, settings) {
  if (!settings.showCarets) {
    return;
  }
  ctx.strokeStyle = 'rgba(27, 122, 90, 0.55)';
  ctx.lineWidth = 1;
  for (const line of paragraph.lines) {
    for (const x of line.carets) {
      ctx.beginPath();
      ctx.moveTo(x + 0.5, line.baseline - 4);
      ctx.lineTo(x + 0.5, line.baseline + 4);
      ctx.stroke();
    }
  }
}

function orderedSelection() {
  const { anchor, focus } = state.selection;
  if (!anchor || !focus) {
    return null;
  }
  if (anchor.paraId === focus.paraId && anchor.byteOffset === focus.byteOffset) {
    return null;
  }
  const forward =
    anchor.paraId < focus.paraId ||
    (anchor.paraId === focus.paraId && anchor.byteOffset <= focus.byteOffset);
  return forward ? { start: anchor, end: focus } : { start: focus, end: anchor };
}

function drawSelection(paraId, selBounds, heightStyle) {
  if (paraId < selBounds.start.paraId || paraId > selBounds.end.paraId) {
    return;
  }
  const lo = paraId === selBounds.start.paraId ? selBounds.start.byteOffset : 0;
  const hi = paraId === selBounds.end.paraId ? selBounds.end.byteOffset : 0x7fffffff;
  if (lo >= hi) {
    return;
  }
  const rects = state.engine.selectionGeometry(paraId, lo, hi, heightStyle);
  if (!rects || rects.length === 0) {
    return;
  }
  ctx.fillStyle = 'rgba(79, 112, 189, 0.24)';
  for (const r of rects) {
    ctx.fillRect(r.x, r.y, r.w, r.h);
  }
}

function drawActiveCaret(pos) {
  const caret = state.engine.caretGeometry(pos.paraId, pos.byteOffset, pos.affinity);
  if (!caret) {
    return;
  }
  const focused = document.activeElement === canvas;
  ctx.save();
  const px = Math.round(caret.primary.x) + 0.5;
  ctx.strokeStyle = focused ? '#2563eb' : '#1b1a17';
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.moveTo(px, caret.primary.y);
  ctx.lineTo(px, caret.primary.y + caret.primary.h);
  ctx.stroke();

  if (caret.secondary) {
    const sx = Math.round(caret.secondary.x) + 0.5;
    const top = caret.secondary.y + caret.secondary.h * 0.45;
    const bottom = caret.secondary.y + caret.secondary.h;
    ctx.strokeStyle = '#d97706';
    ctx.lineWidth = 1.5;
    ctx.setLineDash([3, 2]);
    ctx.beginPath();
    ctx.moveTo(sx, top);
    ctx.lineTo(sx, bottom);
    ctx.stroke();
  }
  ctx.restore();
}

function resolveImeParagraphRange() {
  if (!state.ime.active || state.ime.text.length === 0) {
    return null;
  }
  const startUtf16 = state.ime.startUtf16;
  const endUtf16 = startUtf16 + state.ime.text.length;
  const para = state.paraLayouts.find(
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
  const startByte = utf8Encoder.encode(para.rawText.slice(0, relStart)).length;
  const endByte = utf8Encoder.encode(para.rawText.slice(0, relEnd)).length;
  return { paraId: para.id, startByte, endByte };
}

function drawImeOverlay(paraId, startByte, endByte) {
  const rects = state.engine.rangeRects(paraId, startByte, endByte);
  if (!rects || rects.length === 0) {
    return;
  }
  ctx.save();
  ctx.fillStyle = 'rgba(217, 119, 6, 0.12)';
  ctx.strokeStyle = '#d97706';
  ctx.lineWidth = 2;
  ctx.setLineDash([4, 2]);
  for (const r of rects) {
    ctx.fillRect(r.x, r.y, r.w, r.h);
    const uy = Math.round(r.y + r.h - 1) + 0.5;
    ctx.beginPath();
    ctx.moveTo(r.x, uy);
    ctx.lineTo(r.x + r.w, uy);
    ctx.stroke();
  }
  // Simulated IME candidate window anchor at the bottom-left of the first composing rect.
  const anchor = rects[0];
  const ax = Math.round(anchor.x) + 0.5;
  const ay = Math.round(anchor.y + anchor.h) + 0.5;
  ctx.setLineDash([]);
  ctx.fillStyle = '#d97706';
  ctx.beginPath();
  ctx.moveTo(ax, ay);
  ctx.lineTo(ax - 4, ay + 6);
  ctx.lineTo(ax + 4, ay + 6);
  ctx.closePath();
  ctx.fill();

  ctx.font = '11px ui-sans-serif, system-ui, sans-serif';
  const label = 'IME anchor';
  const tw = ctx.measureText(label).width;
  ctx.fillStyle = '#1b1a17';
  ctx.fillRect(ax - 4, ay + 6, tw + 10, 18);
  ctx.fillStyle = '#fffdf7';
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(label, ax + 1, ay + 15);
  ctx.restore();
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
      : 'Click or drag on canvas · Arrow keys (+ Shift) · IME in Text';
}

function drawPlaceholder(width, message) {
  const pad = 32;
  const dpr = Math.min(window.devicePixelRatio || 1, 2.5);
  const cssWidth = width + pad * 2;
  const cssHeight = 200;
  canvas.width = Math.round(cssWidth * dpr);
  canvas.height = Math.round(cssHeight * dpr);
  canvas.style.width = `${cssWidth}px`;
  canvas.style.height = `${cssHeight}px`;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, cssWidth, cssHeight);
  ctx.fillStyle = '#fffdf7';
  ctx.fillRect(0, 0, cssWidth, cssHeight);
  ctx.fillStyle = '#8a8378';
  ctx.font = '14px ui-sans-serif, system-ui, sans-serif';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(message, cssWidth / 2, cssHeight / 2, cssWidth - pad * 2);
  hint.hidden = true;
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

const phase2Inputs = ['direction', 'justify', 'compress-last', 'hang-start', 'hang-end', 'autospace', 'whitespace']
  .map((id) => $(id));

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

for (const input of layoutInputs) {
  input.addEventListener('input', () => {
    if (input === widthInput) {
      $('width-value').textContent = `${widthInput.value} px`;
    } else if (input === sizeInput) {
      $('size-value').textContent = `${sizeInput.value} px`;
    } else if (input === letterSpacingInput) {
      $('letter-spacing-value').textContent = `${letterSpacingInput.value} px`;
    } else if (input === wordSpacingInput) {
      $('word-spacing-value').textContent = `${wordSpacingInput.value} px`;
    } else if (input === firstIndentInput) {
      $('first-indent-value').textContent = `${firstIndentInput.value} px`;
    } else if (input === hangingIndentInput) {
      $('hanging-indent-value').textContent = `${hangingIndentInput.value} px`;
    }
    schedule();
  });
}

selectionHeightInput.addEventListener('change', () => {
  if (state.layout) {
    const { results, settings } = state.layout;
    settings.selectionHeight = Number(selectionHeightInput.value);
    draw(results, settings);
  }
});

function canvasPointToParagraph(clientX, clientY) {
  if (state.paraLayouts.length === 0) {
    return null;
  }
  const rect = canvas.getBoundingClientRect();
  const cx = clientX - rect.left;
  const cy = clientY - rect.top;
  const pad = 32;
  const paragraphGap = 14;
  let target = state.paraLayouts[state.paraLayouts.length - 1];
  for (const para of state.paraLayouts) {
    if (cy < para.y + para.blockSize + paragraphGap * 0.5) {
      target = para;
      break;
    }
  }
  return { paraId: target.id, x: cx - pad, y: cy - target.y };
}

canvas.addEventListener('pointerdown', (event) => {
  if (event.button !== 0 || !state.layout || state.paraLayouts.length === 0) {
    return;
  }
  canvas.focus({ preventScroll: true });
  canvas.setPointerCapture(event.pointerId);
  const pt = canvasPointToParagraph(event.clientX, event.clientY);
  if (!pt) {
    return;
  }
  const hit = state.engine.hitTest(pt.paraId, pt.x, pt.y);
  if (!hit) {
    return;
  }
  const pos = { paraId: pt.paraId, byteOffset: hit.byteOffset, affinity: hit.affinity };
  if (event.shiftKey && state.selection.anchor) {
    state.selection.focus = pos;
  } else {
    state.selection.anchor = pos;
    state.selection.focus = pos;
  }
  state.selection.preferredInline = null;
  state.selection.dragging = true;
  draw(state.layout.results, state.layout.settings);
});

canvas.addEventListener('pointermove', (event) => {
  if (!state.selection.dragging || !state.layout || state.paraLayouts.length === 0) {
    return;
  }
  const pt = canvasPointToParagraph(event.clientX, event.clientY);
  if (!pt) {
    return;
  }
  const hit = state.engine.hitTest(pt.paraId, pt.x, pt.y);
  if (!hit) {
    return;
  }
  state.selection.focus = { paraId: pt.paraId, byteOffset: hit.byteOffset, affinity: hit.affinity };
  draw(state.layout.results, state.layout.settings);
});

for (const evName of ['pointerup', 'pointercancel']) {
  canvas.addEventListener(evName, () => {
    state.selection.dragging = false;
  });
}

canvas.addEventListener('keydown', (event) => {
  if (!state.layout || state.paraLayouts.length === 0) {
    return;
  }
  const motionMap = {
    ArrowLeft: event.altKey || event.ctrlKey ? 6 : 0,
    ArrowRight: event.altKey || event.ctrlKey ? 7 : 1,
    ArrowUp: 2,
    ArrowDown: 3,
    Home: 4,
    End: 5,
  };
  const motion = motionMap[event.key];
  if (motion === undefined) {
    return;
  }
  event.preventDefault();
  if (!state.selection.focus) {
    const start = { paraId: 0, byteOffset: 0, affinity: 'downstream' };
    state.selection.anchor = start;
    state.selection.focus = start;
  }
  const focus = state.selection.focus;
  if (motion === 2 || motion === 3) {
    if (state.selection.preferredInline === null) {
      const caret = state.engine.caretGeometry(focus.paraId, focus.byteOffset, focus.affinity);
      if (caret) {
        state.selection.preferredInline = caret.primary.x;
      }
    }
  } else {
    state.selection.preferredInline = null;
  }

  const moved = state.engine.moveCursor(
    focus.paraId,
    focus.byteOffset,
    focus.affinity,
    motion,
    state.selection.preferredInline,
  );
  if (!moved) {
    return;
  }
  let nextPos = null;
  if (moved.kind === 'moved') {
    nextPos = { paraId: focus.paraId, byteOffset: moved.byteOffset, affinity: moved.affinity };
  } else if (moved.kind === 'exited') {
    if (moved.edge === 'start' && focus.paraId > 0) {
      const prevId = focus.paraId - 1;
      const inlineX = moved.inlinePosition !== null ? moved.inlinePosition : 1e9;
      const hit = state.engine.hitTestOnLine(prevId, 0xffffffff, inlineX);
      if (hit) {
        nextPos = { paraId: prevId, byteOffset: hit.byteOffset, affinity: hit.affinity };
      }
    } else if (moved.edge === 'end' && focus.paraId + 1 < state.paraLayouts.length) {
      const nextId = focus.paraId + 1;
      const inlineX = moved.inlinePosition !== null ? moved.inlinePosition : -1e9;
      const hit = state.engine.hitTestOnLine(nextId, 0, inlineX);
      if (hit) {
        nextPos = { paraId: nextId, byteOffset: hit.byteOffset, affinity: hit.affinity };
      }
    }
  }
  if (nextPos) {
    state.selection.focus = nextPos;
    if (!event.shiftKey) {
      state.selection.anchor = nextPos;
    }
    draw(state.layout.results, state.layout.settings);
  }
});

for (const evName of ['focus', 'blur']) {
  canvas.addEventListener(evName, () => {
    if (state.layout && state.selection.focus) {
      draw(state.layout.results, state.layout.settings);
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
// `LocaleTag::parse_lenient`, so shorthand such as `jp` or `zh-tc` works too.
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

for (const input of [showBoxes, showBaselines, showCarets]) {
  input.addEventListener('change', () => {
    if (state.layout) {
      const { results, settings } = state.layout;
      settings.showBoxes = showBoxes.checked;
      settings.showBaselines = showBaselines.checked;
      settings.showCarets = showCarets.checked;
      draw(results, settings);
    }
  });
}

const PRESETS = {
  mixed: { locale: '', quotes: 'cjk-aware', face: 'Noto Sans' },
  latin: { locale: 'en', quotes: 'standard', face: 'Noto Sans' },
  japanese: { locale: 'ja-JP', quotes: 'cjk-aware', face: 'Noto Sans' },
  zhHans: { locale: 'zh-CN', quotes: 'cjk-aware', face: 'Noto Sans' },
  regional: { locale: '', quotes: 'cjk-aware', face: 'Noto Sans' },
  indic: { locale: '', quotes: 'standard', face: 'Noto Sans' },
  rtl: { locale: '', quotes: 'standard', face: 'Noto Sans' },
  cjkJustify: { locale: '', quotes: 'cjk-aware', face: 'Noto Sans' },
};

samples.addEventListener('change', () => {
  applySample(samples.value);
  relayout();
});

$('width-value').textContent = `${widthInput.value} px`;
$('size-value').textContent = `${sizeInput.value} px`;

void boot();
