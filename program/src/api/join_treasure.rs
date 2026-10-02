use {
    crate::{
        error::{Result, RomeProgramError::*}, State, origin::Origin, msg, state::pda::Seed,
    },
    super::create_treasure::args,
    solana_program::{
        account_info::AccountInfo, pubkey::Pubkey, sysvar::Sysvar, rent::Rent,
    },
    solana_sdk_ids::bpf_loader_upgradeable,
    solana_loader_v3_interface::state::UpgradeableLoaderState::*,
    solana_bincode::limited_deserialize,
    solana_system_interface::{
        instruction::transfer, program as system_program
    },
};

pub fn join_treasure<'a> (
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    let (chain, from, to) = args(data)?;
    msg!("Instruction: join_treasure {} {} {}", chain, from, to);
    let state = State::new_unchecked(program_id, accounts, chain)?;

    let mut payers = vec![];
    for i in from..to {
        let (key, seed) = state.pda.treasure_wallet(i);
        if let Some(info) = state
            .info_wallet(&key, &seed, &system_program::ID, false)? {
            payers.push((key, info.lamports(), seed));
        }
    }

    let info = state.info_any(program_id)?;
    let key = upgradeable_program_key(info)?;
    let info_ = state.info_any(&key)?;
    let upg_auth = upgrade_authority_key(info_)?;

    join(&state, payers, &upg_auth)
}

pub fn upgradeable_program_key(info: &AccountInfo) -> Result<Pubkey> {
    match *info.owner {
        bpf_loader_upgradeable::ID => {
            match limited_deserialize(&info.data.borrow(), u64::MAX)? {
                Program { programdata_address: key } => Ok(key),
                _ => Err(Custom("unexpected UpgradeableLoaderState of the program account".to_string()))
            }
        }
        _=> Err(Custom(format!("unexpected bpf-loader {}", info.owner))),
    }
}

pub fn upgrade_authority_key(info: &AccountInfo) -> Result<Pubkey> {
    match limited_deserialize(&info.data.borrow(), u64::MAX)? {
        ProgramData {slot: _, upgrade_authority_address: key} =>
            key.ok_or(Custom("upgrade_authority_address not found".to_string())),
        _ => Err(Custom("unexpected UpgradeableLoaderState of the program_data account".to_string())),
    }
}

pub fn join<T: Origin>(state: &T, from: Vec<(Pubkey, u64, Seed)>, to: &Pubkey) -> Result<()> {
    let rent = Rent::get()?.minimum_balance(0);

    from
        .into_iter()
        .map(|(key, balance, seed)| {
            let lamports = balance.saturating_sub(rent);
            let ix = transfer(&key, to, lamports);
            state.invoke_signed_unchecked(&ix, vec![seed])
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(())
}
