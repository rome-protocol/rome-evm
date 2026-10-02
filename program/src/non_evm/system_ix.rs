use {
    solana_program::pubkey::{Pubkey, MAX_SEEDS, MAX_SEED_LEN},
    crate::{
        error::{Result, RomeProgramError::*,}, U256, origin::Origin,
        H160, non_evm::aux::*,
    },
    super::len_eq,
    std::{
        convert::TryFrom, str::FromStr,
    },
};

pub struct System<'a, T: Origin> {
    pub state: &'a T,
}

impl<'a, T: Origin> System<'a, T> {
    pub const ADDRESS: H160 = H160([
        0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x07,
    ]);
    pub fn new(state: &'a T) -> Self {
        Self {
            state
        }
    }
    pub fn find_pda(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (program_id, _) = get_pubkey(abi)?;
        let seeds = split_slice_of_slices(abi, 32)?;

        // Over MAX_SEEDS seeds, or any seed over MAX_SEED_LEN, aborts the whole Solana
        // instruction in the on-chain syscall rather than returning an error. Exactly
        // MAX_SEEDS is not refused here: it derives no PDA, because the bump needs a slot.
        if seeds.len() > MAX_SEEDS || seeds.iter().any(|s| s.len() > MAX_SEED_LEN) {
            return Err(InvalidNonEvmInstructionData);
        }
        let (key, bump) = Pubkey::try_find_program_address(seeds.as_slice(), &program_id)
            .ok_or(InvalidNonEvmInstructionData)?;

        let mut val = key.to_bytes().to_vec();
        val.resize(64, 0);
        *val.last_mut().unwrap() = bump;

        Ok(val)
    }
    /// Constant-time PDA derivation given a pre-computed bump.
    /// Wraps `solana_program::pubkey::Pubkey::create_program_address`. Returns the PDA
    /// or propagates the underlying `PubkeyError` if `seeds + [bump]` produces a curve
    /// point or exceeds Solana's seed-length limits.
    ///
    /// ABI: `create_program_address(bytes32 program, Seed[] seeds, uint8 bump) -> bytes32`
    pub fn create_program_address(abi: &[u8]) -> Result<Vec<u8>> {
        let (program_id, _) = get_pubkey(abi)?;
        let seeds = split_slice_of_slices(abi, 32)?;

        len_ge!(abi, 96);
        let bump = get_u8_unchecked(&abi[64..96])?;

        let bump_seed = [bump];
        let mut seeds_with_bump: Vec<&[u8]> = Vec::with_capacity(seeds.len() + 1);
        seeds_with_bump.extend_from_slice(seeds.as_slice());
        seeds_with_bump.push(&bump_seed);

        let key = Pubkey::create_program_address(seeds_with_bump.as_slice(), &program_id)?;
        Ok(key.to_bytes().to_vec())
    }
    pub fn bytes32_to_base58(abi: &[u8]) -> Result<Vec<u8>> {
        len_eq!(abi, 32);
        let key = Pubkey::try_from(abi).unwrap();
        let b58 = format!("{}", key);

        let offset: U256 = 32.into();
        let len: U256 = b58.len().into();

        let mut vec = vec![0_u8; 64];
        offset.to_big_endian(&mut vec[0..32]);
        len.to_big_endian(&mut vec[32..]);

        let mut a = b58.as_bytes().to_vec();
        vec.append(&mut a);
        Ok(vec)
    }
    pub fn base58_to_bytes32(abi: &[u8]) -> Result<Vec<u8>> {
        let b58 = get_slice(abi, 0, 1)?;
        let str = std::str::from_utf8(b58)
            .map_err(|_| InvalidNonEvmInstructionData)?;

        let key = Pubkey::from_str(&str)?;
        Ok(key.to_bytes().to_vec())
    }
    /// System Program pubkey (all-zeros) — matches `solana_program::system_program::id()`.
    /// Returned ABI-encoded as `bytes32`.
    pub fn program_id() -> Vec<u8> {
        Pubkey::default().to_bytes().to_vec()
    }
    pub fn mint_id(&self) -> Result<Vec<u8>> {
        let mint = self.state.base().owner_info().mint.unwrap_or_default();
        Ok(mint.to_bytes().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use {
        super::System,
        crate::state::State,
        solana_program::pubkey::Pubkey,
    };

    #[test]
    fn program_id_returns_bytes32_zero() {
        let out = System::<'static, State>::program_id();
        assert_eq!(out.len(), 32, "program_id must be 32 bytes");
        assert_eq!(out, vec![0u8; 32], "System Program id is all-zeros (bytes32(0))");
    }

    /// Build the ABI-encoded calldata (minus 4-byte selector) for
    /// `(bytes32 program, Seed[] seeds, uint8 bump)` where `Seed{bytes item;}`.
    ///
    /// Head: [program (32), offset-to-Seed[]=96 (32), bump (32, right-aligned)].
    /// Tail: [outer-len, per-Seed offset list, per-Seed tail (offset-to-bytes=32,
    /// length, padded data)]. Matches the Solidity ABI layout consumed by
    /// `find_pda`'s 2-arg variant plus a trailing uint8 bump word.
    fn encode_create_program_address_args(program: &Pubkey, seeds: &[&[u8]], bump: u8) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();

        out.extend_from_slice(&program.to_bytes());

        let seeds_offset: u64 = 96;
        let mut word = [0u8; 32];
        word[24..].copy_from_slice(&seeds_offset.to_be_bytes());
        out.extend_from_slice(&word);

        let mut word = [0u8; 32];
        word[31] = bump;
        out.extend_from_slice(&word);

        let mut tail: Vec<u8> = Vec::new();

        let mut word = [0u8; 32];
        word[24..].copy_from_slice(&(seeds.len() as u64).to_be_bytes());
        tail.extend_from_slice(&word);

        let mut seed_heads: Vec<u8> = Vec::new();
        let mut seed_tails: Vec<u8> = Vec::new();

        let head_len = 32 * seeds.len();
        let mut tail_cursor = head_len;
        for item in seeds {
            let mut word = [0u8; 32];
            word[24..].copy_from_slice(&(tail_cursor as u64).to_be_bytes());
            seed_heads.extend_from_slice(&word);

            let mut word = [0u8; 32];
            word[24..].copy_from_slice(&(32u64).to_be_bytes());
            seed_tails.extend_from_slice(&word);

            let mut word = [0u8; 32];
            word[24..].copy_from_slice(&(item.len() as u64).to_be_bytes());
            seed_tails.extend_from_slice(&word);

            let padded_len = item.len().div_ceil(32) * 32;
            let mut data = vec![0u8; padded_len];
            data[..item.len()].copy_from_slice(item);
            seed_tails.extend_from_slice(&data);

            tail_cursor += 32 + 32 + padded_len;
        }

        tail.extend_from_slice(&seed_heads);
        tail.extend_from_slice(&seed_tails);

        out.extend_from_slice(&tail);
        out
    }

    #[test]
    fn create_program_address_matches_find_program_address() {
        // `find_program_address` iterates bumps 255..=1 until it finds one that
        // produces a curve-off key. Feeding that same (seeds, bump) into
        // `create_program_address` must yield the identical key.
        let program = Pubkey::new_unique();
        let seeds: &[&[u8]] = &[b"rome", b"pda_deriver_test", &[0xDE, 0xAD, 0xBE, 0xEF]];

        let (expected_key, bump) = Pubkey::find_program_address(seeds, &program);

        let abi = encode_create_program_address_args(&program, seeds, bump);
        let out = System::<'static, State>::create_program_address(&abi)
            .expect("create_program_address handler should succeed");

        assert_eq!(out.len(), 32, "return must be 32-byte bytes32");
        assert_eq!(
            &out[..],
            &expected_key.to_bytes()[..],
            "create_program_address(seeds, bump) must equal find_program_address's key"
        );
    }
}
