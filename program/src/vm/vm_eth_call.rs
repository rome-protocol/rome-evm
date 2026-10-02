use {
    crate::{
        vm::{Vm, Execute}, error::Result, origin::Origin, state::Allocate,
        tx::{
            tx::Tx, legacy::Legacy,
        },
    },
    crate::dmsg,
};

pub enum MachineEthCall {
    Init,
    Execute,
    Exit,
}
use MachineEthCall::*;
use crate::error::RomeProgramError::OpcodeLimitExceeded;
use crate::MAX_OPCODES_PER_EMULATION;

pub struct VmCall<'a, T: Origin + Allocate> {
    pub vm: Vm<'a, T>,
    state_machine: Option<MachineEthCall>,
    tx: Tx,
}

impl<'a, T: Origin + Allocate> VmCall<'a, T> {
    #[allow(dead_code)]
    pub fn new(state: &'a T, legacy: Legacy, ) -> Result<Box<Self>> {
        let vm_call = Self {
            vm: Vm::new(state, true)?,
            state_machine: None,
            tx: Tx::from_legacy(legacy),
        };

        Ok(Box::new(vm_call))
    }
}

impl<T: Origin + Allocate> Execute<MachineEthCall> for VmCall<'_, T> {
    fn advance(&mut self) -> Result<()> {
        let state_machine = self.state_machine.take().expect("vm state machine fault");

        let state_machine = match state_machine {
            Init => {
                dmsg!("Init");
                if let Some((value, reason)) = self.vm.init(&mut self.tx, false, None)? {
                    self.vm.set_exit_reason(reason, value);
                    Exit
                } else {
                    Execute
                }
            }
            Execute => {
                dmsg!("Execute");
                // Precompile work (self.vm.native_work) is bounded
                // together with interpreter steps against the same budget.
                let spent = self.vm.steps_executed.saturating_add(self.vm.native_work);
                let left = MAX_OPCODES_PER_EMULATION.saturating_sub(spent);
                if let Some((return_value, reason)) = self.vm.execute(left) {
                    self.vm.set_exit_reason(reason, return_value);
                    Exit
                } else  if self.vm.steps_executed.saturating_add(self.vm.native_work) >= MAX_OPCODES_PER_EMULATION {
                    return Err(OpcodeLimitExceeded)
                } else {
                    Execute
                }
            }
            Exit => unreachable!(),
        };

        self.state_machine = Some(state_machine);
        Ok(())
    }

    fn consume(&mut self, machine: MachineEthCall) -> Result<()> {
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

