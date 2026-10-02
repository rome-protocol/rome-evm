use {
    evm::H160, solana_bn254::prelude::{alt_bn128_g1_addition_be, ALT_BN128_G1_ADDITION_INPUT_SIZE},
    crate::msg,
    crate::error::RomeProgramError::NonEvmCallError,
    super::impl_contract,
};

impl_contract!(Ecadd, [0_u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 6,]);

fn contract(input: &[u8]) -> Result<Vec<u8>> {
    msg!("ecAdd");
    // The addition accepts any length up to the two-point input and zero-pads a
    // short one itself; only an over-long input needs truncating.
    let end = input.len().min(ALT_BN128_G1_ADDITION_INPUT_SIZE);

    alt_bn128_g1_addition_be(&input[..end]).map_err(|e| NonEvmCallError(format!("ecAdd: {e}")))
}

#[cfg(test)]
mod test {
    use super::contract;

    fn word(n: u8) -> [u8; 32] {
        let mut w = [0_u8; 32];
        w[31] = n;
        w
    }

    #[test]
    fn identity_plus_identity_is_ok_zero_bytes() {
        // (0,0) is the EIP-196 point-at-infinity; the syscall special-cases it
        // rather than curve-checking it. Guards against the accidental-sentinel
        // regression: the identity's legitimate output is also 64 zero bytes.
        let input = [0_u8; 128];
        assert_eq!(contract(&input).unwrap(), vec![0_u8; 64]);
    }

    #[test]
    fn g_plus_g_is_2g() {
        let mut input = Vec::with_capacity(128);
        input.extend_from_slice(&word(1)); // G.x
        input.extend_from_slice(&word(2)); // G.y
        input.extend_from_slice(&word(1)); // G.x
        input.extend_from_slice(&word(2)); // G.y

        // 2G, computed independently via the doubling formula over the BN254
        // base field (lambda = 3x^2 / 2y; x3 = lambda^2 - 2x; y3 = lambda(x-x3)-y).
        let expected = hex::decode(
            "030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd3\
             15ed738c0e0a7c92e7845f96b2ae9c0a68a6a449e3538fc7ff3ebf7a5a18a2c4",
        ).unwrap();
        assert_eq!(contract(&input).unwrap(), expected);
    }

    #[test]
    fn off_curve_point_is_err() {
        // RED: P1=(1,1) is off-curve (1 != 1^3+3 mod p); P2=(0,0) is valid.
        // Before the fix the syscall's error was swallowed into a 64-zero-byte
        // success; after the fix it must surface as Err so the caller's CALL
        // fails instead of reporting a fabricated identity result.
        let mut input = Vec::with_capacity(128);
        input.extend_from_slice(&word(1));
        input.extend_from_slice(&word(1));
        input.extend_from_slice(&[0_u8; 64]);
        assert!(contract(&input).is_err());
    }

    #[test]
    fn short_input_is_zero_padded() {
        // Missing trailing bytes are zero, so a truncated P2 is the point at
        // infinity and G + inf = G for every length below the full 128.
        let mut g = Vec::with_capacity(64);
        g.extend_from_slice(&word(1));
        g.extend_from_slice(&word(2));

        for len in [64_usize, 65, 96, 127] {
            let mut input = g.clone();
            input.resize(len, 0);
            assert_eq!(contract(&input).unwrap(), g, "input length {len}");
        }
    }

    /// The implementation this replaced: a pad buffer sized for the output
    /// (one point) rather than the two-point input.
    fn reference(input: &[u8]) -> std::result::Result<Vec<u8>, ()> {
        use solana_bn254::prelude::{
            alt_bn128_g1_addition_be, ALT_BN128_G1_ADDITION_INPUT_SIZE, ALT_BN128_G1_POINT_SIZE,
        };
        let res = if input.len() >= ALT_BN128_G1_ADDITION_INPUT_SIZE {
            alt_bn128_g1_addition_be(&input[..ALT_BN128_G1_ADDITION_INPUT_SIZE])
        } else {
            let mut buf = vec![0_u8; ALT_BN128_G1_POINT_SIZE];
            buf[..input.len()].copy_from_slice(input);
            alt_bn128_g1_addition_be(&buf)
        };
        res.map_err(|_| ())
    }

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

    /// Every length up to past the two-point input, as a truncated generator,
    /// all zeros, and pseudo-random bytes.
    fn corpus() -> Vec<Vec<u8>> {
        let g: Vec<u8> = [word(1), word(2)].concat();
        let mut out = Vec::new();
        for len in 0..=200_usize {
            let mut a = g.clone();
            a.resize(len, 0);
            out.push(a);
            out.push(vec![0_u8; len]);
            let mut rng = Rng(0xDEAD_BEEF ^ len as u64);
            out.push((0..len).map(|_| (rng.next() & 0xff) as u8).collect());
        }
        out
    }

    /// On every input the old code returned, the new code must return the same
    /// bytes; the only permitted difference is an input the old code panicked on.
    #[test]
    fn matches_the_implementation_it_replaced() {
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let (mut agree, mut old_panicked) = (0_usize, 0_usize);
        for input in corpus() {
            let old = catch_unwind(AssertUnwindSafe(|| reference(&input)));
            let new = catch_unwind(AssertUnwindSafe(|| contract(&input)));

            assert!(
                new.is_ok(),
                "the fix must never panic, and it did at len {}",
                input.len()
            );
            let new = new.unwrap();

            if let Ok(old) = old {
                match (&old, &new) {
                    (Ok(a), Ok(b)) => assert_eq!(a, b, "value divergence at len {}", input.len()),
                    (Err(_), Err(_)) => {}
                    _ => panic!("ok/err divergence at len {}", input.len()),
                }
                agree += 1;
            } else {
                // The old code aborted the whole Solana instruction here. The new
                // code must return instead — Ok for a valid padded pair, Err for an
                // off-curve one; both are a CALL-level outcome, not an abort.
                old_panicked += 1;
            }
        }
        println!("ecadd: {agree} agree, {old_panicked} where the old code panicked");
        assert!(agree > 400, "corpus too small: {agree}");
        assert!(old_panicked >= 63, "panic band 65..=127 under-covered: {old_panicked}");
    }
}
