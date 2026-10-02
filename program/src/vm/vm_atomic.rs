use {
    super::{vm::Vm, Execute,},
    crate::{
        context::AccountLock,
        error::{Result, RomeProgramError::{AtomicTxFailed, CalculationOverflow,}},
        origin::Origin,
        state::Allocate,
        config::{SIG_VERIFY_COST, TREASURE_LAMPORTS},
        H160, H256,
        tx::{
            tx::Tx, legacy::Legacy,
        },
        msg, dmsg,
    },
    solana_program::{keccak,},
    evm::ExitReason,
};

pub enum MachineAt {
    Lock,
    Init,
    Execute,
    Commit,
    GasTransfer,
    Error(ExitReason),
    Exit,
}

use MachineAt::*;
use crate::error::RomeProgramError::OpcodeLimitExceeded;
use crate::MAX_OPCODES_PER_EMULATION;

pub struct VmAt<'a, T: Origin + Allocate, L: AccountLock> {
    pub vm: Vm<'a, T>,
    pub state_machine: Option<MachineAt>,
    tx: Tx,
    hash: H256,
    fee_addr: Option<H160>,
    pri_fee: u64,
    context: &'a L,
}

impl<'a, T: Origin + Allocate, L: AccountLock> VmAt<'a, T, L> {
    pub fn new(state: &'a T, rlp: &'a[u8], fee_addr: Option<H160>, pri_fee: u64, context: &'a L) -> Result<Box<Self>> {
        let atomic = Self {
            vm: Vm::new(state, true)?,
            state_machine: None,
            tx: Tx::from_instruction(rlp, state)?,
            hash: H256::from(keccak::hash(rlp).to_bytes()),
            fee_addr,
            pri_fee,
            context,
        };

        Ok(Box::new(atomic))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_unsigned_tx(state: &'a T, legacy: Legacy, hash: H256, context: &'a L, fee_addr: Option<H160>, pri_fee: u64) -> Result<Box<Self>> {
        let atomic = Self {
            vm: Vm::new(state, true)?,
            state_machine: None,
            tx: Tx::from_legacy(legacy),
            hash,
            fee_addr,
            pri_fee,
            context,
        };

        Ok(Box::new(atomic))
    }

    fn calc_fee(&self) -> Result<u64> {
        atomic_fee(self.pri_fee)
    }
}

/// Pure atomic-settlement fee-reservation math, factored out of
/// `VmAt::calc_fee` so it's unit-testable without a live `Vm` — `Vm::new`
/// constructs a `JournaledState` that reads the Solana `Clock` sysvar via
/// `Clock::get()`, which returns `Err(UnsupportedSysvar)` on a host
/// `cargo test` outside a Mollusk/on-chain runtime.
pub fn atomic_fee(pri_fee: u64) -> Result<u64> {
    (SIG_VERIFY_COST + TREASURE_LAMPORTS)
        .checked_add(pri_fee)
        .ok_or(CalculationOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the atomic settlement fee = sig-verify + treasure toll + the
    /// carried (operator-attested) priority fee — #502: pri_fee now comes
    /// straight from ix data (`VmAt.pri_fee`), not a ComputeBudget
    /// sibling-walk.
    #[test]
    fn vm_atomic_calc_fee_uses_carried_priority() {
        let fee = atomic_fee(777).unwrap();
        assert_eq!(fee, SIG_VERIFY_COST + TREASURE_LAMPORTS + 777);
    }
}

impl<T: Origin + Allocate, L: AccountLock> Execute<MachineAt> for VmAt<'_, T, L> {
    fn advance(&mut self) -> Result<()> {
        let state_machine = self
            .state_machine
            .take()
            .unwrap_or_else(|| panic!("vm state machine fault"));

        let state_machine = match state_machine {
            Lock => {
                dmsg!("Lock");
                self.context.lock()?;
                Init
            }
            Init => {
                dmsg!("Init");
                let check_nonce = !self.context.skip_nonce_check();
                if let Some((value, reason)) = self.vm.init(&mut self.tx, check_nonce, self.fee_addr)? {
                    self.vm.set_exit_reason(reason, value);
                    if reason.is_succeed() {
                        Commit
                    } else {
                        if cfg!(target_os = "solana") {
                            Error(reason)
                        } else {
                            Exit
                        }
                    }
                } else {
                    Execute
                }
            }
            Execute => {
                dmsg!("Execute");
                // FIND-031: precompile work (self.vm.native_work) is bounded
                // together with interpreter steps against the same budget.
                // Unreachable in practice on-chain — the CU meter binds
                // first — kept for parity with the off-chain paths.
                let spent = self.vm.steps_executed.saturating_add(self.vm.native_work);
                let left = MAX_OPCODES_PER_EMULATION.saturating_sub(spent);

                if let Some((return_value, reason)) = self.vm.execute(left) {
                    self.vm.set_exit_reason(reason, return_value);
                    if reason.is_succeed() {
                        Commit
                    } else {
                        if cfg!(target_os = "solana") {
                            Error(reason)
                        } else {
                            Exit
                        }
                    }
                } else if self.vm.steps_executed.saturating_add(self.vm.native_work) >= MAX_OPCODES_PER_EMULATION {
                    return Err(OpcodeLimitExceeded)
                } else {
                    Execute
                }
            }
            Commit=> {
                msg!("Commit");
                self.vm.handler.alloc_slots_unchecked()?;
                self.vm.handler.commit(self.context)?;
                self.vm.log_exit_reason()?;
                GasTransfer
            }
            GasTransfer => {
                self.vm.handler.revert_all();
                let fee = self.calc_fee()?;
                self.vm.handler.state.base().add_fee(fee)?;
                let (fee, refund) = self.vm.handler.state.base().get_fees();
                #[cfg(not(target_os = "solana"))]
                let alloc_payed = self.vm.handler.state.base().alloc_payed();

                self.vm.gas_transfer(fee, refund)?;
                self.vm.treasure_transfer(self.hash)?;
                self.vm.handler.commit(self.context)?;

                #[cfg(not(target_os = "solana"))]
                {
                    self.vm.handler.state.base().set_fees(fee, refund)?;
                    self.vm.handler.state.base().set_alloc_payed(alloc_payed);
                }

                Exit
            }
            Error(reason) => {
                msg!("Error");
                return Err(AtomicTxFailed(format!("{:?}", reason)))
            }
            Exit => {
                msg!("Exit");
                Exit
            }
        };
        self.state_machine = Some(state_machine);
        Ok(())
    }

    fn consume(&mut self, machine: MachineAt) -> Result<()> {
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
