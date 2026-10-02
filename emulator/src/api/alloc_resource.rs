use {
    super::Emulation,
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{
        api::alloc_resource::args, error::Result, STATE_HOLDER_ALLOC_LEN,
    },
    solana_program::{msg, pubkey::Pubkey, instruction::Instruction},
    std::sync::Arc,
};

pub fn alloc_resource<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    _: Option<Instruction>,
) -> Result<Emulation> {
    msg!("Instruction: Allocate resource");

    let (chain, holder) = args(data)?;
    let state = State::new(program_id, Some(*signer), client, chain)?;
    let (key, acc) = state.info_state_holder(holder, true)?;

    if acc.data.len() < STATE_HOLDER_ALLOC_LEN {
        let len = (acc.data.len() + state.alloc_limit()).min(STATE_HOLDER_ALLOC_LEN);
        state.realloc(&key, len, false)?;
    }

    Emulation::without_vm(&state)
}
