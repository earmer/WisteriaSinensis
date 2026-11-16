//! Font bytes for renderers (spec §8, HST-1, HST-2).
//!
//! A renderer resolves a [`FontId`] to the exact face the shaper used through
//! a [`FontSource`], typically the font registry the shaper was built on and
//! shared with the rendering thread. The bytes are never copied: a
//! [`FontBytes`] borrows static data or shares ownership of any buffer,
//! including a memory-mapped file.

use crate::types::FontId;
use core::fmt;
use std::sync::Arc;

/// The bytes of a font file, shared without copying (spec HST-1).
///
/// Cheap to clone and safe to send to another thread. The representation is
/// private: static data is borrowed, and any other buffer (a `Vec<u8>`, a
/// memory map) is moved into a shared owner, so its bytes stay where they are.
#[derive(Clone)]
pub struct FontBytes(Repr);

#[derive(Clone)]
enum Repr {
    Static(&'static [u8]),
    Shared(Arc<dyn AsRef<[u8]> + Send + Sync>),
}

impl FontBytes {
    /// Borrows bytes that live for the whole program, such as an embedded
    /// font.
    #[must_use]
    pub const fn from_static(bytes: &'static [u8]) -> Self {
        Self(Repr::Static(bytes))
    }

    /// Takes ownership of any byte buffer. Moving a `Vec<u8>` or a memory
    /// map in moves its handle, not its bytes.
    #[must_use]
    pub fn new<T: AsRef<[u8]> + Send + Sync + 'static>(owner: T) -> Self {
        Self(Repr::Shared(Arc::new(owner)))
    }

    /// Shares a buffer that is already reference-counted elsewhere.
    #[must_use]
    pub fn from_shared(owner: Arc<dyn AsRef<[u8]> + Send + Sync>) -> Self {
        Self(Repr::Shared(owner))
    }

    /// The font file's bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.0 {
            Repr::Static(bytes) => bytes,
            Repr::Shared(owner) => (**owner).as_ref(),
        }
    }

    /// Returns a reference-counted handle to the underlying bytes without
    /// wrapping an already-shared buffer in a second `Arc`.
    pub(crate) fn shared_arc(&self) -> Arc<dyn AsRef<[u8]> + Send + Sync> {
        match &self.0 {
            Repr::Static(bytes) => Arc::new(*bytes),
            Repr::Shared(owner) => Arc::clone(owner),
        }
    }
}

impl AsRef<[u8]> for FontBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Debug for FontBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontBytes")
            .field("len", &self.as_bytes().len())
            .finish()
    }
}

/// One face of a font file: its bytes and its index in a collection
/// (spec HST-1).
#[derive(Clone, Debug)]
pub struct FontFace {
    bytes: FontBytes,
    face_index: u32,
}

impl FontFace {
    /// The face `face_index` of the font file `bytes` (0 for a single-face
    /// file).
    #[must_use]
    pub const fn new(bytes: FontBytes, face_index: u32) -> Self {
        Self { bytes, face_index }
    }

    /// The font file's bytes.
    #[must_use]
    pub const fn bytes(&self) -> &FontBytes {
        &self.bytes
    }

    /// The face's index in its file.
    #[must_use]
    pub const fn face_index(&self) -> u32 {
        self.face_index
    }
}

/// Resolves the [`FontId`]s a shaper issues to the faces it used
/// (spec FNT-3, HST-2).
///
/// Implemented by whatever owns the shaper's id space, usually a font
/// registry shared by the shaper and the renderer. A source never reuses an
/// id and keeps a face resolvable while any output that names it may still be
/// drawn (spec HST-2).
pub trait FontSource: Send + Sync {
    /// The face `font` names, or `None` when this source did not issue it.
    fn resolve_font(&self, font: FontId) -> Option<FontFace>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_bytes_are_borrowed_in_place() {
        static FONT: [u8; 4] = *b"OTTO";
        let bytes = FontBytes::from_static(&FONT);
        assert_eq!(bytes.as_bytes().as_ptr(), FONT.as_ptr());
    }

    #[test]
    fn an_owned_buffer_keeps_its_address() {
        let buffer = vec![0x00, 0x01, 0x00, 0x00];
        let address = buffer.as_ptr();
        let bytes = FontBytes::new(buffer);
        assert_eq!(bytes.as_bytes().as_ptr(), address);
        let clone = bytes.clone();
        assert_eq!(clone.as_bytes().as_ptr(), address);
    }

    #[test]
    fn a_shared_buffer_keeps_its_address() {
        let owner: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(vec![1_u8, 2, 3]);
        let address = (*owner).as_ref().as_ptr();
        let bytes = FontBytes::from_shared(Arc::clone(&owner));
        assert_eq!(bytes.as_bytes().as_ptr(), address);
        let shared = bytes.shared_arc();
        assert!(Arc::ptr_eq(&owner, &shared));
        assert_eq!((*shared).as_ref().as_ptr(), address);
        assert_eq!(Arc::strong_count(&owner), 3);
    }
}
