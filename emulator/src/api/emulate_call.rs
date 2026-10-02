use {
    crate::{
        ContextAt, state::State, Emulation, account_storage::AccountStorage, do_tx::atomic_tx,
    },
    rome_evm::{
        error::Result, tx::legacy::Legacy, vm::VmAt, H256,
    },
    solana_program::{msg, pubkey::Pubkey, keccak,},
    std::sync::Arc,
};

pub fn emulate_call(
    program_id: &Pubkey,
    legacy: Legacy,
    client: Arc<dyn AccountStorage>,
    signer: &Pubkey,
) -> Result<Emulation> {
    msg!(">> emulate call");
    let state = State::new(program_id, Some(*signer), client, legacy.chain_id.as_u64())?;
    let context = ContextAt::new_gas_estimate(&state);
    let hash = H256::from(keccak::hash(&[1, 2, 3]).to_bytes());

    let vm = VmAt::new_with_unsigned_tx(&state, legacy, hash, &context, None, 0)?;
    atomic_tx(vm)
}
