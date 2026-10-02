use {
    crate::{
        error::{Result, RomeProgramError::*,},
        state::{
            State, aux::{checked_transfer, mint_profile}, origin::Origin,
        },
        msg,
        aux::derive_sender, split_u64, split_pubkey,
    },
    solana_program::{account_info::AccountInfo, pubkey::Pubkey,},
    spl_associated_token_account_interface::address::get_associated_token_address_with_program_id as ata,
};

//  chain_id | tokens
pub fn args(data: &[u8]) -> Result<(u64, Pubkey, u64)> {
    let (chain, right) = split_u64(data)?;
    let (mint, right) = split_pubkey(right)?;
    let (tokens, right) = split_u64(right)?;

    if !right.is_empty() {
        return Err(InvalidInstructionData);
    }

    Ok((chain, mint, tokens))
}

pub fn activate_ata<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: Activation ata");

    let (chain, mint, tokens ) = args(data)?;
    let state = State::new(program_id, accounts, chain)?;
    let synthetic = derive_sender(&state.signer.key);
    let (pda, _) = state.pda.external_auth(&synthetic);

    spl_transfer(tokens, &state, &pda, &mint)
}

pub fn spl_transfer<T:Origin>(tokens: u64, state: &T, to: &Pubkey, mint: &Pubkey) -> Result<()> {
    let profile = mint_profile(state, mint)?;
    let spl_program = profile.program;
    let from_ata= ata(&state.signer(), mint, &spl_program);
    let to_ata = ata(to, mint, &spl_program);

    let ix = checked_transfer(&profile, mint, &from_ata, &to_ata, &state.signer(), &[], tokens)?;

    state.invoke_signed(&ix, vec![], false)
}