use {
    super::Emulation,
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{
        error::Result,
        tx::legacy::Legacy,
        vm::{Execute, VmCall, MachineEthCall},
    },
    solana_program::{msg, pubkey::Pubkey},
    std::sync::Arc,
};

pub fn eth_call(
    program_id: &Pubkey,
    legacy: Legacy,
    client: Arc<dyn AccountStorage>,
    signer: &Pubkey,
) -> Result<Emulation> {
    msg!("eth_call");
    let state = State::new(program_id, Some(*signer), client, legacy.chain_id.as_u64())?;
    let mut vm = VmCall::new(&state, legacy)?;
    vm.consume(MachineEthCall::Init)?;

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
        0,
        0,
        *state.base.found_cpi.borrow(),
        None,
    );

    report
}
