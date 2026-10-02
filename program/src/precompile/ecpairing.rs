use {
    evm::{
        H160, U256,
    },
    solana_bn254::prelude::{
        alt_bn128_pairing_be, ALT_BN128_PAIRING_ELEMENT_SIZE,
    },
    crate::msg, super::impl_contract,
    crate::error::RomeProgramError::NonEvmCallError,
};

impl_contract!(Ecpairing, [0_u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8,]);

fn contract(input: &[u8]) -> Result<Vec<u8>> {
    msg!("ecPairing");
    if input.is_empty() {
        let mut vec = vec![0_u8; 32];
        U256::one().to_big_endian(&mut vec[0..]);
        return Ok(vec)
    }

    if (input.len() % ALT_BN128_PAIRING_ELEMENT_SIZE) != 0 {
        return Err(NonEvmCallError("ecPairing: input length not a multiple of 192".to_string()))
    }

    let res = alt_bn128_pairing_be(input);
    res.map_err(|e| NonEvmCallError(format!("ecPairing: {e}")))
}

#[cfg(test)]
mod test {
    use super::contract;

    fn word_one() -> Vec<u8> {
        let mut v = vec![0_u8; 32];
        v[31] = 1;
        v
    }

    #[test]
    fn empty_input_is_ok_word_one() {
        assert_eq!(contract(&[]).unwrap(), word_one());
    }

    #[test]
    fn identity_pair_is_ok_word_one() {
        // G1=(0,0), G2=(0,0,0,0): both special-cased to the group identity.
        // e(id, id) = 1 in the target group -> word 1, success. Guards the
        // accidental-sentinel regression (an all-zero *result* is legitimate
        // elsewhere in this file; this checks an all-zero well-formed *input*
        // still succeeds after the signature change).
        let input = vec![0_u8; 192];
        assert_eq!(contract(&input).unwrap(), word_one());
    }

    #[test]
    fn bad_length_is_err() {
        // RED: 100 bytes is not a multiple of 192. Before the fix this
        // returned a fabricated zero-word success instead of failing the CALL.
        let input = vec![0_u8; 100];
        assert!(contract(&input).is_err());
    }

    #[test]
    fn off_curve_element_is_err() {
        // RED: G1=(1,1) is off-curve; G2=(0,0,0,0) is the valid identity.
        // Before the fix the syscall's GroupError was swallowed into a
        // fabricated zero-word success.
        let mut input = vec![0_u8; 192];
        input[31] = 1; // G1.x = 1
        input[63] = 1; // G1.y = 1
        assert!(contract(&input).is_err());
    }
}
