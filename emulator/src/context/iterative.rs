use {
    crate::state::State,
    rome_evm::{
        context::{
            iterative::{deserialize_impl, serialize_impl, Request,},
            Context,
        },
        error::Result,
        state::{origin::Origin, Allocate},
        tx::{
            tx::Tx,
        },
        vm::Vm,
        Data, Holder, Iterations, StateHolder, H160, H256,
        api::do_tx_holder::transmit_fee,
        api_offchain::do_call::fee_addr_def,
        GAS_ESTIMATE_HOLDER, GAS_ESTIMATE_SESSION,
    },
    solana_program::{account_info::IntoAccountInfo, msg, pubkey::Pubkey,},
};

pub struct ContextIt<'a, 'b> {
    pub state: &'b State<'a>,
    pub tx_hash: H256,
    pub session: u64,
    pub fee_addr: Option<H160>,
    pub pri_fee: u64,
    pub request: Request<'a>,
    pub tx_holder_key: Option<Pubkey>,
    pub state_holder_key: Pubkey,
    gas_estimate: bool,
}

impl<'a, 'b> ContextIt<'a, 'b> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: &'b State<'a>,
        tx_hash: H256,
        session: u64,
        fee_addr: Option<H160>,
        pri_fee: u64,
        rlp: &'a [u8],
        tx_holder_key: Option<Pubkey>,
        state_holder_key: Pubkey,
    ) -> Result<Self> {
        let bind = state.info_state_holder_by_key(&state_holder_key)?;
        msg!("state_holder data length: {}", bind.1.data.len());

        Ok(Self {
            state,
            tx_hash,
            session,
            fee_addr,
            pri_fee,
            request: Request::Rlp(rlp),
            tx_holder_key,
            state_holder_key,
            gas_estimate: false,
        })
    }

    pub fn new_gas_estimate(state: &'b State<'a>, rlp: &'a[u8], from: H160, hash: H256) -> Result<Self> {
        let holder = GAS_ESTIMATE_HOLDER;  // it should be non-existent resource
        let state_holder = state.info_state_holder(holder, true)?;
        let tx_holder = state.info_tx_holder(holder, true)?;
        Ok(Self {
            state,
            tx_hash: hash,
            session: GAS_ESTIMATE_SESSION, // must not be equal to default value of the StateHolder.session
            fee_addr: fee_addr_def(),
            pri_fee: 0,
            request: Request::UnsignedEip1559(rlp, from),
            tx_holder_key: Some(tx_holder.0),
            state_holder_key: state_holder.0,
            gas_estimate: true,
        })
    }
}

impl<'a, 'b> Context for ContextIt<'a, 'b> {
    fn tx(&self) -> Result<Tx> {
        self.request.to_tx(self.state)
    }
    fn set_iteration(&self, iteration: Iterations) -> Result<()> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();

        StateHolder::set_iteration(&info, iteration)?;
        self.state.update(bind);
        Ok(())
    }
    fn get_iteration(&self) -> Result<Iterations> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();

        StateHolder::get_iteration(&info)
    }
    fn iter_cnt(&self) -> Result<u64> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();

        StateHolder::get_iter_cnt(&info)
    }
    fn serialize<T: Origin + Allocate>(&self, vm: &Vm<T>) -> Result<()> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();

        serialize_impl(&info, vm)?;
        self.state.update(bind);
        Ok(())
    }
    fn deserialize<T: Origin + Allocate>(&self, vm: &mut Vm<T>) -> Result<()> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();

        deserialize_impl(&info, vm)
    }
    fn allocate_holder(&self) -> Result<()> {
        let bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let len = bind.1.data.len() + self.state.alloc_limit();
        self.state.realloc(&bind.0, len, false)?;
        Ok(())
    }

    fn new_session(&self) -> Result<()> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();

        StateHolder::set_session(&info, self.tx_hash, self.session, self.pri_fee)?;
        self.state.update(bind);

        if let Some(key) = self.tx_holder_key {
            let fee = match &self.request {
                Request::Rlp(_) => {
                    let mut bind = self.state.info_tx_holder_by_key(&key)?;
                    let info = bind.into_account_info();
                    transmit_fee(&info)?
                }
                Request::UnsignedEip1559(_, _) => 0,   // transmit_fee will be taken into account on rome-sdk side in gas_estimation call
            };

            self.state.base().add_fee(fee)?;
        }

        Ok(())
    }

    fn has_session(&self) -> Result<bool> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();
        StateHolder::has_session(&info, self.tx_hash, self.session)
    }

    fn tx_hash(&self) -> H256 {
        self.tx_hash
    }

    fn fee_recipient(&self) -> Option<H160> {
        self.fee_addr
    }

    fn state_holder_len(&self) -> Result<usize> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();
        Ok(Holder::size(&info))
    }

    fn fees(&self) -> Result<(u64, u64)> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();
        StateHolder::fees(&info)
    }

    fn collect_fees(&self, lmp_fee: u64, lmp_refund: u64) -> Result<()> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();

        StateHolder::collect_fees(&info, lmp_fee, lmp_refund)?;
        self.state.update(bind);
        Ok(())
    }
    fn is_gas_estimate(&self) -> bool {
        self.gas_estimate
    }
    fn priority_fee(&self) -> Result<u64> {
        let mut bind = self.state.info_state_holder_by_key(&self.state_holder_key)?;
        let info = bind.into_account_info();

        let holder = StateHolder::from_account(&info)?;
        Ok(holder.pri_fee)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_storage::AccountStorage;
    use rome_evm::state::aux::Account as RomeAccount;
    use solana_program::account_info::AccountInfo;
    use std::sync::Arc;

    /// Answers the signer's account and nothing else — just enough for
    /// `State::new_unchecked` to resolve the signer without touching the
    /// chain's real OwnerInfo registration (out of scope for this test).
    struct MockStorage {
        signer: Pubkey,
    }

    impl AccountStorage for MockStorage {
        fn get_account(&self, key: &Pubkey) -> Result<Option<solana_account::Account>> {
            if *key == self.signer {
                Ok(Some(solana_account::Account {
                    lamports: 1_000_000,
                    owner: solana_system_interface::program::ID,
                    ..Default::default()
                }))
            } else {
                Ok(None)
            }
        }
        fn get_multiple_accounts(&self, keys: &[Pubkey]) -> Result<Vec<Option<solana_account::Account>>> {
            keys.iter().map(|k| self.get_account(k)).collect()
        }
    }

    /// #502: `new_session` must persist the `pri_fee` carried on `ContextIt`
    /// (parsed from ix data upstream, via the shared `program::api::split_fee`
    /// / `args` chokepoint), not re-derive it from `Origin::priority_fee` —
    /// that method is deleted. Mirrors the program-side fix
    /// (`program/src/context/iterative.rs::new_session`) into the emulator.
    #[test]
    fn new_session_persists_carried_priority_not_zero() {
        let program_id = Pubkey::new_unique();
        let signer = Pubkey::new_unique();
        let state_holder_key = Pubkey::new_unique();

        let storage: Arc<dyn AccountStorage> = Arc::new(MockStorage { signer });
        let state = State::new_unchecked(&program_id, Some(signer), storage, 1).unwrap();

        // Seed a bare StateHolder PDA owned by the program directly into the
        // account cache, bypassing the client entirely — `new_session` reads
        // it back through `state.info_state_holder_by_key`.
        let mut lamports = 1_000_000u64;
        let mut data = vec![0u8; 256];
        {
            let info = AccountInfo::new(
                &state_holder_key, false, true, &mut lamports, &mut data, &program_id, false,
            );
            StateHolder::init(&info).unwrap();
        }
        state.insert(
            (state_holder_key, RomeAccount {
                lamports,
                data,
                owner: program_id,
                executable: false,
                writable: true,
                signer: false,
            }),
            None,
        );

        let context = ContextIt::new(
            &state,
            H256::default(),
            1,
            None,
            777,
            b"rlp",
            None,
            state_holder_key,
        ).unwrap();

        context.new_session().unwrap();

        let mut bind = state.info_state_holder_by_key(&state_holder_key).unwrap();
        let info = bind.into_account_info();
        let pri_fee = StateHolder::from_account(&info).unwrap().pri_fee;

        assert_eq!(pri_fee, 777, "new_session must persist the carried pri_fee, not re-derive 0");
    }
}
