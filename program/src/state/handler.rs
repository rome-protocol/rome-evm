use {
    super::JournaledState,
    crate::{
        origin::Origin, state::{Allocate, Diff},
    },
    evm::{
        Capture, Context, CreateScheme, ExitError, ExitReason, Handler, Machine, Opcode, Stack,
        Transfer, H160, H256, U256, ExitFatal,
    },
    solana_program::keccak::hash,
    std::{
        convert::Infallible, ops::Sub,
    },
};

pub struct CallInterrupt {
    pub code_address: H160,
    pub transfer: Option<Transfer>,
    pub input: Vec<u8>,
    pub is_static: bool,
    pub context: Context,
}

impl CallInterrupt {
    // false under DELEGATECALL/CALLCODE: the running code is not the dispatched precompile
    pub fn owner_authenticated(&self) -> bool {
        self.context.address == self.code_address
    }
}

pub struct CreateInterrupt {
    pub context: Context,
    pub transfer: Option<Transfer>,
    pub address: H160,
    pub init_code: Vec<u8>,
}

impl<T: Origin + Allocate> Handler for JournaledState<'_, T> {
    type CreateInterrupt = CreateInterrupt;
    type CreateFeedback = Infallible;
    type CallInterrupt = CallInterrupt;
    type CallFeedback = Infallible;

    fn keccak256_h256(&self, data: &[u8]) -> H256 {
        let hash = hash(data);
        H256::from(hash.to_bytes())
    }

    fn nonce(&self, address: H160) -> U256 {
        let diff = self.journal.nonce_diff(&address);
        let nonce = self.state.nonce(&address)
            .expect("error to get nonce")
            .unwrap_or(0);

        (nonce + diff).into()
    }

    // TODO: return Result<(U256, ExitFatal)>
    fn balance(&self, address: H160) -> U256 {
        let debet = self.journal.transfer_from(&address)
            .expect(&format!("Calculation overflow {}", &address));
        
        let credit = self.journal.transfer_to(&address)
        .expect(&format!("Calculation overflow {}", &address));

        let mut base = self.state.balance(&address)
            .expect("error to get balance")
            .unwrap_or(U256::zero());

        base = base.checked_add(credit)
            .expect(&format!("Calculation overflow {}", &address));
        
        base = base.checked_sub(debet)
            .expect(&format!("Calculation underflow {}", &address));
        
        base
    }

    fn code_size(&self, address: H160) -> U256 {
        if let Some(program) = self.non_evm_program(&address) {
            if program.precompile() {
                U256::zero()
            } else {
                U256::one()
            }
        } else {
            if let Some((code, _)) = self.journal.code_valids_diff(&address) {
                return code.len().into();
            }
            // TODO: add code_size() to Origin trait
            self.state.code(&address)
                .expect("error to get code_size")
                .map_or(0, |vec| vec.len()).into()
        }
    }

    fn code_hash(&self, address: H160) -> H256 {
        let code = self.code(address);
        let hash = hash(&code);
        H256::from(hash.to_bytes())
    }

    fn code(&self, address: H160) -> Vec<u8> {
        if let Some((code, _)) = self.journal.code_valids_diff(&address) {
            return code.clone();
        }

        self.state.code(&address)
            .expect("error to get code")
            .unwrap_or(vec![])
    }

    fn valids(&self, address: H160) -> Vec<u8> {
        if let Some((_, valids)) = self.journal.code_valids_diff(&address) {
            return valids.clone();
        }

        self.state.valids(&address)
            .expect("error to get code")
            .unwrap_or(vec![])
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        if let Some(value) = self.journal.storage_diff(&address, &index) {
            return value;
        }
        self.state
            .storage(&address, &index)
            .expect("error to get storage")
            .unwrap_or(U256::zero())
    }

    fn gas_left(&self) -> U256 {
        if self.gas_recipient.is_some() {
            return self.gas_limit.unwrap()
        }

        U256::max_value()
    }

    fn gas_price(&self) -> U256 {
        if self.gas_recipient.is_some() {
            return self.gas_price.unwrap()
        }

        U256::zero()
    }

    fn origin(&self) -> H160 {
        self.origin
            .expect("journal_state.origin expected")
    }

    fn block_hash(&self, number: U256) -> H256 {
        let block = self.block_number();

        if number >= block {
            return H256::zero()
        }

        if block.sub(number) > 256_u64.into() {
            return H256::zero()
        }

        let mut be = [0; 32];
        number.to_big_endian(&mut be);

        self.keccak256_h256(&mut be)
    }

    fn block_number(&self) -> U256 {
        self.slot.into()
    }

    fn block_coinbase(&self) -> H160 {
        if let Some(recipient) = self.gas_recipient {
            return recipient
        }

        H160::default()
    }

    fn block_timestamp(&self) -> U256 {
        self.timestamp.into()
    }

    fn block_difficulty(&self) -> U256 {
        U256::zero()
    }
    fn block_gas_limit(&self) -> U256 {
        U256::max_value()
    }

    fn chain_id(&self) -> U256 {
        U256::from(self.state.base().owner_info().chain)
    }

    fn set_storage(&mut self, address: H160, index: U256, value: U256) -> Result<(), ExitError> {
        if !self.mutable {
            return Err(ExitError::StaticModeViolation);
        }

        self.journal
            .get_mut(&address)
            .push(Diff::StorageChange { key: index, value });
        Ok(())
    }

    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) -> Result<(), ExitError> {
        if !self.mutable {
            return Err(ExitError::StaticModeViolation);
        }

        self.journal
            .get_mut(&address)
            .push(Diff::Event { topics, data });
        Ok(())
    }

    // TODO: create correct test with non-zero transfer
    fn mark_delete(&mut self, address: H160, target: H160) -> Result<(), ExitError> {
        if !self.mutable {
            return Err(ExitError::StaticModeViolation);
        }

        let value = self.balance(address);
        self.transfer(&address, &target, &value);

        if !self.code_size(address).is_zero() {
            if self.non_evm_program(&address).is_none() {
                let onchain_code_size = self
                    .state
                    .code(&address)
                    .expect("error to get code to selfdestruct")
                    .map_or(0, |vec| vec.len());

                if onchain_code_size == 0 {
                    self.journal.selfdestruct(&address)
                }
            }
        }

        Ok(())
    }

    fn create(
        &mut self,
        caller: H160,
        scheme: CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        _target_gas: Option<u64>,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Self::CreateInterrupt> {
        if !self.mutable {
            return Capture::Exit((
                ExitReason::Error(ExitError::StaticModeViolation),
                None,
                vec![],
            ));
        }
        if !value.is_zero() && self.balance(caller) < value {
            return Capture::Exit((ExitReason::Error(ExitError::OutOfFund), None, vec![]));
        }
        let new_addr = self.build_address(scheme);

        if new_addr.is_err() {
            let res = (ExitReason::Error(ExitError::CreateCollision), None,  vec![]);
            return Capture::Exit(res);
        }
        let new_addr = new_addr.unwrap();

        let context = Context {
            address: new_addr,
            caller,
            apparent_value: value,
        };

        let transfer = if value.is_zero() {
            None
        } else {
            Some(Transfer {
                source: caller,
                target: new_addr,
                value,
            })
        };

        let create = CreateInterrupt {
            context,
            transfer,
            address: new_addr,
            init_code,
        };

        Capture::Trap(create)
    }

    fn call(
        &mut self,
        code_address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        _: Option<u64>,
        is_static: bool,
        ctx: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Self::CallInterrupt> {

        let static_call = !self.mutable || is_static;

        if let Some(t) = transfer.as_ref() {
            if !t.value.is_zero() && static_call {
                return Capture::Exit((ExitReason::Error(ExitError::StaticModeViolation), vec![]));
            }
            if self.balance(t.source) < t.value {
                return Capture::Exit((ExitReason::Error(ExitError::OutOfFund), vec![]));
            }
        }

        let call = CallInterrupt {
            code_address,
            transfer,
            input,
            is_static: static_call,
            context: ctx,
        };

        Capture::Trap(call)
    }

    fn pre_validate(
        &mut self,
        _context: &Context,
        _opcode: Opcode,
        _stack: &Stack,
    ) -> Result<(), ExitError> {
        Ok(())
    }

    fn call_feedback(&mut self, _feedback: Self::CallFeedback) -> Result<(), ExitError> {
        Ok(())
    }

    /// Handle other unknown external opcodes.
    fn other(&mut self, opcode: Opcode, _stack: &mut Machine) -> Result<(), ExitFatal> {
        Err(ExitFatal::IncompatibleVersionEVM(opcode.0))
    }

    fn transient_storage(&self, address: H160, index: U256) -> U256 {
        if let Some(value) = self.journal.t_storage_diff(&address, &index) {
            return value;
        }

        U256::zero()
    }

    fn set_transient_storage(
        &mut self,
        address: H160,
        index: U256,
        value: U256,
    ) -> Result<(), ExitError> {
        if !self.mutable {
            return Err(ExitError::StaticModeViolation);
        }

        self.journal
            .get_mut(&address)
            .push(Diff::TStorageChange { key: index, value });
        Ok(())
    }
}
