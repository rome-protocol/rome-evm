use {
    super::Emulation,
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{
        api::transmit_tx::args, error::Result, error::RomeProgramError::TxHolderSizeExceeded,
        Data, Holder, TxHolder, TX_HOLDER_MAX_SIZE,
    },
    solana_program::{account_info::IntoAccountInfo, msg, pubkey::Pubkey, instruction::Instruction,},
    std::sync::Arc,
};

pub fn transmit_tx<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    _: Option<Instruction>,
) -> Result<Emulation> {
    msg!("Instruction: Transmit tx");

    let (holder, from, ix_hash, chain, tx) = args(data)?;
    let state = State::new(program_id, Some(*signer), client, chain)?;

    // TODO: the client side should implement the holder filling with taking into account holder header allocation
    let len = from + tx.len();

    let (filled_len, header, reset, key) = {
        let mut bind = state.info_tx_holder(holder, true)?;
        let info = bind.into_account_info();
        let reset = TxHolder::from_account(&info)?.hash != ix_hash;

        if reset {
            TxHolder::reset(&info, ix_hash)?;
        }
        TxHolder::inc_iteration(&info)?;

        let filled_len = Holder::from_account(&info)?.len();
        let header = Holder::offset(&info);
        let key = bind.0;

        // the bind is a copy of the store's account: write the header (hash,
        // iteration) back before the realloc re-reads the account from the store
        state.update(bind);

        (filled_len, header, reset, key)
    };

    if reset || len > filled_len {
        state.realloc(&key, header + len, false)?;
    }

    let mut bind = state.info_tx_holder(holder, false)?;
    let info = bind.into_account_info();

    if info.data_len() > TX_HOLDER_MAX_SIZE {
        return Err(TxHolderSizeExceeded(*info.key))
    }

    Holder::fill(&info, from, len, tx)?;
    state.update(bind);

    Emulation::without_vm(&state)
}
