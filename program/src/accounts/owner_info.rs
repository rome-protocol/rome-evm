use {
    super::{AccountType, Data, Ver},
    crate::{
        accounts::{cast_slice, cast_slice_mut, slice_len},
        error::{Result, RomeProgramError::*},
    },
    solana_program::{account_info::AccountInfo, clock::Clock, pubkey::Pubkey, sysvar::Sysvar, },
    std::{cell::{Ref, RefMut}},
};

// Version 2 of OwnerInfo struct
#[derive(Clone, Default, Debug)]
#[repr(C, packed)]
pub struct OwnerInfo {
    pub chain: u64,
    pub mint: Option<Pubkey>,
    pub slot: u64,
    pub single_state: bool,
}

impl OwnerInfo {
    pub fn reg_chain(info: &AccountInfo, chain: u64, single_state: bool, mint: Option<Pubkey>) -> Result<()> {
        if OwnerInfo::from_account(info)?
            .iter()
            .find(|info| info.chain == chain).is_some() {
            return Err(Custom(format!("chain {} is already registered", chain)));
        }

        let mut slice = Self::from_account_mut(info)?;
        let owner = slice.last_mut().unwrap();

        *owner = Self {
            chain,
            mint,
            slot: Clock::get()?.slot,
            single_state,
        };

        Ok(())
    }
    pub fn init(info: &AccountInfo) -> Result<()> {
        Ver::init(info, AccountType::OwnerInfo, 2)?;

        let slice = OwnerInfo::from_account_mut(info)?;
        assert!(slice.is_empty());

        Ok(())
    }
    pub fn owner_info(info: &AccountInfo, chain: u64) -> Result<OwnerInfo> {
        OwnerInfo::from_account(info)?
            .iter()
            .find(|info| info.chain == chain)
            .cloned()
            .ok_or(UnregisteredChainId(chain))
    }
}

impl Data for OwnerInfo {
    type Item<'a> = Ref<'a, [Self]>;
    type ItemMut<'a> = RefMut<'a, [Self]>;

    fn from_account<'a>(info: &'a AccountInfo) -> Result<Self::Item<'a>> {
        assert_eq!(Ver::get(info)?, 2); // TODO: remove
        cast_slice(info, Self::offset(info), Self::size(info))
    }
    fn from_account_mut<'a>(info: &'a AccountInfo) -> Result<Self::ItemMut<'a>> {
        assert_eq!(Ver::get(info)?, 2); // TODO: remove
        cast_slice_mut(info, Self::offset(info), Self::size(info))
    }
    fn offset(info: &AccountInfo) -> usize {
        // account_type | ver | reg_owner
        Ver::offset(info) + Ver::size(info)
    }
    fn size(info: &AccountInfo) -> usize {
        slice_len::<Self>(info)
    }
}
