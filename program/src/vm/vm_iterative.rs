use {
    super::{vm::Vm, Execute},
    crate::{
        accounts::Iterations,
        config::SIG_VERIFY_COST,
        context::{AccountLock, Context},
        error::{Result, RomeProgramError::*},
        origin::Origin,
        state::Allocate, msg, dmsg,
    },
    evm::{Handler, U256,},
};

pub enum MachineIt {
    FromStateHolder,
    Lock,
    Init,
    InitLocked,
    Execute,
    IntoTrap,
    Serialize(Box<Self>),
    AllocateHolder(Box<Self>),
    Allocate,
    MergeSlots,
    AllocateStorage,
    Unlock,
    UnlockFailedTx,
    NextIteration(Box<Self>),
    NextIterationUnchecked(Box<Self>),
    Completed,
    Failed,
    Commit,
    Exit,
}

use MachineIt::*;
use crate::TREASURE_LAMPORTS;

impl From<Iterations> for MachineIt {
    fn from(iter: Iterations) -> Self {
        match iter {
            Iterations::Lock => Lock,
            Iterations::Start => Init,
            Iterations::Execute => Execute,
            Iterations::Allocate => Allocate,
            Iterations::MergeSlots => MergeSlots,
            Iterations::AllocateStorage => AllocateStorage,
            Iterations::Commit => Commit,
            Iterations::Footprint => panic!("footprint iteration is not supported"),
            Iterations::Unlock => Unlock,
            Iterations::UnlockFailedTx => UnlockFailedTx,
            Iterations::Completed => Completed,
            Iterations::Failed => Failed,
        }
    }
}
impl From<&MachineIt> for Iterations {
    fn from(machine: &MachineIt) -> Self {
        match machine {
            Lock => Iterations::Lock,
            Init => Iterations::Start,
            Execute => Iterations::Execute,
            Allocate => Iterations::Allocate,
            MergeSlots => Iterations::MergeSlots,
            AllocateStorage => Iterations::AllocateStorage,
            Commit => Iterations::Commit,
            Unlock => Iterations::Unlock,
            UnlockFailedTx => Iterations::UnlockFailedTx,
            Completed => Iterations::Completed,
            Failed => Iterations::Failed,
            _ => panic!("VmFault: MachineIterativeative to Iterations cast error"),
        }
    }
}

pub struct VmIt<'a, T: Origin + Allocate, L: AccountLock + Context> {
    pub vm: Vm<'a, T>,
    pub state_machine: Option<MachineIt>,
    pub context: &'a L,
    pub limit: u64,
}

impl<'a, T: Origin + Allocate, L: AccountLock + Context> VmIt<'a, T, L> {
    pub fn new(state: &'a T, context: &'a L, limit: u64) -> Result<Box<Self>> {
        let vm_it = Self {
            vm: Vm::new(state, false)?,
            state_machine: None,
            context,
            limit,
        };

        Ok(Box::new(vm_it))
    }

    pub fn verify_balance_and_gas(&self) -> Result<()> {
        if self.vm.handler.gas_recipient.is_some() {
            let gas_limit = self.vm.handler.gas_limit.unwrap();
            let gas_price = self.vm.handler.gas_price.unwrap();
            let from = self.vm.handler.origin.unwrap();

            let wei = gas_limit.checked_mul(gas_price).ok_or(CalculationOverflow)?;
            if self.vm.handler.balance(from) < wei {
                return Err(InsufficientFunds(from, wei))
            }
        }
        // TODO check signer.balance >= TREASURE_LAMPORTS

        Ok(())
    }

    pub fn verify_balance(&self, fee: u64, refund: u64) -> Result<()> {
        if self.vm.handler.gas_recipient.is_some() {
            let from = self.vm.handler.origin.unwrap();
            let gas_limit = self.vm.handler.gas_limit.unwrap();
            let gas_price = self.vm.handler.gas_price.unwrap();

            let lamports: U256 = fee.saturating_sub(refund).into();

            if lamports > gas_limit {
                return Err(InsufficientGas(gas_limit, lamports))
            }

            let wei = lamports.checked_mul(gas_price).ok_or(CalculationOverflow)?;

            if self.vm.handler.balance(from) < wei {
                return Err(InsufficientFunds(from, wei))
            }
        }

        Ok(())
    }

    pub fn collect_fees(&self) -> Result<()> {
        let (fee, refund) = self.vm.handler.state.base().get_fees();
        self.vm.handler.state.base().reset_fees();
        self.context.collect_fees(fee, refund)
    }

    fn calc_fee(&self, to: &MachineIt) -> Result<u64> {
        let pri_fee = self.context.priority_fee()?;
        iterative_fee(to, pri_fee)
    }
}

/// Pure per-iteration fee-reservation math, factored out of `calc_fee` so
/// it's unit-testable without a live `VmIt` — `Vm::new` constructs a
/// `JournaledState` that reads the Solana `Clock` sysvar via `Clock::get()`,
/// which returns `Err(UnsupportedSysvar)` on a host `cargo test` outside a
/// Mollusk/on-chain runtime.
pub fn iterative_fee(to: &MachineIt, pri_fee: u64) -> Result<u64> {
    match *to {
        // fee for current_iteration + Commit + Unlock + TREASURE + pri_fee
        Commit => {
            SIG_VERIFY_COST
                .checked_add(pri_fee)
                .ok_or(CalculationOverflow)?
                .checked_mul(3)
                .ok_or(CalculationOverflow)?
                .checked_add(TREASURE_LAMPORTS)
                .ok_or(CalculationOverflow)
        },
        _ => SIG_VERIFY_COST
            .checked_add(pri_fee)
            .ok_or(CalculationOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{Data, StateHolder};
    use evm::H256;
    use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

    /// #502: `pri_fee` is carried from `DoTxIterative` ix data into the
    /// `StateHolder` at `new_session` (see `context/iterative.rs`), not
    /// recomputed from a ComputeBudget sibling-walk on each iteration.
    /// Pins the round-trip: a StateHolder seeded with a carried pri_fee
    /// feeds the same Commit-arm formula `calc_fee` uses.
    #[test]
    fn iterative_commit_arm_fee_uses_carried_priority() {
        let key = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mut lamports = 0u64;
        let mut data = vec![0u8; 256];
        let info = AccountInfo::new(
            &key, false, true, &mut lamports, &mut data, &owner, false,
        );

        StateHolder::init(&info).unwrap();
        StateHolder::set_session(&info, H256::default(), 1, 777).unwrap();

        let pri_fee = StateHolder::from_account(&info).unwrap().pri_fee;
        let fee = iterative_fee(&Commit, pri_fee).unwrap();

        assert_eq!(fee, (SIG_VERIFY_COST + 777) * 3 + TREASURE_LAMPORTS);
    }
}


impl<T: Origin + Allocate, L: AccountLock + Context> Execute<MachineIt> for VmIt<'_, T, L> {
    fn advance(&mut self) -> Result<()> {
        let state_machine = self
            .state_machine
            .take()
            .unwrap_or_else(|| panic!("vm state machine fault"));

        let state_machine = match state_machine {
            FromStateHolder => {
                dmsg!("FromStateHolder");
                // state_holder stores tx_hash and session_id
                if self.context.has_session()? {
                    resume_or_release(self.context.get_iteration()?, self.context.iter_cnt()?)
                } else {
                    self.context.new_session()?;

                    if self.context.state_holder_len()? == 0 {
                        AllocateHolder(Box::new(Lock))
                    } else {
                        Lock    //start execution from the very beginning
                    }
                }
            }
            Lock => {
                dmsg!("Lock");
                self.context.lock()?;
                InitLocked
            }
            Init => {
                dmsg!("Init");
                self.context.update_lock()?;
                InitLocked
            }
            InitLocked => {
                dmsg!("InitLocked");
                let mut tx = self.context.tx()?;
                let fee_addr = self.context.fee_recipient();
                let check_nonce = !self.context.is_gas_estimate();

                let state =  if let Some((value, reason)) = self.vm.init(&mut tx, check_nonce, fee_addr)? {
                    self.vm.set_exit_reason(reason, value);
                    if reason.is_succeed() {
                        Commit
                    } else {
                        UnlockFailedTx // skip Commit
                    }
                } else {
                    Execute
                };

                self.verify_balance_and_gas()?;
                Serialize(Box::new(state))
            }
            Serialize(to) => {
                dmsg!("Serialize");
                match self.context.serialize(&self.vm) {
                    Err(IoError(io)) => {
                        // not enough space
                        match io.kind() {
                            // holder.data is invalid, it cannot be used. The state is lost.
                            // We need to start from the beginning
                            std::io::ErrorKind::WriteZero => AllocateHolder(Box::new(Init)),
                            _ => return Err(IoError(io)),
                        }
                    }
                    Err(e) => return Err(e),
                    Ok(()) => NextIteration(to),
                }
            }
            AllocateHolder(to) => {
                dmsg!("AllocateHolder");
                self.context.allocate_holder()?;
                NextIteration(to)
            }
            Execute => {
                dmsg!("Execute");
                self.context.deserialize(&mut self.vm)?;
                self.context.update_lock()?;
                IntoTrap
            }
            IntoTrap => {
                dmsg!("IntoTrap");
                let steps_left = self.limit.saturating_sub(self.vm.steps_executed);

                if let Some((return_value, reason)) = self.vm.execute(steps_left) {
                    self.vm.set_exit_reason(reason, return_value);
                    let next_step = if reason.is_succeed() {
                        Allocate
                    } else {
                        UnlockFailedTx // skip Commit
                    };
                    Serialize(Box::new(next_step))

                } else if self.limit.saturating_sub(self.vm.steps_executed) > 0 {
                    IntoTrap
                } else {
                    Serialize(Box::new(Execute))
                }
            }
            Allocate => {
                dmsg!("Allocate");
                self.context.deserialize(&mut self.vm)?;
                self.context.update_lock()?;

                if self.vm.handler.allocate(self.context)? {
                    if self.vm.handler.journal.found_storage() {
                        Serialize(Box::new(MergeSlots))
                    } else {
                        // skip merge slots, allocate slots
                        Serialize(Box::new(Commit))
                    }
                } else {
                    Serialize(Box::new(Allocate))
                }
            }
            MergeSlots => {
                dmsg!("MergeSlots");
                self.context.deserialize(&mut self.vm)?;
                self.context.update_lock()?;
                self.vm.handler.merge_slots()?;
                Serialize(Box::new(AllocateStorage))
            }
            AllocateStorage => {
                dmsg!("AllocateStorage");
                self.context.deserialize(&mut self.vm)?;
                self.context.update_lock()?;

                if self.vm.handler.alloc_slots(self.context)? {
                    Serialize(Box::new(Commit))
                } else {
                    Serialize(Box::new(AllocateStorage))
                }
            }
            Commit => {
                msg!("Commit");
                self.context.deserialize(&mut self.vm)?;
                self.context.update_lock()?;
                self.vm.handler.commit(self.context)?;
                self.vm.handler.revert_all();

                self.collect_fees()?;
                let (fee, refund) = self.context.fees()?;

                #[cfg(not(target_os = "solana"))]
                let alloc_payed = self.vm.handler.state.base().alloc_payed();

                // fee_recipient account will be created at the operator's expense.
                // otherwise it is necessary to include this cost in gas_estimate for each tx.
                self.vm.gas_transfer(fee, refund)?;
                self.vm.treasure_transfer(self.context.tx_hash())?;
                self.vm.handler.commit(self.context)?;

                #[cfg(not(target_os = "solana"))]
                self.vm.handler.state.base().set_alloc_payed(alloc_payed);

                self.vm.log_exit_reason()?;
                NextIterationUnchecked(Box::new(Unlock))
            }
            Unlock => {
                dmsg!("Unlock");
                self.context.deserialize(&mut self.vm)?;
                self.context.unlock()?;
                NextIterationUnchecked(Box::new(Completed))
            }
            Completed => {
                msg!("UnnecessaryIteration: {}", self.context.tx_hash());
                return Err(UnnecessaryIteration(self.context.tx_hash()));
            }
            UnlockFailedTx => {
                msg!("UnlockFailedTx");
                self.context.deserialize(&mut self.vm)?;
                msg!("reason: {:?}", self.vm.exit_reason.unwrap());

                self.context.unlock()?;
                NextIterationUnchecked(Box::new(Failed))
            }
            Failed => {
                msg!("TxFailed: {}", self.context.tx_hash());
                return Err(UnnecessaryIteration(self.context.tx_hash()))
            }
            NextIteration(to) => {
                let fee = self.calc_fee(to.as_ref())?;
                self.vm.handler.state.base().add_fee(fee)?;
                self.collect_fees()?;

                let (fee, refund) = self.context.fees()?;
                self.verify_balance(fee, refund)?;

                NextIterationUnchecked(to)
            }
            NextIterationUnchecked(to) => {
                self.context.set_iteration((&*to).into())?;
                Exit
            }
            Exit => unreachable!(),
        };
        self.state_machine = Some(state_machine);
        Ok(())
    }

    fn consume(&mut self, machine: MachineIt) -> Result<()> {
        self.state_machine = Some(machine);

        loop {
            self.advance()?;
            if let Some(Exit) = self.state_machine.as_ref() {
                break;
            }
        }

        Ok(())
    }
}

/// Where a leg goes when it rejoins an existing session. Past
/// `MAX_SESSION_ITERATIONS` the session stops resuming and is routed to
/// `Unlock`, which releases its locked accounts and marks it completed; a
/// terminal iteration is returned unchanged so the usual
/// `UnnecessaryIteration` reporting still happens.
pub fn resume_or_release(iteration: Iterations, iter_cnt: u64) -> MachineIt {
    let terminal = matches!(
        iteration,
        Iterations::Unlock | Iterations::UnlockFailedTx | Iterations::Completed | Iterations::Failed
    );

    if iter_cnt > crate::config::MAX_SESSION_ITERATIONS && !terminal {
        return Unlock;
    }

    iteration.into()
}

#[cfg(test)]
mod session_iteration_cap_tests {
    use {super::*, crate::config::MAX_SESSION_ITERATIONS};

    /// A session that keeps iterating holds its locked accounts the whole time,
    /// so past the cap it must stop resuming and go release them.
    #[test]
    fn a_session_past_the_cap_is_routed_to_unlock() {
        for iteration in [Iterations::Execute, Iterations::Lock, Iterations::Allocate, Iterations::Commit] {
            let next = resume_or_release(iteration, MAX_SESSION_ITERATIONS + 1);
            assert!(matches!(next, Unlock), "a session past the cap must release its locks");
        }
    }

    /// Everything a real transaction needs stays untouched. The client's own
    /// ceiling is 128 legs, so a session at the cap still resumes normally.
    #[test]
    fn a_session_within_the_cap_resumes_where_it_left_off() {
        for iter_cnt in [0, 1, 128, MAX_SESSION_ITERATIONS] {
            let next = resume_or_release(Iterations::Execute, iter_cnt);
            assert!(matches!(next, Execute), "a session within the cap must resume, iter_cnt {iter_cnt}");
        }
    }

    /// A session already at a terminal step is reported the usual way rather
    /// than being sent to Unlock a second time.
    #[test]
    fn terminal_iterations_are_left_alone_past_the_cap() {
        let cases = [
            (Iterations::Unlock, "Unlock"),
            (Iterations::UnlockFailedTx, "UnlockFailedTx"),
            (Iterations::Completed, "Completed"),
            (Iterations::Failed, "Failed"),
        ];
        for (iteration, name) in cases {
            let past = resume_or_release(iteration.clone(), MAX_SESSION_ITERATIONS + 1_000);
            let within = resume_or_release(iteration, 0);
            assert_eq!(
                std::mem::discriminant(&past),
                std::mem::discriminant(&within),
                "{name} must route the same past the cap as within it"
            );
        }
    }

    /// The cap has to sit above the client's own leg ceiling or real
    /// transactions would be released mid-flight.
    #[test]
    fn the_cap_is_above_the_clients_leg_ceiling() {
        assert!(MAX_SESSION_ITERATIONS > 128, "rome-sdk iterative::MAX_LEGS is 128");
    }
}
