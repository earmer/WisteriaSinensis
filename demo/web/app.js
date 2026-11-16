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
  lastError: null,
};

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
  return text
    .split(/\r\n|[\r\n\u2029]/)
    .filter((paragraph) => paragraph.trim() !== '');
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
    | ($('whitespace').value === 'preserved' ? 1 << 8 : 0);
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

  const started = performance.now();
  const results = [];
  let lines = 0;
  let glyphs = 0;
  let justified = 0;
  let hanging = 0;
  for (const [index, paragraph] of paragraphs.entries()) {
    const result = state.engine.layout(
      paragraph,
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
      gateChip.textContent = 'engine rejected the input';
      gateChip.className = 'chip bad';
      stats.innerHTML = '';
      drawPlaceholder(settings.width, 'Layout rejected — see message above.');
      return;
    }
    results.push(result);
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

  gateChip.textContent = results.length === 0 ? 'nothing to draw' : 'output gate passed';
  gateChip.className = `chip ${results.length === 0 ? 'muted' : 'good'}`;
  stats.innerHTML = results.length === 0
    ? ''
    : [
        `<span><strong>${paragraphs.length}</strong> paragraph${paragraphs.length === 1 ? '' : 's'}</span>`,
        `<span><strong>${lines}</strong> lines</span>`,
        `<span><strong>${justified}</strong> justified</span>`,
        `<span><strong>${hanging}</strong> hanging marks</span>`,
        `<span><strong>${glyphs.toLocaleString()}</strong> glyphs</span>`,
        `<span><strong>${elapsed.toFixed(1)}</strong> ms</span>`,
      ].join('');
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
    return;
  }
  hint.hidden = true;

  let y = pad;
  for (const paragraph of results) {
    ctx.save();
    ctx.translate(pad, y);
    drawLineGuides(paragraph, settings);
    drawGlyphs(paragraph);
    drawCaretStops(paragraph, settings);
    ctx.restore();
    y += paragraph.blockSize + paragraphGap;
  }
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

textarea.addEventListener('input', () => {
  clearTimeout(textTimer);
  textTimer = setTimeout(relayout, 140);
});

const phase2Inputs = ['direction', 'justify', 'hang-start', 'hang-end', 'autospace', 'whitespace']
  .map((id) => $(id));

for (const input of [widthInput, sizeInput, lineHeightInput, quotesInput, faceInput, policyInput, ...phase2Inputs]) {
  input.addEventListener('input', () => {
    if (input === widthInput) {
      $('width-value').textContent = `${widthInput.value} px`;
    }
    if (input === sizeInput) {
      $('size-value').textContent = `${sizeInput.value} px`;
    }
    schedule();
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
