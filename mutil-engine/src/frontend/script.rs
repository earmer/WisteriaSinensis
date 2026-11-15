//! Script resolution (spec §3 Step 3, FE-13), the Phase 1 subset.
//!
//! An atom takes the script of its first character with a specific script,
//! so combining marks follow their base. An atom with none (Common or
//! Inherited only) takes the script of the nearest preceding resolved atom,
//! or at the paragraph start of the first following one. A paragraph with no
//! specific script at all resolves to `Latn` (FE-25). Script extensions,
//! paired punctuation and the CJK rules of FE-13 to FE-15 arrive with the
//! CJK step.

use super::segment::{SCRIPT, SCRIPT_NAMES};
use crate::types::ScriptTag;

/// The script of `c`, `Zyyy` or `Zinh` included.
pub(crate) fn char_script(c: char) -> ScriptTag {
    SCRIPT_NAMES
        .get(SCRIPT.get(c))
        .and_then(|name| ScriptTag::try_from_str(name).ok())
        .unwrap_or(ScriptTag::UNKNOWN)
}

/// The first specific script of `atom`'s characters, if any.
pub(crate) fn atom_script(atom: &str) -> Option<ScriptTag> {
    atom.chars()
        .map(char_script)
        .find(|script| !script.is_unresolved())
}

/// Resolves every `None` in `scripts` from its neighbours.
pub(crate) fn resolve_common(scripts: &[Option<ScriptTag>]) -> Vec<ScriptTag> {
    let first = scripts.iter().flatten().next().copied();
    let mut current = first.unwrap_or(ScriptTag::LATIN);
    scripts
        .iter()
        .map(|script| {
            if let Some(script) = script {
                current = *script;
            }
            current
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_atoms_take_the_preceding_script_or_the_first_following() {
        let greek = ScriptTag::GREEK;
        let scripts = [None, Some(greek), None, Some(ScriptTag::LATIN), None];
        assert_eq!(
            resolve_common(&scripts),
            [greek, greek, greek, ScriptTag::LATIN, ScriptTag::LATIN]
        );
        assert_eq!(resolve_common(&[None, None]), [ScriptTag::LATIN; 2]);
    }

    #[test]
    fn marks_follow_their_base() {
        assert_eq!(atom_script("e\u{301}"), Some(ScriptTag::LATIN));
        assert_eq!(atom_script("\u{301}"), None);
        assert_eq!(atom_script("1"), None);
        assert_eq!(atom_script("\u{3b1}"), Some(ScriptTag::GREEK));
    }
}
