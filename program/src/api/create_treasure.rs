use {
    crate::{
        error::{Result, RomeProgramError::*}, State, msg, split_u64, TREASURE_NUMBER,
    },
    solana_program::{
        account_info::AccountInfo, pubkey::Pubkey,
    },
    std::mem::size_of,
};

pub fn args(data: &[u8]) -> Result<(u64, u64, u64)> {
    if data.len() != size_of::<u64>() * 3 {
        return Err(InvalidInstructionData)
    }

    let (chain, data) = split_u64(data)?;
    let (from, data) = split_u64(data)?;
    let (to, _) = split_u64(data)?;

    if from >= to {
        return Err(InvalidInstructionData)
    }
    // Indices past the fee-routing pool are never touched on-chain; off-chain
    // each one costs an RPC fetch, so the emulator bounds the range.
    if cfg!(not(target_os = "solana")) && to > TREASURE_NUMBER {
        return Err(InvalidInstructionData)
    }
    Ok((chain, from, to))
}

pub fn create_treasure<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    let (chain, from, to) = args(data)?;
    msg!("Instruction: create_treasure {} {} {}", chain, from, to);
    let state = State::new_unchecked(program_id, accounts, chain)?;

    let _ = (from..to)
        .into_iter()
        .map(|i| state.info_treasure(i, true))
        .collect::<Result<Vec<_>>>()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(chain: u64, from: u64, to: u64) -> Vec<u8> {
        [chain, from, to].iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    #[test]
    fn range_past_the_treasure_pool_is_rejected() {
        assert!(matches!(args(&data(7, 0, u64::MAX)), Err(InvalidInstructionData)));
        assert!(matches!(args(&data(7, 60, TREASURE_NUMBER + 1)), Err(InvalidInstructionData)));
    }

    #[test]
    fn the_last_sdk_leg_ending_at_the_pool_size_is_accepted() {
        assert_eq!(args(&data(7, 40, TREASURE_NUMBER)).unwrap(), (7, 40, TREASURE_NUMBER));
        assert_eq!(args(&data(7, 60, TREASURE_NUMBER)).unwrap(), (7, 60, TREASURE_NUMBER));
    }
}
