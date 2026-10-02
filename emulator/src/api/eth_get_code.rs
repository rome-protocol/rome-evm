use {
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{error::Result, H160, origin::Origin,},
    solana_program::{msg, pubkey::Pubkey},
    std::sync::Arc,
};

pub fn eth_get_code<'a>(
    program_id: &'a Pubkey,
    address: &'a H160,
    client: Arc<dyn AccountStorage>,
    chain: u64,
) -> Result<Vec<u8>> {
    msg!("eth_getCode");
    let state = State::new(program_id, None, client, chain)?;
    let code = state.code(address)?.unwrap_or(vec![]);

    Ok(code)
}
