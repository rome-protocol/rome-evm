use {
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{error::Result, H160, origin::Origin},
    solana_program::{msg, pubkey::Pubkey},
    std::sync::Arc,
};

pub fn eth_get_tx_count<'a>(
    program_id: &'a Pubkey,
    address: &'a H160,
    client: Arc<dyn AccountStorage>,
    chain: u64,
) -> Result<u64> {
    msg!("eth_getTransactionCount");
    let state = State::new(program_id, None, client, chain)?;
    let nonce = state.nonce(address)?.unwrap_or(0);

    Ok(nonce)
}
