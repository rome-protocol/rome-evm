use {
    super::Emulation,
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{
        api::alloc_resource::args, error::Result,
    },
    solana_program::{msg, pubkey::Pubkey, instruction::Instruction,},
    std::sync::Arc,
};

pub fn dealloc_resource<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    _: Option<Instruction>,
) -> Result<Emulation> {
    msg!("Instruction: Deallocate resource");

    let (chain, holder) = args(data)?;
    let state = State::new(program_id, Some(*signer), client, chain)?;

    if let Some (tx_holder) = state.info_tx_holder_opt(holder)? {
        state.remove_pda(tx_holder)?
    }
    if let Some (state_holder) = state.info_state_holder_opt(holder)? {
        state.remove_pda(state_holder)?
    }

    Emulation::without_vm(&state)
}
