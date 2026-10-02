use {
    crate::{
        error::{Result, RomeProgramError::*}, context::AccountLock,
        tx::tx::{Tx, TxType,},
        State, tx::deposit::Deposit, context::ContextAt, origin::Origin,
        SOL_DECIMALS, EXIT_REASON, GAS_PRICE, GAS_VALUE, SLOT, TIMESTAMP,
        wei_to_u64, msg,
        state::{Allocate, aux::{checked_transfer, mint_profile},},
    },
    evm::{U256},
    solana_program::{
        account_info::AccountInfo, pubkey::Pubkey, log::sol_log_data, clock::Clock, sysvar::Sysvar,
    },
    solana_system_interface::instruction as system_instruction,
    spl_associated_token_account_interface::address::get_associated_token_address_with_program_id as ata,
    std::{convert::TryInto, mem::size_of,},
};

pub fn deposit<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: deposit");

    let (chain, rlp) = args(data)?;
    let state = State::new(program_id, accounts, chain)?;
    let context = ContextAt::new(&state);

    // TODO: add treasure payment ?
    do_deposit(&state, &context, rlp)
}

pub fn do_deposit<T: Origin + Allocate, L: AccountLock>(state: &T, context: &L, rlp: &[u8]) -> Result<()>{
    let tx = from_rlp(rlp)?;
    mint_rsol(&tx, state, context)?; // checks account locks
    transfer(state, tx.mint)
}

//  chain_id | rlp
pub fn args(data: &[u8]) -> Result<(u64, &[u8])> {
    if data.len() <=  size_of::<u64>() {
        return Err(InvalidInstructionData);
    }

    let (left, rlp) = data.split_at(size_of::<u64>());
    let chain = u64::from_le_bytes(left.try_into().unwrap());

    // TODO: calculate footprint
    Ok((chain, rlp))
}

pub fn from_rlp(rlp: &[u8]) -> Result<Deposit> {
    let tx = match Tx::tx_type(rlp)? {
        TxType::Deposit(rlp) => Deposit::from_rlp(&rlp)?,
        _ => return Err(IncorrectRlpType)
    };

    if tx.mint != tx.value || tx.from != tx.to || tx.data.is_some() {
        return Err(InvalidDepositInstruction)
    }
    Ok(tx)
}

pub fn mint_rsol<T: Origin + Allocate, L: AccountLock>(
    tx: &Deposit,
    state: &T,
    context: &L,
) -> Result<()> {

    context.lock()?;

    state.add_balance(&tx.from, &tx.mint, context)?;

    log_msg()
}

fn  log_msg() -> Result<()>{
    let msg = "Succeed(Stopped)";
    let clock = Clock::get()?;

    sol_log_data(&[EXIT_REASON, &[0x0_u8], &msg.len().to_le_bytes(), msg.as_bytes(), &[]]);
    sol_log_data(&[SLOT, &clock.slot.to_le_bytes()]);
    sol_log_data(&[TIMESTAMP, &clock.unix_timestamp.to_le_bytes()]);
    sol_log_data(&[GAS_VALUE, &[0_u8; 32]]);
    sol_log_data(&[GAS_PRICE, &[0_u8; 32]]);

    Ok(())
}

pub fn transfer<T: Origin>(state: &T, amount: U256) -> Result<()> {
    let wallet = state.wallet()?;

    if let Some(mint) = state.base().owner_info().mint {
        spl_transfer(amount, state, &mint, &wallet)
    } else {
        sol_transfer(amount, state, &wallet)
    }
}

pub fn sol_transfer<T:Origin>(rsol: U256, state: &T, wallet: &Pubkey) -> Result<()> {
    let lamports = wei_to_u64(rsol, SOL_DECIMALS)?;
    let ix = system_instruction::transfer(&state.signer(), &wallet, lamports);

    state.invoke_signed(&ix, vec![], false)
}
pub fn spl_transfer<T:Origin>(rsol: U256, state: &T, mint: &Pubkey, wallet: &Pubkey) -> Result<()> {
    let profile = mint_profile(state, mint)?;
    let (spl_program, decimals) = (profile.program, profile.decimals);
    let from_ata= ata(&state.signer(), &mint, &spl_program);
    let to_ata = ata(&wallet, &mint, &spl_program);
    let tokens = wei_to_u64(rsol, decimals)?;

    // TODO: check the balance
    // if payer has an unsufficient balance, the spl-token program issues confusing  InvalidAccountData error
    let ix = checked_transfer(&profile, mint, &from_ata, &to_ata, &state.signer(), &[], tokens)?;

    state.invoke_signed(&ix, vec![], false)
}