use {
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{
        error::Result,
        origin::Origin,
        H160, U256,
    },
    solana_program::{msg, pubkey::Pubkey},
    std::sync::Arc,
};

pub fn eth_get_storage_at<'a>(
    program_id: &'a Pubkey,
    address: &'a H160,
    slot: &'a U256,
    client: Arc<dyn AccountStorage>,
    chain: u64,
) -> Result<U256> {
    msg!("eth_getStorage_at");
    let state = State::new(program_id, None, client, chain)?;
    let value = state.storage(address, slot)?.unwrap_or(U256::zero());

    Ok(value)
}
