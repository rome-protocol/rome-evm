use {
    super::{do_tx::atomic_tx, Emulation},
    crate::{state::State, account_storage::AccountStorage, context::ContextAt,},
    rome_evm::{
        api::do_tx_holder::{args, add_transmit_fee}, error::Result, Holder, vm::VmAt,
    },
    solana_program::{account_info::IntoAccountInfo, msg, pubkey::Pubkey, instruction::Instruction,},
    std::sync::Arc,
};

pub fn do_tx_holder<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    ed25519_ix: Option<Instruction>,
) -> Result<Emulation> {
    msg!("Instruction: Atomic transaction from holder");

    let (holder, hash, chain, fee_addr, pri_fee) = args(data)?;
    let state = State::new_ed25519(program_id, Some(*signer), client, chain, ed25519_ix)?;

    let mut bind = state.info_tx_holder(holder, false)?;
    let info = bind.into_account_info();
    add_transmit_fee(&state, &info)?;

    let rlp = Holder::rlp(&info, hash, chain)?;
    let context = ContextAt::new(&state);
    let vm = VmAt::new(&state, &rlp, fee_addr, pri_fee, &context)?;

    atomic_tx(vm)
}
