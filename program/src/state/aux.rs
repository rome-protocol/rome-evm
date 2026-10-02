use solana_program::keccak;
use {
    crate::{
        config::{REVERT_ERROR, RSOL_DECIMALS},
        non_evm::aux::slice_to_abi, error::{Result, RomeProgramError::*},
        origin::Origin,
    },
    borsh::{BorshDeserialize, BorshSerialize},
    solana_program::{
        account_info::{Account as AccountTrait, AccountInfo},
        pubkey::Pubkey, instruction::{Instruction, AccountMeta, },
    },
    evm::U256,
};
use crate::H160;
use crate::pda::Seed;

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default)]
pub struct Account {
    pub lamports: u64,
    pub data: Vec<u8>,
    pub owner: Pubkey,
    pub executable: bool,
    pub writable: bool,
    pub signer: bool,
}

impl Account {
    pub fn from_account_info(info: &AccountInfo) -> Self {
        Self {
            lamports: info.lamports(),
            data: info.data.borrow().to_vec(),
            owner: *info.owner,
            executable: info.executable,
            writable: info.is_writable,
            signer: info.is_signer,
        }
    }
}

#[cfg(not(target_os = "solana"))]
impl From<Account> for solana_account::Account {
    fn from(acc: Account) -> Self {
        Self {
            lamports: acc.lamports,
            data: acc.data,
            owner: acc.owner,
            executable: acc.executable,
            rent_epoch: 0,
        }
    }
}

impl AccountTrait for Account {
    fn get(&mut self) -> (&mut u64, &mut [u8], &Pubkey, bool) {
        (
            &mut self.lamports,
            self.data.as_mut_slice(),
            &self.owner,
            self.executable,
        )
    }
}

pub fn revert_msg(msg: String) -> Vec<u8> {
    let mut abi = slice_to_abi(msg.as_bytes(), 0);
    #[allow(unused_assignments)]
    let mut vec = Vec::with_capacity(REVERT_ERROR.len() + abi.len());
    vec = REVERT_ERROR.to_vec();
    vec.append(&mut abi);

    vec
}

pub fn wei_to_u64(wei: U256, decimals: u8) -> Result<u64> {
    assert!(RSOL_DECIMALS >= decimals);

    let pow = (RSOL_DECIMALS - decimals) as usize;
    let (int_amount, remainder) = wei.div_mod(U256::exp10(pow));

    if !remainder.is_zero() {
        return Err(TxValueNotMultipleOf(format!("10^{}", pow)))
    }
    if int_amount > U256::from(u64::MAX) {
        return Err(TxValueExceedsU64)
    }

    Ok(int_amount.as_u64())
}

/// A mint's facts, on the legacy track. Byte interpretation lives in
/// `mint_profile`, the one place that reads mint bytes; the zero-lamport check
/// stays here because it is a property of the account, not of its bytes.
pub fn mint_profile<T: Origin>(
    state: &T,
    mint: &Pubkey,
) -> Result<crate::state::mint_profile::MintProfile> {
    let acc = state.account(mint)?;

    if acc.lamports == 0 {
        return Err(InvalidSplMintAccount(*mint));
    }
    crate::state::mint_profile::profile(&acc.owner, &acc.data)
        .map_err(|_| InvalidSplMintAccount(*mint))
}

/// Owner + decimals, for callers that need nothing else. A projection of
/// `mint_profile`, not a second reader.
pub fn mint_owner_decimals<T: Origin>(state: &T, mint: &Pubkey) -> Result<(Pubkey, u8)> {
    let p = mint_profile(state, mint)?;
    Ok((p.program, p.decimals))
}

/// The one place a legacy-track checked transfer is built.
///
/// It reads nothing — the caller already holds the mint's facts — and exists so
/// that the armed-hook refusal cannot be forgotten at one of the nine sites that
/// construct a `transfer_checked`. A hooked transfer CPIs the hook program with
/// the accounts its `ExtraAccountMetaList` declares, and no legacy transfer path
/// supplies them, so without this the transfer fails with the token program's
/// error rather than one naming the cause. The cached track refuses the same
/// thing where it stages; this is the legacy half of that.
///
/// An unarmed hook is inert and must pass: its program_id is zero, so the
/// processor's `get_program_id()` returns `None` and no CPI is reached.
pub fn checked_transfer(
    profile: &crate::state::mint_profile::MintProfile,
    mint: &Pubkey,
    from_ata: &Pubkey,
    to_ata: &Pubkey,
    authority: &Pubkey,
    signers: &[&Pubkey],
    amount: u64,
) -> Result<Instruction> {
    if let Some(hook) = profile.hook_program {
        return Err(NonEvmCallError(format!(
            "mint {mint} has an armed transfer hook {hook}; \
             hooked transfers are not supported"
        )));
    }
    if let Some(bps) = profile.fee_bps {
        return Err(NonEvmCallError(format!(
            "mint {mint} has an armed transfer fee ({bps} bps); \
             fee-bearing transfers are not supported"
        )));
    }

    spl_token_2022_interface::instruction::transfer_checked(
        &profile.program,
        from_ata,
        mint,
        to_ata,
        authority,
        signers,
        amount,
        profile.decimals,
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::state::mint_profile::profile,
        spl_token_2022_interface::extension::ExtensionType,
    };

    const T22: Pubkey = spl_token_2022_interface::ID;
    const PYUSD: &[u8] = include_bytes!("../../tests/fixtures/pyusd.bin");

    fn armed(program: Pubkey) -> Vec<u8> {
        // Walk the TLV to the hook payload the way the processor does, then set
        // the program_id that arms it: authority first, program_id second.
        let mut d = PYUSD.to_vec();
        let mut i = 166;
        loop {
            let t = u16::from_le_bytes([d[i], d[i + 1]]);
            let len = u16::from_le_bytes([d[i + 2], d[i + 3]]) as usize;
            if t == ExtensionType::TransferHook as u16 {
                let at = i + 4 + 32;
                d[at..at + 32].copy_from_slice(program.as_ref());
                return d;
            }
            i += 4 + len;
        }
    }

    fn with_fee(bps: u16) -> Vec<u8> {
        // Same TLV walk as `armed`, landed on TransferFeeConfig's basis-points
        // fields instead of TransferHook's program_id. Layout matches
        // `program/tests/mint_profile.rs::with_fee_bps`: two Pubkey authorities
        // (64) + withheld(8), then older TransferFee, then newer TransferFee —
        // each epoch(8) + maximum_fee(8) + basis_points(2).
        let mut d = PYUSD.to_vec();
        let mut i = 166;
        loop {
            let t = u16::from_le_bytes([d[i], d[i + 1]]);
            let len = u16::from_le_bytes([d[i + 2], d[i + 3]]) as usize;
            if t == ExtensionType::TransferFeeConfig as u16 {
                let at = i + 4;
                for fee in [at + 72, at + 90] {
                    d[fee + 16..fee + 18].copy_from_slice(&bps.to_le_bytes());
                }
                return d;
            }
            i += 4 + len;
        }
    }

    fn build(data: &[u8]) -> Result<Instruction> {
        let p = profile(&T22, data).unwrap();
        checked_transfer(
            &p,
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            &[],
            1,
        )
    }

    #[test]
    fn an_armed_hook_is_refused_by_name() {
        let err = build(&armed(Pubkey::new_unique())).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("armed transfer hook"),
            "the legacy track must name the cause, not defer to the token program: {msg}"
        );
    }

    #[test]
    fn a_present_but_unarmed_hook_still_transfers() {
        // The same fixture, hook present, program_id zero. Refusing this would
        // refuse most real Token-2022 mints.
        let p = profile(&T22, PYUSD).unwrap();
        assert_eq!(p.hook_program, None, "fixture must be unarmed");
        let ix = build(PYUSD).expect("an inert hook must not block a transfer");
        assert_eq!(ix.program_id, T22, "and it routes to the mint's own program");
    }

    #[test]
    fn an_armed_fee_is_refused_by_name() {
        // A fee-bearing Token-2022 gas mint must not reach transfer_checked: the
        // pool ATA would receive net-of-fee while settle credits the user gross,
        // silently under-backing the pool.
        let err = build(&with_fee(150)).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("armed transfer fee"),
            "the legacy track must name the cause, not defer to the token program: {msg}"
        );
    }

    #[test]
    fn a_present_but_unarmed_fee_still_transfers() {
        // PYUSD's fee extension is present at zero bps — inert, and must pass.
        let p = profile(&T22, PYUSD).unwrap();
        assert_eq!(p.fee_bps, None, "fixture's fee must be unarmed");
        let ix = build(PYUSD).expect("an inert fee must not block a transfer");
        assert_eq!(ix.program_id, T22, "and it routes to the mint's own program");
    }
}


#[derive(BorshSerialize, BorshDeserialize, Clone)]
pub struct AccountMeta_ {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}
impl From<AccountMeta> for AccountMeta_ {
    fn from(value: AccountMeta) -> Self {
        Self {
            pubkey: value.pubkey,
            is_signer: value.is_signer,
            is_writable: value.is_writable,
        }
    }
}
impl From<AccountMeta_> for AccountMeta {
    fn from(value: AccountMeta_) -> Self {
        Self {
            pubkey: value.pubkey,
            is_signer: value.is_signer,
            is_writable: value.is_writable,
        }
    }
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct Ix {
    program_id: Pubkey,
    accounts: Vec<AccountMeta_>,
    data: Vec<u8>,
    seeds: Vec<Seed>,
}

impl Ix {
    pub fn new(ix: Instruction, seeds: Vec<Seed>) -> Self {
        let accounts = ix
            .accounts
            .into_iter()
            .map(|a| a.into())
            .collect::<Vec<_>>();

        Self {
            program_id: ix.program_id,
            accounts,
            data: ix.data,
            seeds,
        }
    }

    pub fn cast(self) -> (Instruction, Vec<Seed>) {
        let accounts = self
            .accounts
            .into_iter()
            .map(|a| a.into())
            .collect::<Vec<_>>();

        let ix = Instruction {
            program_id: self.program_id,
            accounts,
            data: self.data,
        };

        (ix, self.seeds)
    }
}

pub fn derive_sender(key: &Pubkey) -> H160 {
    let hash = keccak::hashv(&[key.as_ref()]);
    H160::from_slice(&hash.to_bytes()[12..32])
}

