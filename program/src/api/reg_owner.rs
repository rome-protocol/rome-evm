use {
    crate::{
        error::{Result, RomeProgramError::*},
        registration_key, OwnerInfo, State, origin::Origin,
        RSOL_DECIMALS, SOL_DECIMALS, split_u64, split_u8, split_pubkey_opt,
        msg, state::aux::mint_owner_decimals,
    },
    solana_program::{
        account_info::AccountInfo, pubkey::Pubkey,
    },
    std::{
        mem::size_of,
    },
    spl_associated_token_account_interface::instruction::create_associated_token_account_idempotent,
};

// chain_id  + single_state + {mint_key}
pub fn args(data: &[u8]) -> Result<(u64, bool, Option<Pubkey>)> {
    let (chain, data) = split_u64(data)?;
    let (single_state, data) = split_u8(data)?;
    let mint = split_pubkey_opt(data)?;

    Ok((chain, single_state == 1, mint))
}

pub fn check_signer(signer: &Pubkey) -> Result<()> {
    if *signer != registration_key::ID {
        return Err(Custom(format!(
            "private instruction must be signed by registration keypair: {}",
            registration_key::ID
        )));
    }

    Ok(())
}

// Instruction is used to registry rollup owner.
// This private instruction must be signed by the upgrade-authority keypair.
pub fn reg_owner<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    let (chain, single_state,  mint) = args(data)?;
    msg!("Instruction: chain_id registration {} {:?} {}", chain, mint, single_state);

    let state = State::new_unchecked(program_id, accounts, chain)?;
    check_decimals(mint, &state)?;

    let info = state.info_owner_reg(true)?;
    check_signer(state.signer.key)?;
    state.realloc(info, info.data_len() + size_of::<OwnerInfo>())?;
    OwnerInfo::reg_chain(info, chain, single_state, mint)?;

    let wallet = state.info_sol_wallet(true)?;
    create_spl_wallet(&state, mint, wallet.key)?;

    Ok(())
}

pub fn create_spl_wallet<T: Origin>(
    state: &T,
    mint: Option<Pubkey>,
    wallet: &Pubkey
) -> Result<()> {
    if let Some(mint_) = mint.as_ref() {
        let spl_program = state.owner(&mint_)?;
        assert!(spl_program == spl_token_interface::ID || spl_program == spl_token_2022_interface::ID);

        let ix = create_associated_token_account_idempotent(
            &state.signer(),
            wallet,
            mint_,
            &spl_program
        );

        state.invoke_signed(&ix, vec![], false)?;
    }

    Ok(())
}
pub fn check_decimals<T: Origin>(mint: Option<Pubkey>, state: &T) -> Result<()> {
    if let Some(mint_) = mint {
        let (_, decimals) = mint_owner_decimals(state, &mint_)?;

        if RSOL_DECIMALS < decimals {
            return Err(TooHighSplDecimals(decimals, mint_))
        }
    } else {
        assert!(RSOL_DECIMALS > SOL_DECIMALS);
    }

    Ok(())
}