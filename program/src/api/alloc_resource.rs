use {
    crate::{
        api::split_u64, error::Result, state::State,
        msg,  error::RomeProgramError::InvalidInstructionData,
        STATE_HOLDER_ALLOC_LEN,
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

pub fn alloc_resource<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: Allocate resource");

    let (chain, holder) = args(data)?;
    let state = State::new(program_id, accounts, chain)?;
    let state_holder = state.info_state_holder(holder, true)?;

    if state_holder.data_len() < STATE_HOLDER_ALLOC_LEN {
        let len = (state_holder.data_len() + state.alloc_limit()).min(STATE_HOLDER_ALLOC_LEN);
        state.realloc(state_holder, len)?;
    }

    Ok(())
}
