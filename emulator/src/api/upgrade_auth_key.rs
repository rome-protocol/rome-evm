use {
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{
        error::Result,
        api::{
            join_treasure::{upgrade_authority_key, upgradeable_program_key,}
        },
    },
    solana_program::{pubkey::Pubkey, account_info::IntoAccountInfo},
    std::sync::Arc,
};

pub fn upgrade_auth_key<'a>(
    program_id: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
) -> Result<Pubkey> {

    let state = State::new_unchecked(program_id, None, client, 0)?;

    let mut bind = state.info_sys(program_id)?;
    let info = bind.into_account_info();
    let key = upgradeable_program_key(&info)?;

    let mut bind_ = state.info_sys(&key)?;
    let info_ = bind_.into_account_info();
    let upg_auth = upgrade_authority_key(&info_)?;

    Ok(upg_auth)
}