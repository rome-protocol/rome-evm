use {
    crate::{
        api::split_u64, error::Result, state::State,
        msg,  error::RomeProgramError::InvalidInstructionData,
    },
    solana_program::{account_info::AccountInfo, pubkey::Pubkey},
};

// chain_id | holder_index
pub fn args(data: &[u8]) -> Result<(u64, u64)> {
    let (chain, data) = split_u64(data)?;
    let (holder, data) = split_u64(data)?;
    if !data.is_empty() {
        return Err(InvalidInstructionData);
    }

    Ok((chain, holder))
}

pub fn dealloc_resource<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: Deallocate resource");

    let (chain, holder) = args(data)?;
    let state = State::new(program_id, accounts, chain)?;

    if let Some(tx_holder) = state.info_tx_holder_opt(holder)? {
        state.remove_pda(tx_holder)?;
    }
    if let Some(state_holder) = state.info_state_holder_opt(holder)? {
        state.remove_pda(state_holder)?;
    }

    Ok(())
}
