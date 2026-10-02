use {
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{error::{Result, RomeProgramError}, StateHolder, H256},
    solana_program::{account_info::IntoAccountInfo, pubkey::Pubkey},
    std::sync::Arc,
};

pub fn confirm_tx_iterative_opt(
    program_id: &Pubkey,
    holder: u64,
    hash: H256,
    signer: &Pubkey,
    client: Arc<dyn AccountStorage>,
    chain: u64,
    session: u64,
) -> Result<Option<bool>> {
    let state = State::new(program_id, Some(*signer), client, chain)?;
    let mut bind = state.info_state_holder(holder, false)?;
    let info = bind.into_account_info();

    if !StateHolder::has_session(&info, hash, session)? {
        return Err(RomeProgramError::Custom("state_holder doesn't have a session_id".to_string()))
    }

    Ok(StateHolder::get_iteration(&info)?.is_complete_opt())
}
