//! A fast hasher for the Boolean's maps, whose keys are small integers
//! (point and plane ids, cell coordinates) made by Grid4D itself — never
//! by a file or a user, so the standard hasher's resistance to crafted
//! keys is not needed, and its cost is (a multiply and a rotate per word, as
//! in the hasher Firefox and rustc use).

use std::{
    collections::{HashMap, HashSet},
    hash::{BuildHasherDefault, Hasher},
};

#[derive(Clone, Copy, Default)]
pub(super) struct FastHasher(u64);

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FastHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_i64(&mut self, value: i64) {
        self.add(value.cast_unsigned());
    }

    fn write_usize(&mut self, value: usize) {
        self.add(u64::try_from(value).unwrap_or(u64::MAX));
    }
}

pub(super) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;
pub(super) type FastSet<K> = HashSet<K, BuildHasherDefault<FastHasher>>;
