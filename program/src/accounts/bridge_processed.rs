use {
    super::{AccountType, Data, Ver,},
    crate::error::Result,
    solana_program::account_info::AccountInfo,
    std::cell::{Ref, RefMut},
};
// Replay-protection marker PDA. Existence = "bridge tx already processed."
// Keyed on (source_chain, source_tx_hash) via Pda::bridge_processed_key.
// Data layout: [AccountType | Ver] — no further fields. The PDA is
// created by `settle_inbound_bridge_v2` (`settle_inbound_bridge_v2.rs:250-256`)
// the first time it processes a given (source_chain, source_tx_hash); any
// later call for the same key will fail at info_pda(...,or_create=true)
// because `AccountType::init` refuses to re-init a typed account (returns
// AccountInitialized).

#[repr(C, packed)]
pub struct BridgeProcessed;

impl BridgeProcessed {
    pub fn init(info: &AccountInfo) -> Result<()> {
        Ver::init(info, AccountType::BridgeProcessed, 0)
    }
}

impl Data for BridgeProcessed {
    type Item<'a> = Ref<'a, Self>;
    type ItemMut<'a> = RefMut<'a, Self>;

    fn from_account<'a>(_: &'a AccountInfo) -> Result<Self::Item<'a>> {
        unreachable!()
    }
    fn from_account_mut<'a>(_: &'a AccountInfo) -> Result<Self::ItemMut<'a>> {
        unreachable!()
    }
    fn size(_info: &AccountInfo) -> usize {
        0
    }
    fn offset(info: &AccountInfo) -> usize {
        Ver::offset(info) + Ver::size(info)
    }
}