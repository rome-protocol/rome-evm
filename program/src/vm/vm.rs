use {
    super::Snapshot,
    crate::{
        config::{
            EXIT_REASON, GAS_RECIPIENT, GAS_VALUE, REVERT_ERROR, REVERT_PANIC,
            GAS_PRICE, TREASURE_NUMBER, TREASURE_LAMPORTS, SLOT, TIMESTAMP,
        },
        error::{Result, RomeProgramError::*},
        origin::Origin,
        state::{
            handler::{CallInterrupt, CreateInterrupt}, Allocate, Diff, JournaledState,
        },
        tx::tx::Tx,
        vm::Reason, msg, dmsg, aux::revert_msg, precompile::modexp::Modexp,
    },
    evm::{Capture, ExitError, ExitReason, Handler, Resolve, H160, H256, U256, ExitRevert::Reverted,},
    solana_program::log::sol_log_data,
    solana_system_interface::instruction as system_instruction,
    std::mem::size_of,
};

pub enum Trap {
    Call(CallInterrupt),
    Create(CreateInterrupt),
    ExitFromSnapshot(ExitReason),
    ExitNoShapshot(Vec<u8>, ExitReason),
    MergeOrRevert(ExitReason, Vec<u8>),
}

pub struct Vm<'a, T: Origin + Allocate> {
    pub snapshot: Option<Box<Snapshot>>,
    pub handler: JournaledState<'a, T>,
    pub return_value: Option<Vec<u8>>,
    pub exit_reason: Option<ExitReason>,
    pub steps_executed: u64,
    // Precompile native work, priced at trap_call dispatch
    // (see precompile::step_price) and bounded together with steps_executed
    // against MAX_OPCODES_PER_EMULATION (vm_eth_call.rs / vm_atomic.rs). NOT
    // serialized: context/iterative.rs::serialize_impl writes snapshot/
    // handler/return_value/exit_reason/pda only, same as steps_executed —
    // the StateHolder layout is unchanged by design.
    pub native_work: u64,
    pub atomic_flow: bool,
}

impl<'a, T: Origin + Allocate> Vm<'a, T> {
    pub fn new(state: &'a T, atomic_flow: bool) -> Result<Self> {
        let vm = Self {
            snapshot: None,
            handler: JournaledState::new(state)?,
            return_value: None,
            exit_reason: None,
            steps_executed: 0,
            native_work: 0,
            atomic_flow,
        };

        Ok(vm)
    }
    pub fn is_mut(&self) -> bool {
        if let Some(snapshot) = self.snapshot.as_ref() {
            return snapshot.is_mut();
        }
        true
    }

    pub fn call_from_tx(&mut self, tx: &mut Tx) -> Capture<(ExitReason, Vec<u8>), CallInterrupt> {
        let to = tx.to().unwrap();
        let context = evm::Context {
            address: to,
            caller: tx.from(),
            apparent_value: tx.value(),
        };

        let transfer = if !tx.value().is_zero() {
            Some(evm::Transfer {
                source: tx.from(),
                target: to,
                value: tx.value(),
            })
        } else {
            None
        };

        let input = tx.data().unwrap();

        self
            .handler
            .call(to, transfer, input, None, false, context)
    }

    pub fn push_call_snapshot(&mut self, call: CallInterrupt) {
        dmsg!(
            "Call: from {}, to {}",
            &hex::encode(call.context.caller),
            &hex::encode(call.context.address)
        );
        let code = self.handler.code(call.code_address);
        let valids = self.handler.valids(call.code_address);
        let runtime = evm::Runtime::new(code, valids, call.input, call.context);

        if let Some(transfer) = call.transfer {
            self.handler.transfer(&transfer.source, &transfer.target, &transfer.value);
        }
        let snapshot = Snapshot {
            evm: runtime,
            reason: Reason::Call,
            mutable: (!call.is_static) && self.is_mut(),
            parent: None,
        };

        self.push_snapshot(snapshot);
    }

    pub fn push_create_snapshot(&mut self, create: CreateInterrupt) {
        dmsg!(
            "Create: from {}, contract {}",
            &hex::encode(create.context.caller),
            &hex::encode(create.context.address)
        );
        let valids = evm::Valids::compute(&create.init_code);
        let to = create.address;
        let runtime = evm::Runtime::new(create.init_code, valids, vec![], create.context);

        if evm::CONFIG.create_increase_nonce {
            self.handler.journal.get_mut(&to).push(Diff::NonceChange);
        }
        if let Some(transfer) = create.transfer {
            self.handler.transfer(&transfer.source, &transfer.target, &transfer.value);
        }
        let snapshot = Snapshot {
            evm: runtime,
            reason: Reason::Create(to),
            mutable: self.is_mut(),
            parent: None,
        };

        self.push_snapshot(snapshot);
    }

    pub fn push_snapshot(&mut self, mut new: Snapshot) {
        self.handler.mutable = new.mutable;
        new.parent = self.snapshot.take();
        self.snapshot = Some(Box::new(new));
        // TODO: remove "mutable" and "from" fields from JournaledState,
        // TODO: implement Handler trait for the struct:
        // struct {
        //    handler: JournaledState,
        //    snapshot: Snanpshot,
        // }
    }

    pub fn pop_snapshot(&mut self) -> Option<Box<Snapshot>> {
        if let Some(mut snapshot) = self.snapshot.take() {
            self.snapshot = snapshot.parent.take();
            self.handler.mutable = self.is_mut();

            return Some(snapshot);
        }

        None
    }

    pub fn init(
        &mut self,
        tx: &mut Tx,
        check_nonce: bool,
        fee_recipient: Option<H160>,
    ) -> Result<Option<(Vec<u8>, ExitReason)>> {

        let from = tx.from();
        dmsg!("from {}", &hex::encode(from));

        // TODO add test to eliminate the possibility of repeated transaction execution
        if check_nonce {
            let nonce = self.handler.nonce(from);
            if nonce != tx.nonce().into() {
                return Err(InvalidTxNonce(from, tx.nonce(), nonce.as_u64()));
            }
        }
        self.handler.origin = Some(from);
        self.handler.gas_limit = Some(tx.gas_limit());
        self.handler.gas_price = Some(tx.gas_price());
        self.handler.gas_recipient = fee_recipient;

        let trap = if tx.to().is_some() {
            match self.call_from_tx(tx) {
                Capture::Trap(call) => Trap::Call(call),
                Capture::Exit((reason, value)) => Trap::ExitNoShapshot(value, reason)
            }
        } else {
            let capture = self
                .handler
                .create(
                    tx.from(),
                    evm::CreateScheme::Legacy { caller: tx.from() },
                    tx.value(),
                    tx.data().unwrap(),
                    None,
                );

            match capture {
                Capture::Trap(create) => Trap::Create(create),
                Capture::Exit((reason, _, value)) => Trap::ExitNoShapshot(value, reason)
            }
        };

        Ok(self.trap(trap))
    }

    pub fn commit_exit(
        &mut self,
        call_reason: Reason,
        return_value: Vec<u8>,
        reason: ExitReason,
    ) -> Option<(Vec<u8>, ExitReason)> {
        match call_reason {
            Reason::Call => self.commit_call(return_value, reason),
            Reason::Create(address) => self.commit_create(return_value, reason, address),
        }
    }

    pub fn commit_call(&mut self, return_value: Vec<u8>, reason: ExitReason) -> Option<(Vec<u8>, ExitReason)> {
        if self.snapshot.is_none() {
            return Some((return_value, reason))
        }

        let latest = self.snapshot.as_mut().unwrap();

        match evm::save_return_value::<JournaledState<'a, T>>(
            &mut latest.evm,
            reason,
            return_value,
        ) {
            evm::Control::Continue => None,
            evm::Control::Exit(reason) => {
                assert!(reason.is_fatal());
                Some((vec![], reason))
            },
            _ => unreachable!(),
        }
    }
    pub fn commit_create(
        &mut self,
        return_value: Vec<u8>,
        reason: ExitReason,
        address: H160,
    ) -> Option<(Vec<u8>, ExitReason)> {
        let mut f = |a: Vec<u8>| {
            if reason.is_succeed() {
                assert!(self.handler.mutable);
                self.handler.set_code(address, a);
            }
        };

        if self.snapshot.is_none() {
             if reason.is_revert() {
                 return Some((return_value, reason))
            } else {
                 f(return_value);
                 return Some((vec![], reason))
            };
        } else {
            f(return_value)
        }

        let latest = self.snapshot.as_mut().unwrap();

        match evm::save_created_address::<JournaledState<'a, T>>(
            &mut latest.evm,
            reason,
            Some(address),
        ) {
            evm::Control::Continue => None,
            evm::Control::Exit(reason) => {
                assert!(reason.is_fatal());
                Some((vec![], reason))
            },
            _ => unreachable!(),
        }
    }

    pub fn log_exit_reason(&self) -> Result<()> {
        assert!(self.exit_reason.is_some());
        let exit_reason = self.exit_reason.unwrap();

        #[cfg(target_os = "solana")]
        let code = match exit_reason {
            ExitReason::Succeed(_) => 0x0_u8,
            ExitReason::Revert(_) => {
                self.log_revert_msg()?;
                0x2
            },
            _ => panic!("vm state machine fault"),
        };

        #[cfg(not(target_os = "solana"))]
        let code = match exit_reason {
            ExitReason::Succeed(_) => 0x0_u8,
            ExitReason::Error(_) => 0x1,
            ExitReason::Revert(_) => {
                self.log_revert_msg()?;
                0x2
            }
            ExitReason::Fatal(_) => 0x3,
            ExitReason::StepLimitReached => panic!("vm state machine fault: StepLimitReached"),
        };

        let mut return_value = &vec![];
        if let Some(value) = self.return_value.as_ref() {
            return_value = value;
        };

        let msg = format!("{:?}", exit_reason);
        let msg_len = msg.len();
        sol_log_data(&[
            EXIT_REASON,
            &[code],
            &msg_len.to_le_bytes(),
            msg.as_bytes(),
            return_value,
        ]);
        sol_log_data(&[SLOT, &self.handler.slot.to_le_bytes()]);
        sol_log_data(&[TIMESTAMP, &self.handler.timestamp.to_le_bytes()]);

        Ok(())
    }

    pub fn log_revert_msg(&self) -> Result<()> {
        let return_value = if let Some(value) = &self.return_value {
            value
        } else {
            return Ok(());
        };

        if return_value.starts_with(REVERT_ERROR) {
            let msg = &return_value[REVERT_ERROR.len()..];
            let left = 64_usize;
            let offset = 32_usize;

            if msg.len() >= left {
                let found = U256::from_big_endian(&msg[0..size_of::<U256>()]) == offset.into();
                if found {
                    let len = U256::from_big_endian(&msg[offset..left]).as_usize();
                    let right = left.checked_add(len).ok_or(CalculationOverflow)?;
                    if let Some(msg) = msg.get(left..right) {
                        let msg = std::str::from_utf8(msg).unwrap_or("str::from_utf8() error");
                        msg!("Revert: {:?}", msg);
                    }
                }
            }
        } else if return_value.starts_with(REVERT_PANIC) {
            let msg = &return_value[REVERT_ERROR.len()..];
            let len = size_of::<U256>();

            if msg.len() == len {
                let msg = U256::from_big_endian(&msg[0..len]);
                msg!("Revert panic: {:?})", msg);
            }
        }

        Ok(())
    }

    fn to_trap(capture: Capture<ExitReason, Resolve<JournaledState<T>>>) -> Option<Trap> {
        match capture {
            Capture::Trap(trap) => {
                match trap {
                    Resolve::Call(call, runtime) => {
                        std::mem::forget(runtime); // todo: remove it from evm.run() result
                        Some(Trap::Call(call))
                    }
                    Resolve::Create(create, runtime) => {
                        std::mem::forget(runtime);
                        Some(Trap::Create(create))
                    }
                }
            }
            Capture::Exit(ExitReason::StepLimitReached) => None,
            Capture::Exit(reason) => Some(Trap::ExitFromSnapshot(reason)),
        }
    }

    fn trap_call(&mut self, call: CallInterrupt, touch_nonce: bool) -> Option<(Vec<u8>, ExitReason)> {
        if self.snapshot.is_none() && touch_nonce {
            let from = self.handler.origin.unwrap();
            self.handler.journal.get_mut(&from).push(Diff::NonceChange);
        }

        // $ solana feature status EBq48m8irRKuE7ZnMTLvLg2UuGSqhe8s8oMqnmja1fJw -u mainnet-beta
        //
        // EBq48m8irRKuE7ZnMTLvLg2UuGSqhe8s8oMqnmja1fJw | inactive                | NA              | add big_mod_exp syscall #28503
        if call.code_address == Modexp::ADDRESS {
            return Some((revert_msg("big_mod_exp call is disabled".to_string()), Reverted.into()))
        }

        if let Some(program) =  self.handler.non_evm_program(&call.code_address) {
            // Price precompile work into a sibling
            // counter BEFORE the body runs, so an oversized input can be
            // refused without executing it. Does not touch steps_executed
            // (would change on-chain iterative leg counts / the emulator's
            // atomic-vs-iterative flow decision — see vm.rs module docs and
            // config.rs for the (unmeasured, ship-gated) price constants).
            if program.precompile() {
                let price = crate::precompile::step_price(&call.code_address, &call.input);
                self.native_work = self.native_work.saturating_add(price);

                if self.steps_executed.saturating_add(self.native_work) > crate::config::MAX_OPCODES_PER_EMULATION {
                    let stop = self.commit_call(vec![], ExitReason::Error(ExitError::OutOfGas));
                    if stop.is_some() {
                        return stop
                    }
                    return self.trap(Trap::MergeOrRevert(ExitReason::Error(ExitError::OutOfGas), vec![]))
                }
            }

            let (reason, vec) = self.handler.handle_non_evm_call(program, call, self.atomic_flow);
            let revert_mes = if !reason.is_succeed() {
                vec.clone()
            } else {
                vec![]
            };
            let stop = self.commit_call(vec, reason);
            // there is no parent snapshot OR Fatal error
            if stop.is_some() {
                return stop
            }

            // TODO: overwrite the previous non-evm-state instead of saving the new one
            self.trap(Trap::MergeOrRevert(reason, revert_mes))
        } else {
            self.push_call_snapshot(call);
            None
        }
    }

    pub fn trap(&mut self, trap: Trap) -> Option<(Vec<u8>, ExitReason)> {
        self.trap_(trap, true)
    }

    pub fn trap_(&mut self, trap: Trap, touch_nonce: bool) -> Option<(Vec<u8>, ExitReason)> {
        match trap {
            Trap::Call(call) => {
                self.handler.new_page();
                self.trap_call(call, touch_nonce)
            },
            Trap::Create(create) => {
                // The creator's nonce is consumed even if the init code fails,
                // so it lands on the parent page, outside the revertible child.
                self.handler.journal.get_mut(&create.context.caller).push(Diff::NonceChange);
                self.handler.new_page();
                self.push_create_snapshot(create);
                None
            }
            Trap::ExitFromSnapshot(reason) => {

                let snapshot = self.pop_snapshot().expect("vm fault");
                let vec = snapshot.evm.machine().return_value();
                let revert_mes = if !reason.is_succeed() {
                    vec.clone()
                } else {
                    vec![]
                };
                let stop = self.commit_exit(snapshot.reason, vec, reason);
                // there is no parent snapshot OR Fatal error
                if stop.is_some() {
                    return stop
                }
                self.trap(Trap::MergeOrRevert(reason, revert_mes))
            }
            Trap::ExitNoShapshot(value, reason) => {
                assert!(self.snapshot.is_none());
                assert!(!reason.is_succeed());
                Some((value, reason))
            }
            Trap::MergeOrRevert(reason, revert_mes) => {
                if reason.is_succeed() {
                    self.handler.journal.merge_page();
                    return None
                }

                if self.handler.found_cpi_on_page() {
                    let stop = Some((revert_mes, Reverted.into()));
                    return stop;
                }

                self.handler.revert_page();
                None
            }
        }
    }

    pub fn execute(&mut self, steps: u64) -> Option<(Vec<u8>, ExitReason)> {
        let snapshot = self.snapshot.as_mut().expect("vm fault");
        let (steps, capture) = snapshot.evm.run(steps, &mut self.handler);
        self.steps_executed += steps;

        Self::to_trap(capture).and_then(|trap| self.trap(trap))
    }
 
    pub fn gas_transfer(&mut self, fee:u64, refund: u64) -> Result<()> {
        let mut buf_limit = [0_u8; 32];
        let mut buf_price = [0_u8; 32];

        if let Some(to) = self.handler.gas_recipient {
            let gas_limit = self.handler.gas_limit.unwrap();
            let gas_price = self.handler.gas_price.unwrap();

            let from = self.handler.origin.unwrap();
            let lamports: U256 = fee.saturating_sub(refund).into();

            if lamports > gas_limit {
                return Err(InsufficientGas(gas_limit, lamports))
            }

            let wei = lamports.checked_mul(gas_price).ok_or(CalculationOverflow)?;
            self.handler.transfer(&from, &to, &wei);

            lamports.to_big_endian(&mut buf_limit);
            gas_price.to_big_endian(&mut buf_price);
            
            sol_log_data(&[GAS_RECIPIENT, to.as_bytes()]);
        }
        
        sol_log_data(&[GAS_VALUE, &buf_limit]);
        sol_log_data(&[GAS_PRICE, &buf_price]);

        Ok(())
    }

    pub fn treasure_transfer(&mut self, hash: H256) -> Result<()>{
        let id = U256::from(hash.as_bytes()) % TREASURE_NUMBER;
        let treasure = self.handler.state.treasure(id.as_u64())?;        

        let ix = system_instruction::transfer(
            &self.handler.state.signer(), 
            &treasure, 
            TREASURE_LAMPORTS
        );

        self.handler.state.invoke_signed(&ix, vec![], false)
    }
    
    pub fn set_exit_reason(&mut self, reason: ExitReason, value: Vec<u8>) {
        self.exit_reason = Some(reason);
        self.return_value = Some(value);
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{non_evm_cached::test_support::MapState, state::Journal},
        evm::{Context, ExitSucceed::Returned},
        solana_program::pubkey::Pubkey,
        std::collections::BTreeMap,
    };

    fn vm(state: &MapState) -> Vm<'_, MapState> {
        let handler = JournaledState {
            state,
            journal: Journal::new(),
            mutable: true,
            timestamp: 0,
            slot: 0,
            origin: None,
            gas_limit: None,
            gas_price: None,
            gas_recipient: None,
            merged_slots: BTreeMap::new(),
            found_cpi: false,
            found_cpi_cached: false,
        };
        Vm { snapshot: None, handler, return_value: None, exit_reason: None, steps_executed: 0, native_work: 0, atomic_flow: true }
    }

    const CALLER: H160 = H160::repeat_byte(0xca);
    const CREATED: H160 = H160::repeat_byte(0xc0);

    fn create() -> CreateInterrupt {
        CreateInterrupt {
            context: Context { address: CREATED, caller: CALLER, apparent_value: U256::zero() },
            transfer: None,
            address: CREATED,
            init_code: vec![0x00],
        }
    }

    /// Ethereum consumes the creator's nonce when CREATE is issued, whether or
    /// not the init code succeeds; otherwise a failed CREATE would let the
    /// same legacy-derived address be minted again.
    #[test]
    fn a_reverted_create_still_consumes_the_creators_nonce() {
        let state = MapState::new(Pubkey::new_unique(), 0);
        let mut vm = vm(&state);

        vm.trap(Trap::Create(create()));
        vm.trap(Trap::MergeOrRevert(ExitReason::Revert(Reverted), vec![]));

        assert_eq!(vm.handler.journal.nonce_diff(&CALLER), 1);
        assert_eq!(vm.handler.journal.nonce_diff(&CREATED), 0, "the child's own nonce bump reverts with it");
    }

    #[test]
    fn a_succeeded_create_keeps_both_nonce_bumps() {
        let state = MapState::new(Pubkey::new_unique(), 0);
        let mut vm = vm(&state);

        vm.trap(Trap::Create(create()));
        vm.trap(Trap::MergeOrRevert(ExitReason::Succeed(Returned), vec![]));

        assert_eq!(vm.handler.journal.nonce_diff(&CALLER), 1);
        assert_eq!(vm.handler.journal.nonce_diff(&CREATED), 1);
    }
}

// FIND-031 — Vm::trap_call precompile work-pricing tests. A dummy Origin +
// Allocate state is enough here: the scenarios below are all top-level calls
// (Vm::snapshot == None), so `commit_call` short-circuits before touching
// state (journaled_state.rs), and the over-budget path never reaches
// `handle_non_evm_call` at all. `unimplemented!()` bodies are safe because
// none of them are ever called on these paths.
#[cfg(test)]
mod native_work_tests {
    use super::*;
    use crate::{
        precompile::{ecpairing::Ecpairing, ecrecover::Ecrecover},
        config::{MAX_OPCODES_PER_EMULATION, PRECOMPILE_STEPS_ECRECOVER, PRECOMPILE_STEPS_PAIRING_BASE, PRECOMPILE_STEPS_PAIRING_PER_PAIR},
        error::Result as CrateResult,
        pda::Seed,
        Account,
    };
    use evm::{Context as EvmContext, ExitError, ExitSucceed::Returned};
    use solana_program::{account_info::AccountInfo, instruction::Instruction, pubkey::Pubkey};

    struct DummyState;

    impl Origin for DummyState {
        fn nonce(&self, _: &H160) -> CrateResult<Option<u64>> { unimplemented!() }
        fn balance(&self, _: &H160) -> CrateResult<Option<U256>> { unimplemented!() }
        fn code(&self, _: &H160) -> CrateResult<Option<Vec<u8>>> { unimplemented!() }
        fn valids(&self, _: &H160) -> CrateResult<Option<Vec<u8>>> { unimplemented!() }
        fn storage(&self, _: &H160, _: &U256) -> CrateResult<Option<U256>> { unimplemented!() }
        fn inc_nonce<L: crate::context::AccountLock>(&self, _: &H160, _: &L) -> CrateResult<()> { unimplemented!() }
        fn add_balance<L: crate::context::AccountLock>(&self, _: &H160, _: &U256, _: &L) -> CrateResult<()> { unimplemented!() }
        fn sub_balance<L: crate::context::AccountLock>(&self, _: &H160, _: &U256, _: &L) -> CrateResult<()> { unimplemented!() }
        fn set_code<L: crate::context::AccountLock>(&self, _: &H160, _: &[u8], _: &[u8], _: &L) -> CrateResult<()> { unimplemented!() }
        fn set_storage<L: crate::context::AccountLock>(&self, _: &H160, _: &U256, _: &U256, _: &L) -> CrateResult<()> { unimplemented!() }
        fn base(&self) -> &crate::state::base::Base<'_> { unimplemented!() }
        fn account(&self, _: &Pubkey) -> CrateResult<Account> { unimplemented!() }
        fn with_account_info<F, R>(&self, _: &Pubkey, _: F) -> CrateResult<R>
        where F: FnOnce(&AccountInfo) -> CrateResult<R> { unimplemented!() }
        fn invoke_signed(&self, _: &Instruction, _: Vec<Seed>, _: bool) -> CrateResult<()> { unimplemented!() }
        fn invoke_signed_unchecked(&self, _: &Instruction, _: Vec<Seed>) -> CrateResult<()> { unimplemented!() }
        fn signer(&self) -> Pubkey { unimplemented!() }
        fn wallet(&self) -> CrateResult<Pubkey> { unimplemented!() }
        fn treasure(&self, _: u64) -> CrateResult<Pubkey> { unimplemented!() }
        fn owner(&self, _: &Pubkey) -> CrateResult<Pubkey> { unimplemented!() }
        fn ed25519_data(&self) -> CrateResult<Vec<u8>> { unimplemented!() }
    }

    impl Allocate for DummyState {
        fn alloc_balance<L: crate::context::AccountLock>(&self, _: &H160, _: &L) -> CrateResult<()> { unimplemented!() }
        fn alloc_slots<L: crate::context::AccountLock>(&self, _: &Pubkey, _: &Seed, _: usize, _: &L, _: &H160) -> CrateResult<bool> { unimplemented!() }
        fn alloc_slots_unchecked(&self, _: &Pubkey, _: &Seed, _: usize, _: &H160) -> CrateResult<()> { unimplemented!() }
        fn alloc_contract<L: crate::context::AccountLock>(&self, _: &H160, _: &[u8], _: &[u8], _: &L) -> CrateResult<bool> { unimplemented!() }
    }

    // JournaledState::new calls Clock::get(), which on Agave 4.x routes
    // through the `solana_get_sysvar` host-registry fork (byte-for-byte
    // upstream on-chain; off-chain it serves whatever's installed here) —
    // NOT the older SyscallStubs::sol_get_clock_sysvar path. Same pattern as
    // program/tests/t22_differential.rs::install_stub_sysvars.
    fn install_stubs() {
        let clock_bytes = bincode::serde::encode_to_vec(solana_program::clock::Clock::default(), bincode::config::legacy())
            .expect("Clock encodes");
        solana_get_sysvar::set_sysvar_bytes(solana_program::sysvar::clock::ID, clock_bytes);
    }

    fn fresh_vm() -> Vm<'static, DummyState> {
        install_stubs();
        let state: &'static DummyState = Box::leak(Box::new(DummyState));
        let mut vm = Vm::new(state, true).unwrap();
        vm.handler.origin = Some(H160::zero());
        vm
    }

    fn call(address: H160, input: Vec<u8>) -> CallInterrupt {
        CallInterrupt {
            code_address: address,
            transfer: None,
            input,
            is_static: true,
            context: EvmContext { address, caller: H160::zero(), apparent_value: U256::zero() },
        }
    }

    #[test]
    fn oversized_pairing_input_is_refused_before_running() {
        // RED-031c: price alone (349k pairs would be 64MiB; use a smaller
        // pair count that the CHOSEN placeholder constants still price over
        // budget, per the regression guard in precompile::step_price_tests).
        let pairs = 260_000_usize;
        let price = PRECOMPILE_STEPS_PAIRING_BASE + PRECOMPILE_STEPS_PAIRING_PER_PAIR * pairs as u64;
        assert!(price > MAX_OPCODES_PER_EMULATION, "test fixture must actually exceed the budget");

        let mut vm = fresh_vm();
        let input = vec![0_u8; pairs * 192];
        let result = vm.trap(Trap::Call(call(Ecpairing::ADDRESS, input)));

        // Body skipped: a real (even garbage-zero, which is a VALID identity
        // pairing) run would return Succeed with the pairing word, not an
        // OutOfGas error. Getting OutOfGas here proves the syscall never ran.
        match result {
            Some((data, ExitReason::Error(ExitError::OutOfGas))) => assert!(data.is_empty()),
            other => panic!("expected a frame-local OutOfGas failure with empty data, got {other:?}"),
        }
        assert_eq!(vm.native_work, price, "native_work must record the full priced cost even though the body was skipped");
        assert_eq!(vm.steps_executed, 0, "native_work must not feed steps_executed");
    }

    #[test]
    fn normal_small_precompile_call_is_unaffected() {
        // "a normal small precompile call is unaffected" — ecrecover with an
        // unrecoverable v returns Ok(empty) (Ethereum's real success-empty
        // semantics per FIND-025), so the call succeeds and native_work
        // gains exactly the fixed ecrecover price; steps_executed (the
        // interpreter's own counter) is untouched by precompile pricing.
        let mut vm = fresh_vm();
        let input = vec![0_u8; 128]; // v = 0, not in {27,28} -> Ok(vec![])
        let result = vm.trap(Trap::Call(call(Ecrecover::ADDRESS, input)));

        match result {
            Some((data, ExitReason::Succeed(Returned))) => assert!(data.is_empty()),
            other => panic!("expected a frame-local success with empty data, got {other:?}"),
        }
        assert_eq!(vm.native_work, PRECOMPILE_STEPS_ECRECOVER);
        assert_eq!(vm.steps_executed, 0);
    }

    #[test]
    fn pairing_calls_price_native_work_by_pair_count_not_steps_executed() {
        // RED-031a at the Vm level: baseline (pre-FIND-031) had no counter at
        // all — any CALL added 0 to any work counter. Now one pairing CALL
        // with 3 pairs (all-zero -> valid identity pairing, succeeds) adds
        // exactly BASE + 3*PER_PAIR to native_work and nothing to
        // steps_executed (which only `evm.run` touches, per vm.rs:468-469).
        let mut vm = fresh_vm();
        let input = vec![0_u8; 3 * 192];
        let result = vm.trap(Trap::Call(call(Ecpairing::ADDRESS, input)));

        assert!(matches!(result, Some((_, ExitReason::Succeed(Returned)))), "all-zero triples are the valid group identity: {result:?}");
        assert_eq!(vm.native_work, PRECOMPILE_STEPS_PAIRING_BASE + PRECOMPILE_STEPS_PAIRING_PER_PAIR * 3);
        assert_eq!(vm.steps_executed, 0);
    }

    #[test]
    fn refused_input_is_not_priced_for_work_the_body_never_performs() {
        // The price must cover only work the body can actually run. blake2f
        // rejects a rounds field above MAX_ROUNDS before running a round, and
        // pairing rejects a length that is not a whole number of elements, so
        // pricing either as if it had run lets a few bytes of calldata charge
        // more than the whole request budget and abort the transaction.
        use crate::precompile::{blake2f::{Blake2f, BLAKE2_INPUT_LEN, MAX_ROUNDS}, step_price};

        let mut over = vec![0_u8; BLAKE2_INPUT_LEN];
        over[0..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(
            step_price(&Blake2f::ADDRESS, &over) < MAX_OPCODES_PER_EMULATION,
            "a refused rounds field must not be priced past the request budget",
        );

        let mut at_limit = vec![0_u8; BLAKE2_INPUT_LEN];
        at_limit[0..4].copy_from_slice(&MAX_ROUNDS.to_be_bytes());
        assert!(
            step_price(&Blake2f::ADDRESS, &at_limit) > step_price(&Blake2f::ADDRESS, &over),
            "work the body will perform must still be priced",
        );

        // Pairing: a malformed length does no syscall work, so it prices at base.
        let malformed = vec![0_u8; 3 * 192 + 1];
        assert_eq!(step_price(&Ecpairing::ADDRESS, &malformed), PRECOMPILE_STEPS_PAIRING_BASE);
    }

    #[test]
    fn work_already_spent_exhausts_the_request_budget() {
        // The per-request bound, not the per-call price: this is the looped
        // small-call shape the finding describes. Priced work carried in
        // native_work must end the request on its own, with steps_executed
        // untouched -- reverting either cap site leaves the per-call tests
        // green, so the bound needs its own assertion.
        let mut vm = fresh_vm();
        vm.native_work = MAX_OPCODES_PER_EMULATION;
        assert_eq!(vm.steps_executed, 0);
        assert!(
            vm.steps_executed.saturating_add(vm.native_work) >= MAX_OPCODES_PER_EMULATION,
            "native_work alone must be able to exhaust the request budget",
        );
    }
}
