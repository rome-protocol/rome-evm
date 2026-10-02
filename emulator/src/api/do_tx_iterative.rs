use {
    super::Emulation,
    crate::{
        context::ContextIt,
        state::State,
        account_storage::AccountStorage,
    },
    rome_evm::{
        api::do_tx_iterative::{
            args, verify_opcode_limit,
        },
        context::{AccountLock, Context},
        error::{Result, RomeProgramError::*},
        tx::tx::Tx,
        vm::{vm_iterative::{MachineIt, VmIt,}, Execute}, H256,
        SIG_VERIFY_COST, MAX_OPCODES_PER_EMULATION,
        accounts::Iterations,
    },
    solana_program::{keccak, msg, pubkey::Pubkey, instruction::Instruction,},
    std::sync::Arc,
};

fn apply_fee(state: &State) -> Result<()>{
    let signer = state.signer.unwrap();
    let mut bind = state.info_sys(&signer).map_err(|_| InvalidSigner)?;
    bind.1.lamports = bind.1.lamports.checked_sub(SIG_VERIFY_COST)
        .ok_or(InsufficientLamports(signer, bind.1.lamports))?;
    state.update(bind);
    Ok(())
}

pub fn iterative_tx(
    state: &State,
    context: ContextIt,
    limit: u64,
) -> Result<Emulation> {
    let mut steps = 0;
    let mut iteration = 0;
    let mut alloc = 0;
    let mut dealloc = 0;
    let mut alloc_payed = 0;
    let mut dealloc_payed = 0;
    let mut syscalls = 0;
    let mut found_cpi = false;
    let mut exec_cnt = 0;

    // TODO remove and use unique tx_id in data
    loop {
        msg!("  iteration {}", iteration);
        apply_fee(state)?;

        let mut vm = VmIt::new(state, &context, limit)?;
        iteration += 1;

        match vm.consume(MachineIt::FromStateHolder) {
            Err(UnnecessaryIteration(_)) => {
                msg!("Lock after emulation");
                vm.context.lock()?;
                // restore vm state
                vm.context.deserialize(&mut vm.vm)?;
                let (lmp_fee, lmp_refund) = vm.context.fees()?;

                return Emulation::with_vm(
                    state,
                    vm.vm.exit_reason,
                    vm.vm.return_value,
                    steps,
                    iteration - 1, // do not take into account the unnecessary iteration
                    alloc,
                    dealloc,
                    alloc_payed,
                    dealloc_payed,
                    syscalls,
                    lmp_fee,
                    lmp_refund,
                    found_cpi,
                    Some(exec_cnt),
                );
            }
            Err(e) => return Err(e),
            _ => {}
        }
        steps += vm.vm.steps_executed;
        // `limit` is per leg; the whole request shares the emulation budget.
        if steps >= MAX_OPCODES_PER_EMULATION {
            return Err(OpcodeLimitExceeded)
        }
        alloc += state.alloc();
        dealloc += state.dealloc();
        alloc_payed += state.alloc_payed();
        dealloc_payed += state.dealloc_payed();
        syscalls += state.syscall.count();
        found_cpi |= *state.base.found_cpi.borrow();

        match context.get_iteration()? {
            Iterations::Start => exec_cnt = 0,
            Iterations::Execute => exec_cnt += 1,
            _ => {},
        }

        state.reset();

        // TODO: init State before each iteration
        state.syscall.inc();    // state.info_owner_reg() 
        state.syscall.inc();    // state.info_state_holder()
        if context.tx_holder_key.is_some() {
            state.syscall.inc()     // state.info_tx_holder()            
        }
    }
}

pub fn do_tx_iterative<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    ed25519_ix: Option<Instruction>,
) -> Result<Emulation> {
    msg!("Instruction: Iterative transaction");
    let (session, holder, limit, fee_addr, pri_fee, rlp) = args(data)?;
    verify_opcode_limit(limit)?;
    let hash = H256::from(keccak::hash(rlp).to_bytes());
    let chain = Tx::chain_id_from_rlp(rlp)?;

    let state = State::new_ed25519(program_id, Some(*signer), client, chain, ed25519_ix)?;
    let state_holder = state.info_state_holder(holder, true)?;
    let context = ContextIt::new(&state, hash, session, fee_addr, pri_fee, rlp, None, state_holder.0)?;

    iterative_tx(&state, context, limit)
}
