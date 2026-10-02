use {
    super::{cast, cast_mut, Data},
    crate::error::{Result, RomeProgramError::*},
    solana_program::{account_info::AccountInfo, pubkey::Pubkey},
    std::{
        cell::{Ref, RefMut},
        mem::size_of,
    },
};

#[repr(u8)]
#[derive(PartialEq, Clone, Debug)]
pub enum AccountType {
    New = 0,
    Balance = 1,
    Storage = 2,
    TxHolder = 3,
    StateHolder = 4,
    OwnerInfo = 6,
    AltSlots = 7,
    BridgeProcessed = 8,
    StarterUnwrapDone = 9,
}

impl AccountType {
    pub fn init(info: &AccountInfo, new: AccountType) -> Result<()> {
        let mut old = AccountType::from_account_mut(info)?;

        // TODO: create_pda must fill 0
        if *old != AccountType::New {
            return Err(AccountInitialized(*info.key));
        }
        assert!(new != AccountType::New);

        *old = new;
        Ok(())
    }

    pub fn check_owner(info: &AccountInfo, program_id: &Pubkey) -> Result<()> {
        // TODO introduce the type for any rome-evm accounts
        if info.owner != program_id || info.data_len() == 0 {
            return Err(InvalidOwner(*info.key));
        }

        Ok(())
    }
    pub fn is_ok(info: &AccountInfo, typ: Self, program_id: &Pubkey) -> Result<()> {
        AccountType::check_owner(info, program_id)?;

        if *AccountType::from_account(info)? == typ {
            Ok(())
        } else {
            Err(InvalidAccountType(*info.key))
        }
    }

    pub fn is_paid(&self) -> bool {
        match self {
            AccountType::New => unreachable!(),
            AccountType::Balance | AccountType::Storage => true,
            _ => false
        }
    }
}

impl Data for AccountType {
    type Item<'a> = Ref<'a, Self>;
    type ItemMut<'a> = RefMut<'a, Self>;

    fn from_account<'a>(info: &'a AccountInfo) -> Result<Self::Item<'a>> {
        cast(info, Self::offset(info), Self::size(info))
    }
    fn from_account_mut<'a>(info: &'a AccountInfo) -> Result<Self::ItemMut<'a>> {
        cast_mut(info, Self::offset(info), Self::size(info))
    }
    fn offset(_info: &AccountInfo) -> usize {
        0
    }
    fn size(_info: &AccountInfo) -> usize {
        size_of::<Self>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins every `AccountType` discriminant byte — serialized as the first
    /// byte of every on-chain PDA of that type. `AltSlots = 7` stays
    /// vestigial post-ALT-removal: deleting it would make live AltSlots
    /// PDAs unparseable and invite a renumbering that corrupts type-tagging.
    #[test]
    fn test_account_type_discriminants_stable() {
        assert_eq!(AccountType::New as u8, 0);
        assert_eq!(AccountType::Balance as u8, 1);
        assert_eq!(AccountType::Storage as u8, 2);
        assert_eq!(AccountType::TxHolder as u8, 3);
        assert_eq!(AccountType::StateHolder as u8, 4);
        assert_eq!(AccountType::OwnerInfo as u8, 6);
        assert_eq!(AccountType::AltSlots as u8, 7);
        assert_eq!(AccountType::BridgeProcessed as u8, 8);
        assert_eq!(AccountType::StarterUnwrapDone as u8, 9);
    }
}
