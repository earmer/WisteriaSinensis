//! Every byte buffer that crosses the page boundary, in one table.
//!
//! The page never holds a raw allocation: `demo_alloc` and every result word
//! hand it the address of a buffer this table owns, and `demo_free` takes only
//! that address. A foreign, repeated or stale address is ignored rather than
//! handed to the allocator, and inputs are read back through the table, so a
//! length the page got wrong is refused instead of read out of bounds.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// The address handed out for an empty buffer: never owned, so never freed.
pub const EMPTY: usize = 1;

/// Live buffers by address (a wasm32 address fits the page's `u32`).
static LIVE: Mutex<BTreeMap<usize, Box<[u8]>>> = Mutex::new(BTreeMap::new());

fn live() -> std::sync::MutexGuard<'static, BTreeMap<usize, Box<[u8]>>> {
    LIVE.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Takes ownership of `bytes` and returns its address, or [`EMPTY`] for none.
pub fn register(bytes: Box<[u8]>) -> usize {
    if bytes.is_empty() {
        return EMPTY;
    }
    let address = bytes.as_ptr() as usize;
    live().insert(address, bytes);
    address
}

/// A zeroed buffer of `len` bytes, or `None` when memory runs out.
pub fn zeroed(len: usize) -> Option<Box<[u8]>> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).ok()?;
    bytes.resize(len, 0);
    Some(bytes.into_boxed_slice())
}

/// Releases the buffer at `address`; anything else is a no-op.
pub fn release(address: usize) {
    live().remove(&address);
}

/// The `len` bytes the page wrote at `address`, copied; `None` unless
/// `address` is a live buffer at least `len` long. `len == 0` is always empty.
pub fn read(address: usize, len: usize) -> Option<Vec<u8>> {
    if len == 0 {
        return Some(Vec::new());
    }
    live().get(&address)?.get(..len).map(<[u8]>::to_vec)
}

/// How many buffers are live: zero between page calls.
pub fn count() -> usize {
    live().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The table is global and tests run in parallel, so these assert on their
    // own addresses; the live count is checked by the single-threaded wasm test.

    #[test]
    fn a_buffer_is_readable_until_released() {
        let at = register(zeroed(300).expect("memory"));
        assert_eq!(read(at, 300).map(|bytes| bytes.len()), Some(300));
        release(at);
        assert!(read(at, 1).is_none());
    }

    #[test]
    fn a_wrong_length_or_address_is_refused_not_read() {
        let at = register(zeroed(100).expect("memory"));
        assert!(read(at, 101).is_none());
        assert!(read(at + 1, 1).is_none());
        assert_eq!(read(at, 0), Some(Vec::new()));
        release(at);
    }

    #[test]
    fn releasing_twice_or_a_stranger_leaves_others_alone() {
        let kept = register(zeroed(64).expect("memory"));
        let gone = register(zeroed(64).expect("memory"));
        release(gone);
        release(gone);
        release(0);
        release(EMPTY);
        release(usize::MAX);
        assert!(read(kept, 64).is_some());
        release(kept);
    }

    #[test]
    fn an_empty_buffer_is_never_owned() {
        assert_eq!(register(zeroed(0).expect("memory")), EMPTY);
        assert!(read(EMPTY, 1).is_none());
    }

    #[test]
    fn an_impossible_size_is_none() {
        assert!(zeroed(usize::MAX).is_none());
    }
}
