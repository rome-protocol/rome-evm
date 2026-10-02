use {
    super::Emulation,
    crate::{state::State, context::ContextAt, account_storage::AccountStorage},
    rome_evm::{
        api::deposit::{args, do_deposit},
        error::Result,
    },
    solana_program::{msg, pubkey::Pubkey, instruction::Instruction,},
    std::sync::Arc,
};

pub fn deposit<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    _: Option<Instruction>,
) -> Result<Emulation> {
    msg!("Instruction: deposit");

    let (chain, rlp) = args(data)?;
    let state = State::new(program_id, Some(*signer), client, chain)?;
    let context = ContextAt::new(&state);

    do_deposit(&state, &context, rlp)?;

    Emulation::without_vm(&state)
}
