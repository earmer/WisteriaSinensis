//! `AtomFlags` is a 16-bit mask of which twelve bits, 0 to 11, are assigned
//! (spec §2.2); the high four are reserved and masked off.

use mutil_engine::*;

const ASSIGNED: [AtomFlags; 12] = [
    AtomFlags::IS_CJK_IDEOGRAPH,
    AtomFlags::ALLOW_CJK_JUSTIFY_AFTER,
    AtomFlags::NO_LETTER_SPACING,
    AtomFlags::IS_EMOJI_SEQUENCE,
    AtomFlags::IS_OPEN_PUNCT,
    AtomFlags::IS_CLOSE_PUNCT,
    AtomFlags::HAS_VARIATION_SELECTOR,
    AtomFlags::IS_APOSTROPHE,
    AtomFlags::BINDS_TO_PREVIOUS,
    AtomFlags::L1_RESETTABLE,
    AtomFlags::HANGABLE_START,
    AtomFlags::HANGABLE_END,
];

#[test]
fn all_is_exactly_the_twelve_assigned_flags() {
    let mut union = AtomFlags::EMPTY;
    for (bit, flag) in ASSIGNED.into_iter().enumerate() {
        assert_eq!(flag.bits().count_ones(), 1, "flag {bit} is one bit");
        assert!(!union.intersects(flag), "flag {bit} is distinct");
        union |= flag;
    }
    assert_eq!(union, AtomFlags::ALL);
    assert_eq!(AtomFlags::ALL.bits().count_ones(), 12);
    // Bits 0 to 11 are the assigned ones.
    assert_eq!(AtomFlags::ALL.bits(), 0x0FFF);
    assert_eq!(AtomFlags::EMPTY.bits(), 0);
}

#[test]
fn the_reserved_high_bits_are_masked_off() {
    assert_eq!(AtomFlags::from_bits_truncate(u16::MAX), AtomFlags::ALL);
    assert_eq!(AtomFlags::from_bits_truncate(0xF000), AtomFlags::EMPTY);
    assert_eq!(!AtomFlags::EMPTY, AtomFlags::ALL);
    assert_eq!(!AtomFlags::ALL, AtomFlags::EMPTY);
    assert_eq!(AtomFlags::from_bits(0x1000), None);
    assert_eq!(AtomFlags::from_bits(0x0FFF), Some(AtomFlags::ALL));
}
