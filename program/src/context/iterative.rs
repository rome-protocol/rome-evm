use {
    super::Context,
    crate::{
        api_offchain::do_call::{
            fee_addr_def, set_unlimit_gas,
        },
        accounts::Iterations,
        accounts::{Data, Holder, StateHolder},
        error::Result,
        state::{origin::Origin, Allocate, JournaledState},
        tx::{
            legacy::Legacy,
            tx::Tx, Base,
        },
        vm::{Snapshot, Vm},
        State, do_tx_holder::transmit_fee,
        config::{MAX_KEYS_WITHOUT_ALT, GAS_ESTIMATE_SESSION, GAS_ESTIMATE_HOLDER,},
    },
    borsh::{BorshDeserialize, BorshSerialize},
    evm::{H160, H256},
    solana_program::{
        account_info::AccountInfo,
    },
};

pub enum Request<'a> {
    Rlp(&'a [u8]),
    UnsignedEip1559(&'a [u8], H160),
}
impl Request<'_> {
    pub fn to_tx<T: Origin>(&self, state: &T) -> Result<Tx> {
        match self {
            Request::Rlp(rlp) => Tx::from_instruction(rlp, state),
            Request::UnsignedEip1559(rlp, from) => {
                let mut legacy = Legacy::from_eip1559_unsigned(rlp)?;
                legacy.set_from(*from);
                set_unlimit_gas(&mut legacy);

                Ok(Tx::from_legacy(legacy))
            },
        }
    }
}

pub struct ContextIt<'a, 'b> {
    pub state: &'b State<'a>,
    pub tx_hash: H256,
    request: Request<'b>,
    pub session: u64,
    pub fee_addr: Option<H160>,
    pub pri_fee: u64,
    pub tx_holder: Option<&'a AccountInfo<'a>>,
    pub state_holder: &'a AccountInfo<'a>,
    pub alt: bool,
    pub gas_estimate: bool,
}

impl<'a, 'b> ContextIt<'a, 'b> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: &'b State<'a>,
        rlp: &'b [u8],
        tx_hash: H256,
        session: u64,
        fee_addr: Option<H160>,
        pri_fee: u64,
        tx_holder: Option<&'a AccountInfo<'a>>,
        state_holder: &'a AccountInfo<'a>,
    ) -> Result<Self> {

        Ok(Self {
            state,
            tx_hash,
            request: Request::Rlp(rlp),
            session,
            fee_addr,
            pri_fee,
            tx_holder,
            state_holder,
            alt: state.all().len() > MAX_KEYS_WITHOUT_ALT,
            gas_estimate: false,
        })
    }
    pub fn new_gas_estimate(state: &'b State<'a>, rlp: &'a [u8], from: H160, hash: H256) -> Result<Self> {
        let state_holder = state.info_state_holder(GAS_ESTIMATE_HOLDER, true)?;

        Ok(Self {
            state,
            tx_hash: hash,
            request: Request::UnsignedEip1559(rlp, from),
            session: GAS_ESTIMATE_SESSION,
            fee_addr: fee_addr_def(),
            pri_fee: 0,
            tx_holder: None,
            state_holder,
            alt: state.all().len() > MAX_KEYS_WITHOUT_ALT,
            gas_estimate: true,
        })
    }
}

impl<'a, 'b> Context for ContextIt<'a, 'b> {
    fn tx(&self) -> Result<Tx> {
        self.request.to_tx(self.state)
    }

    fn set_iteration(&self, iteration: Iterations) -> Result<()> {
        StateHolder::set_iteration(self.state_holder, iteration)
    }

    fn get_iteration(&self) -> Result<Iterations> {
        StateHolder::get_iteration(self.state_holder)
    }

    fn iter_cnt(&self) -> Result<u64> {
        StateHolder::get_iter_cnt(self.state_holder)
    }

    fn serialize<T: Origin + Allocate>(&self, vm: &Vm<T>) -> Result<()> {
        serialize_impl(self.state_holder, vm)
    }

    fn deserialize<T: Origin + Allocate>(&self, vm: &mut Vm<T>) -> Result<()> {
        deserialize_impl(self.state_holder, vm)
    }

    fn allocate_holder(&self) -> Result<()> {
        let len = self.state_holder.data_len() + self.state.alloc_limit();
        self.state.realloc(self.state_holder, len)
    }

    fn new_session(&self) -> Result<()> {
        StateHolder::set_session(self.state_holder, self.tx_hash, self.session, self.pri_fee)?;

        if let Some(info) = self.tx_holder {
            let fee = transmit_fee(info)?;
            self.state.base().add_fee(fee)?;
        }
        
        Ok(())
    }

    fn has_session(&self) -> Result<bool> {
        StateHolder::has_session(self.state_holder, self.tx_hash, self.session)
    }

    fn tx_hash(&self) -> H256 {
        self.tx_hash
    }

    fn fee_recipient(&self) -> Option<H160> {
        self.fee_addr
    }

    fn state_holder_len(&self) -> Result<usize> {
        Ok(Holder::size(self.state_holder))
    }

    fn collect_fees(&self, lamports_fee: u64, lamports_refund: u64) -> Result<()> {
        StateHolder::collect_fees(self.state_holder, lamports_fee, lamports_refund)
    }

    fn fees(&self) -> Result<(u64, u64)> {
        StateHolder::fees(self.state_holder)
    }

    fn is_gas_estimate(&self) -> bool {
        self.gas_estimate
    }

    fn priority_fee(&self) -> Result<u64> {
        let holder = StateHolder::from_account(self.state_holder)?;
        Ok(holder.pri_fee)
    }
}

// these functions are used both in the contract and in the emulator
pub fn serialize_impl<T: Origin + Allocate>(info: &AccountInfo, vm: &Vm<T>) -> Result<()> {
    let mut into: &mut [u8] = &mut Holder::from_account_mut(info)?;
    Snapshot::serialize(&vm.snapshot, &mut into)?;
    vm.handler.serialize(&mut into)?;
    vm.return_value.serialize(&mut into)?;
    vm.exit_reason.serialize(&mut into)?;
    vm.handler.state.base().pda.serialize(&mut into)
}

pub fn deserialize_impl<T: Origin + Allocate>(info: &AccountInfo, vm: &mut Vm<T>) -> Result<()> {
    let mut bin: &[u8] = &Holder::from_account(info)?;

    vm.snapshot = Snapshot::deserialize(&mut bin)?;
    vm.handler = JournaledState::deserialize(&mut bin, vm.handler.state)?;
    vm.return_value = BorshDeserialize::deserialize(&mut bin)?;
    vm.exit_reason = BorshDeserialize::deserialize(&mut bin)?;
    vm.handler.state.base().pda.deserialize(&mut bin)
}
