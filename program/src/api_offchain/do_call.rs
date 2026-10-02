use {
    crate::{
        context::ContextAt,
        error::Result,
        state::State,
        tx::{
            legacy::Legacy, Base, tx::Tx,
        },
        vm::{vm_atomic::MachineAt, Execute, VmAt}, msg,
        H256, split_h160, H160, U256,
    },
    solana_program::{account_info::AccountInfo, pubkey::Pubkey, keccak,},
};

//  from | rlp
pub fn args(data: &[u8]) -> Result<(H160, &[u8])> {
    let (from, rlp) = split_h160(data)?;
    Ok((from, rlp))
}

pub fn fee_addr_def() -> Option<H160> {
    Some(H160::default())
}
pub fn set_unlimit_gas(legacy: &mut Legacy) {
    legacy.gas_limit = U256::max_value();
    legacy.gas_price = U256::zero();
}

pub fn do_call<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: Atomic call");

    let (from, rlp_) = args(data)?;
    let rlp = Tx::eip1559unsigned(rlp_)?;
    
    let mut legacy = Legacy::from_eip1559_unsigned(rlp.as_raw())?;
    legacy.set_from(from);
    set_unlimit_gas(&mut legacy);
    let chain = legacy.chain_id.as_u64();
    let hash = H256::from(keccak::hash(data).to_bytes());
    let state = State::new(program_id, accounts, chain)?;
    state.base.pda.gas_estimate_key_assert(state.signer.key)?;

    let context = ContextAt::new_gas_estimate(&state);
    let mut vm = VmAt::new_with_unsigned_tx(&state, legacy, hash, &context, fee_addr_def(), 0)?;

    vm.consume(MachineAt::Lock)
}
