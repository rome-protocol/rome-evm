use {
    crate::{
        context::ContextIt,
        error::Result,
        split_u64,
        state::State,
        vm::{vm_iterative::MachineIt::FromStateHolder, Execute, VmIt},
        msg, H160, split_h160,
        tx::tx::Tx,
    },
    solana_program::{
        account_info::AccountInfo, pubkey::Pubkey,
        // keccak,
    },
};
use crate::H256;

// from | limit | rlp
pub fn args(data: &[u8]) -> Result<(H160, u64, &[u8])> {
    let (from, right) = split_h160(data)?;
    let (limit, rlp, ) = split_u64(right)?;
    Ok((from, limit, rlp))
}

pub fn do_call_iterative<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: Iterative call");

    let (from, limit, rlp_) = args(data)?;
    // TODO: fix it
    let hash = H256::default();
    // let hash = H256::from(keccak::hash(rlp_).to_bytes());

    let chain = Tx::chain_id_from_rlp(rlp_)?;
    let rlp = Tx::eip1559unsigned(rlp_)?;

    let state = State::new(program_id, accounts, chain)?;
    state.base.pda.gas_estimate_key_assert(state.signer.key)?;

    let context = ContextIt::new_gas_estimate(&state, rlp.as_raw(), from, hash)?;
    let mut vm = VmIt::new(&state, &context, limit)?;
    vm.consume(FromStateHolder)
}
