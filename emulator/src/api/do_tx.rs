use {
    super::Emulation,
    crate::{state::State, ContextAt, account_storage::AccountStorage},
    rome_evm::{
        api::split_fee,
        error::Result,
        tx::tx::Tx,
        vm::{vm_atomic::{VmAt, MachineAt}, Execute},
    },
    solana_program::{msg, pubkey::Pubkey, instruction::Instruction,},
    std::sync::Arc,
};

pub fn do_tx<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    ed25519_ix: Option<Instruction>,
) -> Result<Emulation> {
    msg!("Instruction: Atomic transaction");
    let (fee_addr, pri_fee, rlp) = split_fee(data)?;
    let chain = Tx::chain_id_from_rlp(rlp)?;
    let state = State::new_ed25519(program_id, Some(*signer), client, chain, ed25519_ix)?;
    let context = ContextAt::new(&state);
    let vm = VmAt::new(&state, rlp, fee_addr, pri_fee, &context)?;
    atomic_tx(vm)
}

pub fn atomic_tx(mut vm: Box::<VmAt<State, ContextAt>>) -> Result<Emulation> {
    let state = vm.vm.handler.state;
    vm.consume(MachineAt::Lock)?;

    let (fee, refund) = state.get_fees();
    let report = Emulation::with_vm(
        &state,
        vm.vm.exit_reason,
        vm.vm.return_value,
        vm.vm.steps_executed,
        1,
        state.alloc(),
        state.dealloc(),
        state.alloc_payed(),
        state.dealloc_payed(),
        state.syscall.count(),
        fee,
        refund,
        *state.base.found_cpi.borrow(),
        None,
    );

    report
}
