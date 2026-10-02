use {
    super::{AccountType, Data, Ver,},
    crate::error::Result,
    solana_program::account_info::AccountInfo,
    std::cell::{Ref, RefMut},
};
// Historical: once-per-user marker PDA from the abandoned
// `grant_starter_unwrap` design (see config.rs `STARTER_UNWRAP_MIN/MAX`
// comment). Existence meant "this user's first-time starter unwrap has
// already been granted." Was keyed on (user_h160). Data layout:
// [AccountType | Ver] — no
// further fields. `grant_starter_unwrap` itself no longer exists; the
// SEED / AccountType / struct are kept for account-type discriminant
// compat — no instruction allocates new ones.
//
// Mirrored BridgeProcessed in shape but keyed on user instead of
// (source_chain, source_tx_hash). Together they enforced both
// "once per user ever" (this PDA) and "once per source-chain bridge
// tx" (BridgeProcessed PDA) — defense in depth against a misbehaving
// worker that tried to drain a user's starter via a single bridge.

#[repr(C, packed)]
pub struct StarterUnwrapDone;

impl StarterUnwrapDone {
    pub fn init(info: &AccountInfo) -> Result<()> {
        Ver::init(info, AccountType::StarterUnwrapDone, 0)
    }
}

impl Data for StarterUnwrapDone {
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
