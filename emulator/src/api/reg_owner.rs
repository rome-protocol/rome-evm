use {
    super::Emulation,
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{
        accounts::OwnerInfo,
        api::reg_owner::{args, check_signer, check_decimals, create_spl_wallet},
        error::Result,
    },
    solana_program::{account_info::IntoAccountInfo, msg, pubkey::Pubkey, instruction::Instruction,},
    std::{
        sync::Arc, mem::size_of,
    },
};

pub fn reg_owner<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    _: Option<Instruction>,
) -> Result<Emulation> {
    let (chain, single_state, mint) = args(data)?;
    msg!("Instruction: chain_id registration {} {:?}, single-state {}", chain, mint, single_state);

    let state = State::new_unchecked(program_id, Some(*signer), client, chain)?;
    check_decimals(mint, &state)?;

    let bind = state.info_owner_reg(true)?;
    check_signer(signer)?;
    let len = bind.1.data.len() + size_of::<OwnerInfo>();
    state.realloc(&bind.0, len, false)?;

    let mut bind = state.info_owner_reg(false)?;
    let info = bind.into_account_info();
    OwnerInfo::reg_chain(&info, chain, single_state, mint)?;
    state.update(bind);

    let wallet = state.info_sol_wallet(true)?;
    create_spl_wallet(&state, mint, &wallet.0)?;

    Emulation::without_vm(&state)
}
