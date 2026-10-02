use {
    super::Emulation,
    crate::{state::State, account_storage::AccountStorage},
    rome_evm::{
        error::Result, api::{
            create_treasure::args, join_treasure::{join, upgrade_authority_key, upgradeable_program_key,}
        }, state::pda::Seed,
    },
    solana_program::{msg, pubkey::Pubkey, account_info::IntoAccountInfo, instruction::Instruction,},
    std::sync::Arc,
    solana_system_interface::program as system_program,
};

pub fn join_treasure<'a>(
    program_id: &'a Pubkey,
    data: &'a [u8],
    signer: &'a Pubkey,
    client: Arc<dyn AccountStorage>,
    _: Option<Instruction>,
) -> Result<Emulation> {
    let (chain, from, to) = args(data)?;
    msg!("Instruction: join_treasure {} {} {}", chain, from, to);

    let state = State::new_unchecked(program_id, Some(*signer), client, chain)?;

    let mut payers = vec![];
    for i in from..to {
        let (key, _) = state.pda.treasure_wallet(i);
        if let Some(bind) = state
            .info_wallet(&key, &system_program::ID, false)? {
            payers.push((key, bind.1.lamports, Seed::default()));
        }
    }

    let mut bind = state.info_sys(program_id)?;
    let info = bind.into_account_info();
    let key = upgradeable_program_key(&info)?;

    let mut bind_ = state.info_sys(&key)?;
    let info_ = bind_.into_account_info();
    let upg_auth = upgrade_authority_key(&info_)?;

    join(&state, payers, &upg_auth)?;

    Emulation::without_vm(&state)
}
