// Thin ABI over the wasm build of `mutil-engine`.
//
// The module is a plain `cdylib` (no wasm-bindgen): strings go in through
// `demo_alloc`, results come back as a packed (pointer, length) word, and the
// page copies them out before freeing. Nothing here shapes or positions text;
// it only moves bytes.

const decoder = new TextDecoder();
const encoder = new TextEncoder();

export class Engine {
  #exports;

  constructor(instance) {
    this.#exports = instance.exports;
  }

  static async load(url) {
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`${url}: HTTP ${response.status}`);
    }
    // `instantiateStreaming` is picky about MIME types from simple static
    // servers, so buffer the module first.
    const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {});
    return new Engine(instance);
  }

  /**
   * The buffer contract: every address `demo_alloc` returns, and every result
   * word, is released exactly once with `demo_free(address)`. Rust owns the
   * lengths, so a wrong or repeated free is ignored, never a trap. Inputs are
   * only ever allocated here, and only ever freed in the `finally` below.
   */
  #call(name, ...args) {
    const { memory, demo_alloc: alloc, demo_free: free } = this.#exports;
    const owned = [];
    try {
      const flat = args.flatMap((arg) => {
        if (typeof arg !== 'string' && !(arg instanceof Uint8Array)) {
          return [arg];
        }
        const bytes = typeof arg === 'string' ? encoder.encode(arg) : arg;
        const ptr = bytes.length === 0 ? 0 : alloc(bytes.length);
        if (bytes.length !== 0) {
          if (ptr === 0) {
            throw new Error(`wasm out of memory (${bytes.length} bytes)`);
          }
          owned.push(ptr);
          new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
        }
        return [ptr, bytes.length];
      });
      return this.#exports[name](...flat);
    } finally {
      owned.forEach((ptr) => free(ptr));
    }
  }

  /** Copies a result word `(address << 32 | length)` out and releases it. */
  #take(packed) {
    const word = BigInt.asUintN(64, BigInt(packed));
    const length = Number(word & 0xffffffffn);
    const ptr = Number(word >> 32n);
    const text = decoder.decode(new Uint8Array(this.#exports.memory.buffer, ptr, length));
    this.#exports.demo_free(ptr);
    return text;
  }

  #text(name, ...args) {
    return this.#take(this.#call(name, ...args));
  }

  #json(name, ...args) {
    return JSON.parse(this.#text(name, ...args));
  }

  addFont(family, bytes) {
    return this.#json('demo_add_font', family, bytes);
  }

  layout(text, width, fontSize, lineHeight, locale, cjkQuotes, primary, heuristics, phase2) {
    return this.#json(
      'demo_layout',
      text, width, fontSize, lineHeight,
      locale ?? '', cjkQuotes ? 1 : 0, primary ?? '', heuristics ? 1 : 0, phase2,
    );
  }

  outline(fontId, glyphId) {
    return this.#text('demo_outline', fontId, glyphId);
  }

  conformance() {
    return this.#json('demo_conformance');
  }

  clearParagraphs() {
    this.#exports.demo_clear_paragraphs();
  }

  setOptions({
    lineBreak = 0,
    wordBreak = 0,
    letterSpacing = 0,
    wordSpacing = 0,
    firstLineIndent = 0,
    hangingIndent = 0,
  }) {
    this.#exports.demo_set_options(
      lineBreak,
      wordBreak,
      letterSpacing,
      wordSpacing,
      firstLineIndent,
      hangingIndent,
    );
  }

  hitTest(paraId, x, y) {
    return this.#json('demo_hit_test', paraId, x, y);
  }

  hitTestOnLine(paraId, line, inlinePosition) {
    return this.#json('demo_hit_test_on_line', paraId, line, inlinePosition);
  }

  caretGeometry(paraId, byteOffset, affinity) {
    const aff = affinity === 'upstream' ? 1 : 0;
    return this.#json('demo_caret_geometry', paraId, byteOffset, aff);
  }

  moveCursor(paraId, byteOffset, affinity, motion, preferredInline = null) {
    const aff = affinity === 'upstream' ? 1 : 0;
    const hasPreferred = preferredInline !== null && Number.isFinite(preferredInline) ? 1 : 0;
    return this.#json('demo_move_cursor', paraId, byteOffset, aff, motion, hasPreferred, hasPreferred ? preferredInline : 0);
  }

  selectionGeometry(paraId, start, end, heightStyle = 0) {
    return this.#json('demo_selection_geometry', paraId, start, end, heightStyle);
  }

  /** The laid-out text offset of source byte `sourceOffset`, past locale tags. */
  textOffset(paraId, sourceOffset) {
    const offset = this.#exports.demo_text_offset(paraId, sourceOffset) >>> 0;
    return offset === 0xffffffff ? null : offset;
  }

  rangeRects(paraId, start, end) {
    return this.#json('demo_range_rects', paraId, start, end);
  }
}
