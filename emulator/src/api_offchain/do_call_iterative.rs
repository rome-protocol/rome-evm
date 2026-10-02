use {
    crate::{
        Emulation,
    },
    crate::{
        context::ContextIt,
        state::State,
        account_storage::AccountStorage,
        api::do_tx_iterative::iterative_tx,
    },
    rome_evm::{
        api_offchain::do_call_iterative::args,
        error::Result,
        tx::tx::Tx, H256,
    },
    solana_program::{
        msg, pubkey::Pubkey, instruction::Instruction,
        // keccak,
    },
    std::sync::Arc,
};

pub fn do_call_iterative<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    cli: Arc<dyn AccountStorage>,
    _: Option<Instruction>,
) -> Result<Emulation> {
    msg!("Instruction: Iterative call");

    let (from, limit, rlp_) = args(data)?;

    // TODO: fix it
    let hash = H256::default();
    // let hash = H256::from(keccak::hash(rlp_).to_bytes());

    let chain = Tx::chain_id_from_rlp(rlp_)?;
    let rlp = Tx::eip1559unsigned(rlp_)?;

    let state = State::new_ed25519(program_id, Some(*signer), cli, chain, None)?;
    state.base.pda.gas_estimate_key_assert(state.signer.as_ref().unwrap())?;

    let context = ContextIt::new_gas_estimate(&state, rlp.as_raw(), from, hash)?;
    iterative_tx(&state, context, limit)
}
