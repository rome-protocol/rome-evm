use {
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{error::Result, H160, U256, origin::Origin,},
    solana_program::{msg, pubkey::Pubkey},
    std::sync::Arc,
};

pub fn eth_get_balance<'a>(
    program_id: &'a Pubkey,
    address: &'a H160,
    client: Arc<dyn AccountStorage>,
    chain: u64,
) -> Result<U256> {
    msg!("eth_getBalance");
    let state = State::new(program_id, None, client, chain)?;
    let balance = state.balance(address)?.unwrap_or(U256::zero());

    Ok(balance)
}
