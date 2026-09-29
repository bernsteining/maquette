//! Plugin state that persists across calls: Typst keeps one wasm instance per
//! document and calls it repeatedly, so parsed models, derived meshes and
//! baked maps are kept here between calls.
//!
//! The wasm instance is single-threaded, which is what makes [`Global`]
//! sound; native builds (CLI, bindings, tests) must also call a plugin from
//! one thread at a time.

use std::cell::UnsafeCell;

/// A mutable plugin-wide global.
pub struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    pub const fn new(value: T) -> Self { Self(UnsafeCell::new(value)) }

    /// The value. Callers must not hold two overlapping mutable borrows of
    /// the same global, which a single-threaded plugin call never needs.
    #[allow(clippy::mut_from_ref)]
    pub fn get(&'static self) -> &'static mut T {
        unsafe { &mut *self.0.get() }
    }
}

/// Values keyed by a `u64` hash, evicted oldest first once `cap` entries
/// (`0` = unbounded) are held. Entries are boxed, so a reference to one stays
/// valid until that entry itself is evicted.
pub struct KeyedCache<T> {
    entries: Vec<(u64, Box<T>)>,
    cap: usize,
}

impl<T> KeyedCache<T> {
    pub const fn new(cap: usize) -> Self { Self { entries: Vec::new(), cap } }

    pub fn get(&self, key: u64) -> Option<&T> {
        self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| &**v)
    }

    pub fn contains(&self, key: u64) -> bool { self.entries.iter().any(|(k, _)| *k == key) }

    /// Store `value` under `key`, evicting the oldest entries to make room.
    pub fn insert(&mut self, key: u64, value: T) -> &T {
        if self.cap > 0 {
            while self.entries.len() >= self.cap { self.entries.remove(0); }
        }
        self.entries.push((key, Box::new(value)));
        &self.entries[self.entries.len() - 1].1
    }

    pub fn get_or_insert_with(&mut self, key: u64, make: impl FnOnce() -> T) -> &T {
        match self.entries.iter().position(|(k, _)| *k == key) {
            Some(i) => &self.entries[i].1,
            None => self.insert(key, make()),
        }
    }

    pub fn get_or_try_insert_with<E>(&mut self, key: u64, make: impl FnOnce() -> Result<T, E>) -> Result<&T, E> {
        match self.entries.iter().position(|(k, _)| *k == key) {
            Some(i) => Ok(&self.entries[i].1),
            None => Ok(self.insert(key, make()?)),
        }
    }

    /// Evict oldest entries until adding one of `incoming` bytes keeps the
    /// total, as measured by `size`, within `budget`.
    pub fn make_room(&mut self, incoming: usize, budget: usize, size: impl Fn(&T) -> usize) {
        while !self.entries.is_empty() && self.entries.iter().map(|(_, v)| size(v)).sum::<usize>() + incoming > budget {
            self.entries.remove(0);
        }
    }
}

/// Model handle: an 8-byte `magic` naming the plugin's format followed by the
/// model's cache key (`u64`, little-endian). A plugin hands one out so later
/// calls can name an already-parsed model instead of resending its bytes.
pub fn handle(magic: &[u8; 8], key: u64) -> Vec<u8> {
    let mut out = magic.to_vec();
    out.extend_from_slice(&key.to_le_bytes());
    out
}

/// The cache key of `data` if it starts with a `magic` handle.
pub fn handle_key(magic: &[u8; 8], data: &[u8]) -> Option<u64> {
    if data.len() >= 16 && data[..8] == magic[..] {
        Some(u64::from_le_bytes(data[8..16].try_into().unwrap()))
    } else {
        None
    }
}
