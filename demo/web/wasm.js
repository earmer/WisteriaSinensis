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

  #put(text) {
    const bytes = encoder.encode(text);
    const ptr = this.#exports.demo_alloc(bytes.length);
    if (bytes.length > 0) {
      new Uint8Array(this.#exports.memory.buffer, ptr, bytes.length).set(bytes);
    }
    return [ptr, bytes.length];
  }

  #putBytes(bytes) {
    const ptr = this.#exports.demo_alloc(bytes.length);
    new Uint8Array(this.#exports.memory.buffer, ptr, bytes.length).set(bytes);
    return [ptr, bytes.length];
  }

  #take(packed) {
    const word = BigInt.asUintN(64, BigInt(packed));
    const length = Number(word & 0xffffffffn);
    const ptr = Number(word >> 32n);
    const view = new Uint8Array(this.#exports.memory.buffer, ptr, length);
    const bytes = view.slice();
    this.#exports.demo_free(ptr, length);
    return decoder.decode(bytes);
  }

  addFont(family, bytes) {
    const [familyPtr, familyLen] = this.#put(family);
    const [dataPtr, dataLen] = this.#putBytes(bytes);
    try {
      return JSON.parse(this.#take(this.#exports.demo_add_font(familyPtr, familyLen, dataPtr, dataLen)));
    } finally {
      this.#exports.demo_free(familyPtr, familyLen);
      this.#exports.demo_free(dataPtr, dataLen);
    }
  }

  layout(text, width, fontSize, lineHeight, locale, cjkQuotes, primary, heuristics, phase2) {
    const [ptr, len] = this.#put(text);
    const [localePtr, localeLen] = this.#put(locale ?? '');
    const [primaryPtr, primaryLen] = this.#put(primary ?? '');
    try {
      const packed = this.#exports.demo_layout(
        ptr, len, width, fontSize, lineHeight,
        localePtr, localeLen, cjkQuotes ? 1 : 0, primaryPtr, primaryLen,
        heuristics ? 1 : 0, phase2,
      );
      return JSON.parse(this.#take(packed));
    } finally {
      this.#exports.demo_free(ptr, len);
      this.#exports.demo_free(localePtr, localeLen);
      this.#exports.demo_free(primaryPtr, primaryLen);
    }
  }

  outline(fontId, glyphId) {
    return this.#take(this.#exports.demo_outline(fontId, glyphId));
  }

  conformance() {
    return JSON.parse(this.#take(this.#exports.demo_conformance()));
  }
}
