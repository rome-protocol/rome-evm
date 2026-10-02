//! Differential check of the ABI offset helpers against the pre-change
//! implementations. On every input where the old code returned, the new code
//! must return the same thing; the only permitted difference is an input where
//! the old code panicked, which must now be a typed error.

use {
    rome_evm::{
        error::{Result, RomeProgramError::*},
        non_evm::aux::{get_nested_slice, get_slice, get_usize, get_vec_bytes32},
        U256,
    },
    std::panic::{catch_unwind, AssertUnwindSafe},
};

fn fits_word(arr: &[u64; 4]) -> bool {
    arr[1] == 0 && arr[2] == 0 && arr[3] == 0
}

// ---- pre-change implementations, copied verbatim from origin/master ----

fn ref_get_usize(src: &[u8], offset: usize) -> Result<usize> {
    if src.len() < offset + 32 {
        return Err(ParseNonEvmInstructionDataError("len".to_string()));
    }
    let v = U256::from_big_endian(&src[offset..offset + 32]);
    let &U256(ref arr) = &v;
    if !fits_word(arr) || arr[0] > usize::MAX as u64 {
        return Err(ParseNonEvmInstructionDataError("cannot convert".to_string()));
    }
    Ok(arr[0] as usize)
}

fn ref_get_slice(abi: &[u8], pos: usize, element_size: usize) -> Result<&[u8]> {
    let mut offset = ref_get_usize(abi, pos)?;
    let len = ref_get_usize(abi, offset)?;
    let end = len * element_size;
    offset += 32;
    if abi.len() < offset + end {
        return Err(ParseNonEvmInstructionDataError("len".to_string()));
    }
    Ok(&abi[offset..offset + end])
}

fn ref_get_nested_slice(abi: &[u8], pos: usize) -> Result<&[u8]> {
    let mut offset = ref_get_usize(abi, pos)?;
    offset += pos;
    let len = ref_get_usize(abi, offset)?;
    offset += 32;
    if abi.len() < offset + len {
        return Err(ParseNonEvmInstructionDataError("len".to_string()));
    }
    Ok(&abi[offset..offset + len])
}

fn ref_get_vec_bytes32(abi: &[u8], pos: usize) -> Result<Vec<&[u8]>> {
    Ok(ref_get_slice(abi, pos, 32)?.chunks(32).collect())
}

// ---- corpus ----

fn word(n: u64) -> [u8; 32] {
    let mut w = [0_u8; 32];
    w[24..].copy_from_slice(&n.to_be_bytes());
    w
}

/// Deterministic xorshift so the corpus is reproducible without a dep.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

fn corpus() -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();

    // every truncation of a well-formed 2-element bytes32 array
    let full: Vec<u8> = [word(32), word(2), [0xaa; 32], [0xbb; 32]].concat();
    for n in 0..=full.len() {
        out.push(full[..n].to_vec());
    }

    // hostile scalars in the offset and length slots
    let nasty = [
        0u64, 1, 31, 32, 33, 64, 96, 128, 255, 256,
        u32::MAX as u64, u64::MAX / 32, u64::MAX / 32 + 1,
        usize::MAX as u64 - 31, usize::MAX as u64 - 32, u64::MAX - 1, u64::MAX,
    ];
    for a in nasty {
        for b in nasty {
            out.push([word(a), word(b), [0xcc; 32], [0xdd; 32]].concat());
            out.push([word(a), word(b)].concat());
        }
    }

    // high-limb values (do not fit a word)
    for shift in [64u32, 128, 192] {
        let v = U256::from(1u64) << shift;
        let mut w = [0u8; 32];
        v.to_big_endian(&mut w);
        out.push([w, word(2), [0xee; 32]].concat());
    }

    // random blobs, aligned and ragged
    let mut rng = Rng(0x9E3779B97F4A7C15);
    for i in 0..4000 {
        let len = (rng.next() % 200) as usize + if i % 3 == 0 { 1 } else { 0 };
        let mut v = vec![0u8; len];
        for b in v.iter_mut() {
            *b = (rng.next() & 0xff) as u8;
        }
        // bias a third of them toward small plausible offsets
        if i % 3 == 0 && len >= 32 {
            v[..32].copy_from_slice(&word(rng.next() % 160));
        }
        out.push(v);
    }
    out
}

#[derive(Default, Debug)]
struct Tally {
    agree: usize,
    old_panicked_new_errored: usize,
}

macro_rules! differential {
    ($name:ident, $new:expr, $old:expr) => {
        #[test]
        fn $name() {
            let mut t = Tally::default();
            for input in corpus() {
                // `pos` reaches these helpers from calldata (offsets_slice_of_slices
                // feeds get_usize an offset it just decoded), so hostile positions
                // are part of the real input space, not a synthetic case.
                for pos in [
                    0usize, 32, 64,
                    usize::MAX, usize::MAX - 1, usize::MAX - 31,
                    usize::MAX - 32, usize::MAX - 33, usize::MAX / 2,
                ] {
                    let old = catch_unwind(AssertUnwindSafe(|| $old(&input, pos)));
                    let new = catch_unwind(AssertUnwindSafe(|| $new(&input, pos)));

                    assert!(
                        new.is_ok(),
                        "NEW PANICKED (a fix must never panic): pos={pos} input={:02x?}",
                        &input[..input.len().min(48)]
                    );
                    let new = new.unwrap();

                    match old {
                        Err(_) => {
                            // the defect: old panicked. New must be a clean error.
                            assert!(
                                new.is_err(),
                                "old panicked but new returned Ok — semantics changed: \
                                 pos={pos} input={:02x?}",
                                &input[..input.len().min(48)]
                            );
                            t.old_panicked_new_errored += 1;
                        }
                        Ok(old) => {
                            // BEHAVIOUR MUST BE IDENTICAL on every input the old code handled.
                            match (&old, &new) {
                                (Ok(a), Ok(b)) => assert_eq!(
                                    a, b,
                                    "VALUE DIVERGENCE: pos={pos} input={:02x?}",
                                    &input[..input.len().min(48)]
                                ),
                                (Err(_), Err(_)) => {}
                                _ => panic!(
                                    "OK/ERR DIVERGENCE: old={:?} new={:?} pos={pos} input={:02x?}",
                                    old.is_ok(),
                                    new.is_ok(),
                                    &input[..input.len().min(48)]
                                ),
                            }
                            t.agree += 1;
                        }
                    }
                }
            }
            println!(
                "{}: {} inputs agree, {} where the old code panicked and the new one errors",
                stringify!($name),
                t.agree,
                t.old_panicked_new_errored
            );
            assert!(t.agree > 10_000, "corpus too small to mean anything: {t:?}");
            assert!(
                t.old_panicked_new_errored > 0,
                "corpus never hit the panic the fix exists for: {t:?}"
            );
        }
    };
}

// Results are copied to owned values so no borrow of the corpus escapes the closure.
fn own<T: ToOwned + ?Sized>(r: Result<&T>) -> Result<T::Owned> { r.map(ToOwned::to_owned) }
fn own_vec(r: Result<Vec<&[u8]>>) -> Result<Vec<Vec<u8>>> {
    r.map(|v| v.into_iter().map(<[u8]>::to_vec).collect())
}

differential!(get_usize_is_unchanged,
    |a: &[u8], p: usize| get_usize(a, p),
    |a: &[u8], p: usize| ref_get_usize(a, p));
differential!(get_nested_slice_is_unchanged,
    |a: &[u8], p: usize| own(get_nested_slice(a, p)),
    |a: &[u8], p: usize| own(ref_get_nested_slice(a, p)));
differential!(get_vec_bytes32_is_unchanged,
    |a: &[u8], p: usize| own_vec(get_vec_bytes32(a, p)),
    |a: &[u8], p: usize| own_vec(ref_get_vec_bytes32(a, p)));
differential!(get_slice_is_unchanged,
    |a: &[u8], p: usize| own(get_slice(a, p, 32)),
    |a: &[u8], p: usize| own(ref_get_slice(a, p, 32)));
