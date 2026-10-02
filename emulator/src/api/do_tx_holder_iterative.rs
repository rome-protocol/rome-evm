use {
    super::{do_tx_iterative::iterative_tx, Emulation},
    crate::{context::ContextIt, state::State, account_storage::AccountStorage},
    rome_evm::{
        error::Result,
        Holder, api::{
            do_tx_holder_iterative::args, do_tx_iterative::verify_opcode_limit,
        },
    },
    solana_program::{account_info::IntoAccountInfo, msg, pubkey::Pubkey, instruction::Instruction,},
    std::sync::Arc,
};

pub fn do_tx_holder_iterative<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    ed25519_ix: Option<Instruction>,
) -> Result<Emulation> {
    msg!("Instruction: Iterative transaction from holder");

    let (session, holder, limit, hash, chain, fee_addr, pri_fee) = args(data)?;
    verify_opcode_limit(limit)?;
    let state = State::new_ed25519(program_id, Some(*signer), client, chain, ed25519_ix)?;

    let mut bind = state.info_tx_holder(holder, false)?;
    let tx_holder = bind.into_account_info();
    let rlp = Holder::rlp(&tx_holder, hash, chain)?;

    let state_holder = state.info_state_holder(holder, true)?;

    let context = ContextIt::new(&state, hash, session, fee_addr, pri_fee, &rlp, Some(*tx_holder.key), state_holder.0)?;
    iterative_tx(&state, context, limit)
}
