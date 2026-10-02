use {
    crate::{
        Emulation, api::do_tx::atomic_tx,
    },
    crate::{
        state::State, account_storage::AccountStorage, ContextAt,
    },
    rome_evm::{
        api_offchain::do_call::args,
        error::Result,
        tx::{
            Base,
            legacy::Legacy,
            tx::Tx,
        }, H256, vm::VmAt,
        api_offchain::do_call::{
            fee_addr_def, set_unlimit_gas,
        },
    },
    solana_program::{
        msg, pubkey::Pubkey, instruction::Instruction, keccak,
    },
    std::sync::Arc,
};

pub fn do_call<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    _: Option<Instruction>,
) -> Result<Emulation> {
    msg!("Instruction: Atomic call");

    let (from, rlp_) = args(data)?;
    let rlp = Tx::eip1559unsigned(rlp_)?;

    let mut legacy = Legacy::from_eip1559_unsigned(rlp.as_raw())?;
    legacy.set_from(from);
    set_unlimit_gas(&mut legacy);

    let chain = legacy.chain_id.as_u64();
    let hash = H256::from(keccak::hash(data).to_bytes());

    let state = State::new_ed25519(program_id, Some(*signer), client, chain, None)?;
    state.base.pda.gas_estimate_key_assert(state.signer.as_ref().unwrap())?;

    let context = ContextAt::new_gas_estimate(&state);
    let vm = VmAt::new_with_unsigned_tx(&state, legacy, hash, &context, fee_addr_def(), 0)?;

    atomic_tx(vm)
}
