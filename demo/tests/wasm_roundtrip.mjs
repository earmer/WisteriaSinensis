// Runs the built wasm through the page's own `wasm.js`. `cargo test` is
// native and cannot see allocator or marshaling faults; this can.
// Usage: node tests/wasm_roundtrip.mjs   (after ./build.sh)
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { Engine } from '../web/wasm.js';

const root = new URL('../', import.meta.url);
const wasm = fs.readFileSync(new URL('web/pkg/mutil_demo.wasm', root));
const { instance } = await WebAssembly.instantiate(wasm, {});
const x = instance.exports;
const engine = new Engine(instance);
const live = () => x.demo_live_buffers();

// 1. Raw alloc/free at sizes that straddle every allocator size class.
for (const n of [0, 1, 7, 8, 16, 63, 64, 97, 98, 99, 100, 127, 128, 255, 256, 4095, 4096, 65535, 1 << 20]) {
  const p = x.demo_alloc(n);
  assert.notEqual(p, 0, `alloc(${n}) returned null`);
  x.demo_free(p);
}
assert.equal(live(), 0, 'raw round trip leaked');

// 2. Hostile frees must be ignored, not trap.
const p = x.demo_alloc(200);
x.demo_free(p); x.demo_free(p); x.demo_free(0); x.demo_free(1); x.demo_free(0xfffffff0);
assert.equal(live(), 0);

// 3. A real font and paragraphs well past the 98-byte threshold.
const face = fs.readFileSync(new URL('../mutil-engine/tests/assets/JetBrainsMono-Subset.ttf', root));
const font = engine.addFont('JetBrains Mono', new Uint8Array(face));
assert.ok(font.id !== null, 'font registered');
assert.equal(live(), 0, 'addFont leaked');
for (const len of [1, 50, 98, 99, 500, 5000]) {
  engine.clearParagraphs();
  const result = engine.layout('ab '.repeat(len), 400, 16, 0, null, false, null, false, 0);
  assert.equal(result.ok, true, `layout ${len}: ${JSON.stringify(result).slice(0, 200)}`);
  assert.ok(engine.hitTest(0, 1, 1) !== undefined);
  assert.equal(live(), 0, `layout(${len}) leaked`);
}

// 4. Non-ASCII input crosses with UTF-8 byte lengths, not UTF-16 lengths.
engine.clearParagraphs();
assert.equal(engine.layout('日本語 abc '.repeat(30), 300, 16, 0, 'ja', false, null, false, 0).ok, true);
assert.equal(live(), 0);

// 5. Steady state: repeated calls neither leak buffers nor grow memory.
const before = x.memory.buffer.byteLength;
for (let i = 0; i < 2000; i++) { engine.clearParagraphs(); engine.layout('xy '.repeat(100), 400, 16, 0, null, false, null, false, 0); }
assert.equal(live(), 0);
assert.ok(x.memory.buffer.byteLength - before < 4 << 20, 'memory grew across 2000 identical layouts');
console.log('wasm round trip ok');
