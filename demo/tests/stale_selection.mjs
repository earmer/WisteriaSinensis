// A selection or caret made against one text must not survive a relayout for
// different text; a relayout of the same text keeps it. Boots the real
// web/app.js with a stub DOM and the real wasm, and observes only the DOM.
// Usage: node tests/stale_selection.mjs [demo-dir]   (after ./build.sh)
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const demo = path.resolve(process.argv[2] ?? fileURLToPath(new URL('../', import.meta.url)));
const repo = fileURLToPath(new URL('../../', import.meta.url));
const face = fs.readFileSync(path.join(repo, 'mutil-engine/tests/assets/JetBrainsMono-Subset.ttf'));

const defaults = {
  width: '400', 'font-size': '18', 'line-height': 'normal', quotes: 'standard', 'locale-policy': 'explicit',
  'line-break': '0', 'word-break': '0', 'letter-spacing': '0', 'word-spacing': '0', 'first-indent': '0',
  'hanging-indent': '0', 'selection-height': '0', direction: 'auto', 'hang-start': 'none', 'hang-end': 'none',
  autospace: 'jlreq', whitespace: 'collapsible', 'cjk-punct-trim': 'auto', face: 'JetBrains Mono', samples: 'latin', locale: '',
};
const els = new Map();
function el(id) {
  if (els.has(id)) return els.get(id);
  const listeners = {};
  const ctx = new Proxy({}, {
    get: (t, k) => (k in t ? t[k] : k === 'measureText' ? () => ({ width: 5 }) : () => {}),
    set: (t, k, v) => ((t[k] = v), true),
  });
  const e = {
    id, value: defaults[id] ?? '', checked: false, textContent: '', innerHTML: '', hidden: false, className: '',
    style: {}, classList: { toggle() {} }, dataset: {}, listeners,
    addEventListener(n, f) { (listeners[n] ??= []).push(f); },
    fire(n, ev = {}) { for (const f of listeners[n] ?? []) f({ preventDefault() {}, ...ev }); },
    getContext: () => ctx, getBoundingClientRect: () => ({ left: 0, top: 0 }),
    setPointerCapture() {}, releasePointerCapture() {}, focus() {}, setAttribute() {}, querySelectorAll: () => [],
    replaceChildren() {}, selectionStart: 0, width: 0, height: 0,
  };
  els.set(id, e);
  return e;
}
globalThis.document = { getElementById: el, activeElement: null, createElement: () => ({ append() {}, set textContent(v) {} }) };
globalThis.window = { devicePixelRatio: 1 };
globalThis.Path2D = class { addPath() {} };
globalThis.DOMMatrix = class {};
globalThis.requestAnimationFrame = (f) => setTimeout(f, 0);
globalThis.fetch = async (url) => {
  const p = fileURLToPath(new URL(url));
  const buf = p.endsWith('.wasm') ? fs.readFileSync(p) : face;
  return { ok: true, arrayBuffer: async () => buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength) };
};

const text = el('text');
const canvas = el('canvas');
text.value = 'abc def ghi\n\njkl mno';
await import(path.join(demo, 'web/app.js'));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
await sleep(500);
assert.equal(el('error').hidden, true, `app failed to boot: ${el('error').innerHTML}`);

const status = () => el('cursor-status').textContent;
const click = () => {
  canvas.fire('pointerdown', { button: 0, clientX: 40, clientY: 40, pointerId: 1 });
  canvas.fire('pointerup', { pointerId: 1 });
};
const key = (k, shiftKey = false) => canvas.fire('keydown', { key: k, shiftKey });
const relayout = async () => { el('width').fire('input'); await sleep(400); };

const idle = status();
assert.doesNotMatch(idle, /Caret/, 'no caret before any interaction');

// Same text: a relayout (width, then font size) keeps caret and selection.
click();
const caret = status();
assert.match(caret, /Caret/, 'click places a caret');
key('ArrowRight', true); key('ArrowRight', true);
const selected = status();
assert.notEqual(selected, caret, 'shift+arrow extends the selection');
await relayout();
assert.equal(status(), selected, 'same-text width relayout keeps the selection');
el('font-size').value = '20';
el('font-size').fire('input'); await sleep(400);
assert.equal(status(), selected, 'same-text font-size relayout keeps the selection');

// Text changes and the relayout runs: no selection, status reset.
text.value = 'xyz\n\nuvw qrs tuv';
text.fire('input');
await sleep(500);
assert.equal(status(), idle, 'selection dropped after the text changed');
assert.doesNotMatch(status(), /Caret/);

// Click made inside the debounce window (against the old layout) must not survive.
click();
assert.match(status(), /Caret/, 'click in the debounce window selects in the old layout');
text.value = 'one two three\n\nfour five';
text.fire('input');
click();
key('ArrowRight', true);
await sleep(500);
assert.equal(status(), idle, 'selection made during the debounce is dropped by the relayout');

// A fresh selection on the new text works and survives a same-text relayout.
click();
assert.match(status(), /Caret/);
const fresh = status();
await relayout();
assert.equal(status(), fresh, 'fresh selection survives a same-text relayout');

console.log('stale_selection ok');
process.exit(0);
