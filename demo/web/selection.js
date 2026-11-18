// Caret and selection rules of the demo page, with no DOM in sight.
//
// Everything here is a function of a selection value and an `engine` (the
// `Engine` of wasm.js, or any object with the same query methods), so Node can
// run it. The page only turns keys into `Motion`s and applies the result.

/** The motion codes `demo_move_cursor` decodes, in the order Rust lists them. */
export const Motion = Object.freeze({
  Left: 0,
  Right: 1,
  Up: 2,
  Down: 3,
  LineStart: 4,
  LineEnd: 5,
  WordLeft: 6,
  WordRight: 7,
  Previous: 8,
  Next: 9,
});

/** `hitTestOnLine` line index meaning "the paragraph's last line". */
export const LAST_LINE = 0xffffffff;

/** An inline position beyond either end of any line. */
const FAR_INLINE = 1e9;

const START = Object.freeze({ paraId: 0, byteOffset: 0, affinity: 'downstream' });

/** The motion a key asks for, or `undefined`. `key` is an event's `key`/`code`. */
export function motionFor({ key, code, altKey = false, ctrlKey = false }) {
  const word = altKey || ctrlKey;
  const byKey = {
    ArrowLeft: word ? Motion.WordLeft : Motion.Left,
    ArrowRight: word ? Motion.WordRight : Motion.Right,
    ArrowUp: Motion.Up,
    ArrowDown: Motion.Down,
    Home: Motion.LineStart,
    End: Motion.LineEnd,
    // Logical order, found by key position so Shift still extends.
    BracketLeft: Motion.Previous,
    BracketRight: Motion.Next,
  };
  return byKey[key] ?? byKey[code];
}

/** A caret position from a hit-test or move result in paragraph `paraId`. */
export function position(paraId, { byteOffset, affinity }) {
  return { paraId, byteOffset, affinity };
}

/** `{ start, end }` in document order, or `null` for an empty selection. */
export function orderedSelection({ anchor, focus }) {
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

/**
 * The end of a selection that an unshifted Left or Right collapses to (its
 * visual edge on `side`, 'left' or 'right'), or `null` when there is nothing
 * to collapse.
 */
function collapsedEdge({ engine, isRtl }, selection, side) {
  const bounds = orderedSelection(selection);
  if (!bounds) {
    return null;
  }
  const { start, end } = bounds;
  const caret = (pos) => engine.caretGeometry(pos.paraId, pos.byteOffset, pos.affinity)?.primary;
  const a = caret(start);
  const b = caret(end);
  // Each end says where `side` points from it: in an LTR paragraph Left is
  // backwards, so the start is the left edge; in an RTL one it is the end.
  const startOnSide = (side === 'left') !== isRtl(start.paraId);
  const endOnSide = (side === 'left') === isRtl(end.paraId);
  const sameLine = a && b && start.paraId === end.paraId && Math.abs(a.y - b.y) < 1;
  if (!sameLine && startOnSide !== endOnSide) {
    // Exactly one end is on `side`, as across lines of one direction.
    return startOnSide ? start : end;
  }
  if (a && b) {
    // On one line, or when mixed directions put both ends or neither on
    // `side`, the edge on `side` is the visual one.
    return (a.x <= b.x) === (side === 'left') ? start : end;
  }
  return start;
}

/**
 * Where a caret that left paragraph `paraId` through `moved.edge` lands in the
 * neighbouring paragraph, or `null` at the first or last one. With no column
 * to keep it lands on the neighbour's logical start (entering forwards) or end,
 * whose side depends on that paragraph's direction.
 */
function crossParagraph({ engine, paraCount, isRtl }, paraId, moved) {
  const forward = moved.edge === 'end';
  const neighbour = paraId + (forward ? 1 : -1);
  if (neighbour < 0 || neighbour >= paraCount) {
    return null;
  }
  const farInline = forward === isRtl(neighbour) ? FAR_INLINE : -FAR_INLINE;
  const hit = engine.hitTestOnLine(
    neighbour,
    forward ? 0 : LAST_LINE,
    moved.inlinePosition ?? farInline,
  );
  return hit && position(neighbour, hit);
}

/**
 * The selection after `motion`: `{ anchor, focus, preferredInline }`, never
 * mutating `selection`. `extend` (Shift) keeps the anchor. `context` is
 * `{ engine, paraCount, isRtl(paraId) }`. A motion that cannot go anywhere
 * (the first or last paragraph's edge) leaves the selection as it was, apart
 * from a caret appearing at the start of the text when there was none.
 */
export function step(context, selection, motion, extend) {
  if (!extend && (motion === Motion.Left || motion === Motion.Right)) {
    const edge = collapsedEdge(context, selection, motion === Motion.Left ? 'left' : 'right');
    if (edge) {
      return { anchor: edge, focus: edge, preferredInline: null };
    }
  }
  const { anchor, focus } = selection.focus ? selection : { anchor: START, focus: START };
  // Only vertical motions keep the column they started from.
  const vertical = motion === Motion.Up || motion === Motion.Down;
  let preferredInline = vertical ? selection.preferredInline : null;
  if (vertical && preferredInline === null) {
    const caret = context.engine.caretGeometry(focus.paraId, focus.byteOffset, focus.affinity);
    preferredInline = caret?.primary.x ?? null;
  }
  const stay = { anchor, focus, preferredInline };
  const moved = context.engine.moveCursor(
    focus.paraId,
    focus.byteOffset,
    focus.affinity,
    motion,
    preferredInline,
  );
  if (!moved) {
    return stay;
  }
  const next =
    moved.kind === 'moved' ? position(focus.paraId, moved) : crossParagraph(context, focus.paraId, moved);
  return next ? { anchor: extend ? anchor : next, focus: next, preferredInline } : stay;
}
