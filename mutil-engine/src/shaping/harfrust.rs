//! The HarfRust backend and its in-memory font registry (spec §4.7, SHP-21,
//! SHP-22).
//!
//! **Skeleton:** the registry stores and resolves faces; shaping, metrics
//! and font matching are not implemented yet, so [`HarfRustShaper`] is not a
//! `TextShaper` yet.

use super::{FontFace, FontSource};
use crate::types::{FontId, FontStyle, GenericFamily};
use read_fonts::model::Font;
use std::sync::Arc;

/// One registered face with the family and style it answers to.
#[derive(Clone, Debug)]
struct RegisteredFace {
    #[expect(
        dead_code,
        reason = "read by font matching, which is not implemented yet"
    )]
    family: Arc<str>,
    #[expect(
        dead_code,
        reason = "read by font matching, which is not implemented yet"
    )]
    style: FontStyle,
    face: FontFace,
}

/// An in-memory set of font faces: the id space of a [`HarfRustShaper`] and
/// the [`FontSource`] a renderer resolves its ids through (spec SHP-21,
/// HST-2).
///
/// Faces are added while the registry is being built; once it is shared it
/// no longer changes, so an id always names the same face. A `FontId` is the
/// index of its face in registration order and is never reused.
#[derive(Clone, Debug, Default)]
pub struct FontRegistry {
    faces: Vec<RegisteredFace>,
    generics: Vec<(GenericFamily, Arc<str>)>,
}

impl FontRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `face` under `family` and `style` and returns its id, or
    /// `None`, registering nothing, when its bytes hold no font at its face
    /// index. The bytes are shared, not copied (spec HST-1).
    pub fn add_face(
        &mut self,
        family: Arc<str>,
        style: FontStyle,
        face: FontFace,
    ) -> Option<FontId> {
        let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(face.bytes().clone());
        Font::new(bytes, face.face_index())?;
        let id = FontId(u32::try_from(self.faces.len()).ok()?);
        self.faces.push(RegisteredFace {
            family,
            style,
            face,
        });
        Some(id)
    }

    /// Names the family a generic keyword resolves to; a later call for the
    /// same keyword replaces the earlier one.
    pub fn set_generic(&mut self, generic: GenericFamily, family: Arc<str>) {
        self.generics.retain(|(keyword, _)| *keyword != generic);
        self.generics.push((generic, family));
    }
}

impl FontSource for FontRegistry {
    fn resolve_font(&self, font: FontId) -> Option<FontFace> {
        let index = usize::try_from(font.0).ok()?;
        self.faces
            .get(index)
            .map(|registered| registered.face.clone())
    }
}

/// The HarfRust shaping backend over one shared [`FontRegistry`]
/// (spec SHP-22). It is also the [`FontSource`] of its own id space.
#[derive(Clone, Debug)]
pub struct HarfRustShaper {
    registry: Arc<FontRegistry>,
}

impl HarfRustShaper {
    /// A shaper over `registry`.
    #[must_use]
    pub fn new(registry: Arc<FontRegistry>) -> Self {
        Self { registry }
    }

    /// The registry the shaper's ids belong to, to share with a renderer.
    #[must_use]
    pub fn registry(&self) -> &Arc<FontRegistry> {
        &self.registry
    }
}

impl FontSource for HarfRustShaper {
    fn resolve_font(&self, font: FontId) -> Option<FontFace> {
        self.registry.resolve_font(font)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaping::FontBytes;

    #[test]
    fn bytes_that_hold_no_font_are_not_registered() {
        let mut registry = FontRegistry::new();
        let face = FontFace::new(FontBytes::from_static(b"not a font"), 0);
        assert_eq!(
            registry.add_face("Test".into(), FontStyle::default(), face),
            None
        );
        assert!(registry.resolve_font(FontId(0)).is_none());
        let shaper = HarfRustShaper::new(Arc::new(registry));
        assert!(shaper.resolve_font(FontId(0)).is_none());
        assert!(shaper.registry().resolve_font(FontId(0)).is_none());
    }

    #[test]
    fn a_generic_keyword_keeps_its_last_family() {
        let mut registry = FontRegistry::new();
        registry.set_generic(GenericFamily::Serif, "A".into());
        registry.set_generic(GenericFamily::Serif, "B".into());
        assert_eq!(registry.generics, [(GenericFamily::Serif, Arc::from("B"))]);
    }

    /// The backend shares parsed fonts between layout threads (spec SHP-1).
    #[test]
    fn parsed_fonts_and_the_backend_are_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Font>();
        check::<FontRegistry>();
        check::<HarfRustShaper>();
    }
}
