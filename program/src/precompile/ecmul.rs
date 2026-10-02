use {
    evm::H160, solana_bn254::{
        prelude::{
            alt_bn128_g1_multiplication_be,
        },
    }, crate::msg,
    crate::error::RomeProgramError::NonEvmCallError,
    super::impl_contract,
};

impl_contract!(Ecmul, [0_u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7,]);

const LEN: usize = 96;

fn contract(input: &[u8]) -> Result<Vec<u8>> {
    msg!("ecMul");

    let res = if input.len() >= LEN {
        alt_bn128_g1_multiplication_be(&input[..LEN])
    } else {
        let mut buf = vec![0_u8; LEN];
        buf[..input.len()].copy_from_slice(input);
        alt_bn128_g1_multiplication_be(&buf)
    };

    res.map_err(|e| NonEvmCallError(format!("ecMul: {e}")))
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
    fn g_times_zero_is_ok_zero_bytes() {
        // Scalar 0 -> identity; the identity's legitimate output is also 64
        // zero bytes. Guards the accidental-sentinel regression.
        let mut input = Vec::with_capacity(96);
        input.extend_from_slice(&word(1)); // G.x
        input.extend_from_slice(&word(2)); // G.y
        input.extend_from_slice(&word(0)); // scalar
        assert_eq!(contract(&input).unwrap(), vec![0_u8; 64]);
    }

    #[test]
    fn g_times_one_is_g() {
        let mut input = Vec::with_capacity(96);
        input.extend_from_slice(&word(1));
        input.extend_from_slice(&word(2));
        input.extend_from_slice(&word(1));
        let mut expected = Vec::with_capacity(64);
        expected.extend_from_slice(&word(1));
        expected.extend_from_slice(&word(2));
        assert_eq!(contract(&input).unwrap(), expected);
    }

    #[test]
    fn g_times_two_is_2g() {
        let mut input = Vec::with_capacity(96);
        input.extend_from_slice(&word(1));
        input.extend_from_slice(&word(2));
        input.extend_from_slice(&word(2));
        // Same 2G value independently derived in ecadd.rs's doubling test.
        let expected = hex::decode(
            "030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd3\
             15ed738c0e0a7c92e7845f96b2ae9c0a68a6a449e3538fc7ff3ebf7a5a18a2c4",
        ).unwrap();
        assert_eq!(contract(&input).unwrap(), expected);
    }

    #[test]
    fn off_curve_point_is_err() {
        // RED: P=(1,1) is off-curve; before the fix this returned a fabricated
        // 64-zero-byte success instead of failing the CALL.
        let mut input = Vec::with_capacity(96);
        input.extend_from_slice(&word(1));
        input.extend_from_slice(&word(1));
        input.extend_from_slice(&word(1));
        assert!(contract(&input).is_err());
    }
}
