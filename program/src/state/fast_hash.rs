// Deterministic, fast (FxHash-style) hasher for the hot Pubkey/H160-keyed
// account maps. The default std hasher is SipHash, which spends ~180 BPF
// instructions per 32-byte key; on a swap that is ~13% of total CU because the
// State map is rebuilt per iterative leg. Keys here are already uniformly
// random (Solana Pubkeys, EVM H160s), so a cheap multiply-rotate mixer is
// collision-safe. Deterministic on SBF (no RandomState seed) -> consensus-safe.
use std::{
    collections::{HashMap, HashSet},
    hash::{BuildHasherDefault, Hasher},
};

/// HashMap/HashSet specialized to the deterministic FxHash-style hasher below.
/// Both stay Borsh-safe: borsh sorts map/set entries by key before serializing
/// (canonical wire format, hasher-independent), so swapping the hasher does not
/// change StateHolder bytes.
pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub type FastSet<T> = HashSet<T, BuildHasherDefault<FxHasher>>;

const SEED: u64 = 0x517c_c1b7_2722_0a95; // FxHash mixing constant

#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        while bytes.len() >= 8 {
            let mut w = [0u8; 8];
            w.copy_from_slice(&bytes[..8]);
            self.add(u64::from_le_bytes(w));
            bytes = &bytes[8..];
        }
        if !bytes.is_empty() {
            let mut w = [0u8; 8];
            w[..bytes.len()].copy_from_slice(bytes);
            self.add(u64::from_le_bytes(w));
        }
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::{FastMap, FastSet, FxHasher};
    use std::hash::{Hash, Hasher};
    fn h(b: &[u8; 32]) -> u64 {
        let mut s = FxHasher::default();
        b.hash(&mut s);
        s.finish()
    }
    #[test]
    fn deterministic() {
        let k = [7u8; 32];
        assert_eq!(h(&k), h(&k), "same key must hash identically");
    }
    #[test]
    fn distinct_keys_differ() {
        assert_ne!(h(&[1u8; 32]), h(&[2u8; 32]), "distinct keys must not collide");
        assert_ne!(h(&[0u8; 32]), h(&[1u8; 32]));
    }
    #[test]
    fn fastmap_roundtrips() {
        let mut m: FastMap<[u8; 32], u32> = FastMap::default();
        m.insert([1u8; 32], 42);
        m.insert([2u8; 32], 7);
        assert_eq!(m.get(&[1u8; 32]), Some(&42));
        assert_eq!(m.get(&[2u8; 32]), Some(&7));
        assert_eq!(m.get(&[9u8; 32]), None);
    }
    #[test]
    fn fastset_roundtrips() {
        let mut s: FastSet<[u8; 32]> = FastSet::default();
        s.insert([1u8; 32]);
        assert!(s.contains(&[1u8; 32]));
        assert!(!s.contains(&[2u8; 32]));
    }
}
