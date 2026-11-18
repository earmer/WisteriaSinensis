// Pure selection rules of the page (`web/selection.js`), against a fake engine.
// Usage: node tests/selection.mjs
import assert from 'node:assert/strict';
import { LAST_LINE, Motion, motionFor, orderedSelection, step } from '../web/selection.js';

const at = (paraId, byteOffset, affinity = 'downstream') => ({ paraId, byteOffset, affinity });
const rtl = new Set();
const calls = [];
let reply = { kind: 'moved', byteOffset: 5, affinity: 'upstream' };
const hit = { byteOffset: 9, affinity: 'downstream' };
// Caret x is the byte offset, y the paragraph: a one-line-per-paragraph page.
const engine = {
  caretGeometry: (paraId, byteOffset) => ({ primary: { x: byteOffset, y: paraId * 100 } }),
  moveCursor: (...args) => (calls.push(['move', ...args]), reply),
  hitTestOnLine: (...args) => (calls.push(['line', ...args]), hit),
};
const context = { engine, paraCount: 3, isRtl: (id) => rtl.has(id) };
const sel = (anchor, focus, preferredInline = null) => ({ anchor, focus, preferredInline });

// Keys map to motions; Alt or Ctrl make Left/Right word motions; brackets go by position.
assert.equal(motionFor({ key: 'ArrowLeft' }), Motion.Left);
assert.equal(motionFor({ key: 'ArrowRight', ctrlKey: true }), Motion.WordRight);
assert.equal(motionFor({ key: '{', code: 'BracketLeft' }), Motion.Previous);
assert.equal(motionFor({ key: 'a', code: 'KeyA' }), undefined);

// A selection is ordered by paragraph, then offset; an empty one is null.
assert.equal(orderedSelection(sel(at(0, 3), at(0, 3))), null);
assert.deepEqual(orderedSelection(sel(at(1, 0), at(0, 4))), { start: at(0, 4), end: at(1, 0) });

// A move keeps the anchor only when extending; without a caret it starts at the text start.
let next = step(context, sel(at(0, 1), at(0, 2)), Motion.Right, true);
assert.deepEqual(next, sel(at(0, 1), at(0, 5, 'upstream')));
next = step(context, sel(at(0, 1), at(0, 2)), Motion.Next, false);
assert.deepEqual(next.anchor, next.focus);
calls.length = 0;
step(context, sel(null, null), Motion.Right, false);
assert.deepEqual(calls[0].slice(1, 4), [0, 0, 'downstream']);

// Only Up/Down remember the column; other motions drop it.
calls.length = 0;
next = step(context, sel(at(0, 7), at(0, 7)), Motion.Down, false);
assert.equal(next.preferredInline, 7);
assert.equal(calls[0][4], Motion.Down);
assert.equal(calls[0][5], 7);
assert.equal(step(context, sel(at(0, 7), at(0, 7), 7), Motion.Left, false).preferredInline, null);

// An unshifted Left or Right collapses a range to its visual edge first.
const range = sel(at(0, 2), at(0, 8));
calls.length = 0;
assert.deepEqual(step(context, range, Motion.Left, false), sel(at(0, 2), at(0, 2)));
assert.deepEqual(step(context, range, Motion.Right, false), sel(at(0, 8), at(0, 8)));
assert.equal(calls.length, 0, 'collapse does not move the cursor');
rtl.add(0);
assert.deepEqual(step(context, range, Motion.Left, false), sel(at(0, 2), at(0, 2)), 'one line: visual edge');
const lines = sel(at(0, 2), at(1, 8));
rtl.add(1);
assert.deepEqual(step(context, lines, Motion.Left, false).focus, at(1, 8), 'RTL: left is the end');
rtl.clear();
assert.deepEqual(step(context, lines, Motion.Left, false).focus, at(0, 2), 'LTR: left is the start');
// Shift never collapses.
assert.equal(step(context, range, Motion.Left, true).focus.byteOffset, 5);

// Leaving a paragraph lands on the neighbour's first or last line.
reply = { kind: 'exited', edge: 'end', inlinePosition: 12 };
calls.length = 0;
assert.deepEqual(step(context, sel(at(0, 3), at(0, 3)), Motion.Down, false).focus, at(1, 9));
assert.deepEqual(calls[1], ['line', 1, 0, 12]);
reply = { kind: 'exited', edge: 'start', inlinePosition: null };
calls.length = 0;
step(context, sel(at(1, 0), at(1, 0)), Motion.Left, false);
assert.deepEqual(calls[1], ['line', 0, LAST_LINE, 1e9], 'LTR neighbour entered from its right end');
rtl.add(0);
calls.length = 0;
step(context, sel(at(1, 0), at(1, 0)), Motion.Left, false);
assert.deepEqual(calls[1], ['line', 0, LAST_LINE, -1e9], 'RTL neighbour: logical end is on the left');
rtl.clear();
reply = { kind: 'exited', edge: 'end', inlinePosition: null };
calls.length = 0;
step(context, sel(at(0, 0), at(0, 0)), Motion.Right, false);
assert.deepEqual(calls[1], ['line', 1, 0, -1e9], 'forwards into LTR: its logical start');

// At the first or last paragraph the selection stays.
const last = sel(at(2, 4), at(2, 4));
assert.deepEqual(step(context, last, Motion.Next, false), last);
reply = { kind: 'exited', edge: 'start', inlinePosition: null };
const first = sel(at(0, 4), at(0, 4));
assert.deepEqual(step(context, first, Motion.Previous, false), first);
console.log('selection ok');
