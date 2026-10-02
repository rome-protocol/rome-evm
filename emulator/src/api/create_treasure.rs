use {
    super::Emulation,
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{
        error::Result, api::create_treasure::args
    },
    solana_program::{msg, pubkey::Pubkey, instruction::Instruction,},
    std::sync::Arc,
};

pub fn create_treasure<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    _: Option<Instruction>,
) -> Result<Emulation> {
    let (chain, from, to) = args(data)?;
    msg!("Instruction: create_treasure {} {} {}", chain, from, to);

    let state = State::new_unchecked(program_id, Some(*signer), client, chain)?;

    let _ = (from..to)
        .into_iter()
        .map(|ix| state.info_treasure(ix, true))
        .collect::<Result<Vec<_>>>()?;

    Emulation::without_vm(&state)
}
