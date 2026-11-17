//! A browser harness for `mutil-engine`.
//!
//! Nothing here lays out text itself. The engine compiles, breaks and
//! validates the paragraph; this crate only
//!
//! * registers font faces and keeps their bytes for outline extraction,
//! * turns a validated layout into numbers a canvas can draw
//!   (`ValidatedLayout::glyph_runs`, spec HST-3/HST-4), and
//! * pulls glyph outlines out of the same bytes with `skrifa` for the
//!   renderer.
//!
//! The boundary is a small C ABI so the page needs no JS package manager:
//! strings and byte arrays are copied through exported `alloc`/`free`, and
//! every call returns a length-prefixed, packed pointer to UTF-8 JSON (or an
//! SVG path). The demo is compiled to `wasm32-unknown-unknown`; the crate is
//! also a plain `rlib` so `cargo test` can exercise it natively.

mod engine;
mod fonts;
mod json;
mod markup;
mod options;

use std::sync::Mutex;

use mutil_engine::conformance_report;

use engine::{Demo, LayoutRequest};
use json::{conformance_json, error_json, font_json};
use options::{ExtraOptions, Phase2};

/// A global engine. The page registers faces once and lays out repeatedly.
static DEMO: Mutex<Option<Demo>> = Mutex::new(None);

fn with_demo<T>(f: impl FnOnce(&mut Demo) -> T) -> Option<T> {
    let mut guard = DEMO.lock().ok()?;
    Some(f(guard.get_or_insert_with(Demo::new)))
}

/// Runs a paragraph query and returns its JSON, or `fallback` when there is
/// no engine or no such paragraph.
fn query(fallback: &str, f: impl FnOnce(&mut Demo) -> Option<String>) -> u64 {
    leak(
        with_demo(f)
            .flatten()
            .unwrap_or_else(|| fallback.to_string()),
    )
}

/// Packs a pointer and a length into one word: high 32 bits are the pointer,
/// low 32 the length (both fit in wasm32).
fn packed(ptr: *mut u8, len: usize) -> u64 {
    ((ptr as u64) << 32) | (len as u64 & 0xffff_ffff)
}

/// Hands `text` to the page as a packed pointer it frees with [`demo_free`].
fn leak(text: String) -> u64 {
    let boxed = text.into_bytes().into_boxed_slice();
    let len = boxed.len();
    let ptr = Box::into_raw(boxed).cast::<u8>();
    packed(ptr, len)
}

/// Allocates `len` bytes for the page to write into; free with [`demo_free`].
#[unsafe(no_mangle)]
pub extern "C" fn demo_alloc(len: u32) -> *mut u8 {
    let len = len as usize;
    if len == 0 {
        return std::ptr::NonNull::<u8>::dangling().as_ptr();
    }
    let layout = std::alloc::Layout::array::<u8>(len).expect("byte layout");
    // SAFETY: the layout is non-zero-sized and valid.
    unsafe { std::alloc::alloc(layout) }
}

/// Releases a buffer from [`demo_alloc`] or a result from any call below.
///
/// # Safety
/// `ptr` must come from this module and `len` must be its exact length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_free(ptr: *mut u8, len: u32) {
    let len = len as usize;
    if len == 0 {
        return;
    }
    // SAFETY: the caller contract mirrors the allocator that produced `ptr`.
    unsafe {
        if let Ok(layout) = std::alloc::Layout::from_size_align(len, 1) {
            std::alloc::dealloc(ptr, layout);
        }
    }
}

fn read_str<'a>(ptr: *const u8, len: u32) -> Option<&'a str> {
    if len == 0 {
        return Some("");
    }
    // SAFETY: the page wrote `len` bytes at `ptr` before the call.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    std::str::from_utf8(bytes).ok()
}

/// Copies `bytes` into the engine's face table and registers it under
/// `family`. Returns `{"id":n,"upem":n}` or `{"id":null}` when the bytes hold
/// no font.
///
/// # Safety
/// `family_ptr` and `data_ptr` must each point to the stated number of bytes
/// that stay readable for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_add_font(
    family_ptr: *const u8,
    family_len: u32,
    data_ptr: *const u8,
    data_len: u32,
) -> u64 {
    let registered = read_str(family_ptr, family_len)
        .filter(|_| data_len != 0)
        .and_then(|family| {
            // SAFETY: the page wrote `data_len` bytes at `data_ptr`.
            let bytes = unsafe { std::slice::from_raw_parts(data_ptr, data_len as usize) };
            with_demo(|demo| demo.add_font(family, bytes.to_vec())).flatten()
        });
    leak(font_json(registered))
}

/// Lays out one paragraph. `line_height` 0 means normal; `phase2` packs the
/// bidi, justification, hanging and autospace options ([`Phase2`]).
///
/// # Safety
/// `text_ptr` must point to `text_len` bytes that stay readable for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_layout(
    text_ptr: *const u8,
    text_len: u32,
    width: f32,
    font_size: f32,
    line_height: f32,
    locale_ptr: *const u8,
    locale_len: u32,
    cjk_quotes: u32,
    primary_ptr: *const u8,
    primary_len: u32,
    heuristics: u32,
    phase2: u32,
) -> u64 {
    let Some(text) = read_str(text_ptr, text_len) else {
        return leak(error_json("input", "text is not UTF-8", None));
    };
    let Some(locale) = read_str(locale_ptr, locale_len) else {
        return leak(error_json("input", "locale is not UTF-8", None));
    };
    let Some(primary) = read_str(primary_ptr, primary_len) else {
        return leak(error_json("input", "face is not UTF-8", None));
    };
    let req = LayoutRequest {
        text,
        width,
        font_size,
        line_height,
        locale: (!locale.is_empty()).then_some(locale),
        cjk_quotes: cjk_quotes != 0,
        primary: (!primary.is_empty()).then_some(primary),
        heuristics: heuristics != 0,
        phase2: Phase2::from_bits(phase2),
    };
    leak(
        with_demo(|demo| demo.layout(&req))
            .unwrap_or_else(|| error_json("engine", "the engine is not initialised", None)),
    )
}

/// The outline of `glyph` in `font`, in font units, y down, as SVG path data.
#[unsafe(no_mangle)]
pub extern "C" fn demo_outline(font_id: u32, glyph_id: u32) -> u64 {
    leak(with_demo(|demo| demo.outline(font_id, glyph_id)).unwrap_or_default())
}

/// The components the engine itself reports as installed (spec SCO-8). A
/// `null` version means the component is not implemented.
#[unsafe(no_mangle)]
pub extern "C" fn demo_conformance() -> u64 {
    let report = conformance_report();
    leak(conformance_json(
        mutil_engine::UNICODE_DATA_VERSION,
        &report,
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_clear_paragraphs() {
    with_demo(Demo::clear_paragraphs);
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_set_options(
    line_break: u32,
    word_break: u32,
    letter_spacing: f32,
    word_spacing: f32,
    first_line_indent: f32,
    hanging_indent: f32,
) {
    with_demo(|demo| {
        demo.extra = ExtraOptions {
            line_break: options::line_break(line_break),
            word_break: options::word_break(word_break),
            letter_spacing,
            word_spacing,
            first_line_indent,
            hanging_indent,
        };
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_hit_test(para_id: u32, x: f32, y: f32) -> u64 {
    query("null", |demo| demo.hit_test(para_id as usize, x, y))
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_hit_test_on_line(para_id: u32, line: u32, inline_position: f32) -> u64 {
    query("null", |demo| {
        demo.hit_test_on_line(para_id as usize, line, inline_position)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_caret_geometry(para_id: u32, byte_offset: u32, affinity: u32) -> u64 {
    let pos = options::cursor(byte_offset, affinity);
    query("null", |demo| demo.caret_geometry(para_id as usize, pos))
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_move_cursor(
    para_id: u32,
    byte_offset: u32,
    affinity: u32,
    motion: u32,
    has_preferred: u32,
    preferred_inline: f32,
) -> u64 {
    let pos = options::cursor(byte_offset, affinity);
    let motion = options::motion(motion);
    let preferred = (has_preferred != 0).then_some(preferred_inline);
    query("null", |demo| {
        demo.move_cursor(para_id as usize, pos, motion, preferred)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_selection_geometry(
    para_id: u32,
    start: u32,
    end: u32,
    height_style: u32,
) -> u64 {
    let height = options::box_height(height_style);
    query("[]", |demo| {
        demo.selection_geometry(para_id as usize, start as usize, end as usize, height)
    })
}

/// The offset in paragraph `para_id`'s laid-out text of byte `source_offset`
/// of its source, past any locale tags; `u32::MAX` for no such paragraph.
#[unsafe(no_mangle)]
pub extern "C" fn demo_text_offset(para_id: u32, source_offset: u32) -> u32 {
    with_demo(|demo| demo.text_offset(para_id as usize, source_offset as usize))
        .flatten()
        .map_or(u32::MAX, |offset| offset as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_range_rects(para_id: u32, start: u32, end: u32) -> u64 {
    query("[]", |demo| {
        demo.range_rects(para_id as usize, start as usize, end as usize)
    })
}
