use {
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{accounts::OwnerInfo, error::Result, Data},
    solana_program::{account_info::IntoAccountInfo, msg, pubkey::Pubkey},
    std::sync::Arc,
};
pub fn get_rollups(program_id: &Pubkey, client: Arc<dyn AccountStorage>) -> Result<Vec<OwnerInfo>> {
    msg!("Get rollups");
    let state = State::new_unchecked(program_id, None, client, 0)?;

    if let Ok(mut bind) = state.info_owner_reg(false) {
        let info = bind.into_account_info();
        let rollups = OwnerInfo::from_account(&info)?;
        return Ok(rollups.to_vec());
    }

    Ok(vec![])
}
