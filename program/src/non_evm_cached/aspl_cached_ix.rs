use {
    solana_program::{
        instruction::Instruction, pubkey::Pubkey,
        rent::Rent, sysvar::Sysvar,
    },
    solana_system_interface::{program as system_program},
    crate::{
        error::Result, origin::Origin, error::RomeProgramError::*, 
        non_evm::{get_h160, get_pubkey, next, Program, }, 
        state::NonEvmState,
        handler::CallInterrupt,
        non_evm_cached::{
            CachedMut, CachedState, SplCached, SystemCached,
        },
    },
    spl_associated_token_account_interface::{
        address::get_associated_token_address_with_program_id as ata_,
        instruction::create_associated_token_account_idempotent,
    },
    spl_token_2022_interface::{
        extension::StateWithExtensions, state::{Account as Account2022}, instruction::{initialize_account3, initialize_immutable_owner},
    },
};

#[derive(Clone)]
pub struct ASplCachedReader<'a, T: Origin> {
    pub cached: CachedState<'a, T>,
}

impl<'a, T: Origin> ASplCachedReader<'a, T> {
    pub fn new(state: &'a T, ne_state: Option<&'a NonEvmState>) -> Self {
        Self {
            cached: CachedState::new(state, ne_state)
        }
    }
    pub fn create_ata(&self, params: &CallInterrupt) -> Result<Instruction> {
        let (pda, _) = self.cached.base().pda.external_auth(&params.context.caller);
        let mint = self.cached.mint()?;
        self.create_ata_(&pda, &mint) 
    }
    pub fn create_ata_mint(&self, abi: &[u8], params: &CallInterrupt) -> Result<Instruction> {
        let (mint, _) = get_pubkey(abi)?;
        let (pda, _) = self.cached.base().pda.external_auth(&params.context.caller);
        self.create_ata_(&pda, &mint)
    }
    pub fn create_ata_address(&self, abi: &[u8]) -> Result<Instruction> {
        let (addr, _) = get_h160(abi)?;
        let (pda, _) = self.cached.base().pda.external_auth(&addr);
        let mint = self.cached.mint()?;
        self.create_ata_(&pda, &mint)
    }
    pub fn create_ata_address_mint(&self, abi: &[u8]) -> Result<Instruction> {
        let (addr, right) = get_h160(abi)?;
        let (mint, _) = get_pubkey(right)?;
        let (pda, _) = self.cached.base().pda.external_auth(&addr);
        self.create_ata_(&pda, &mint)
    }
    fn create_ata_(&self, owner: &Pubkey, mint: &Pubkey,) -> Result<Instruction> {
        let spl_program = self.cached.owner(&mint)?;
        let ix = create_associated_token_account_idempotent(
            &self.cached.signer(), &owner, &mint, &spl_program);

        Ok(ix)
    }
}


pub struct ASplCachedWriter<'a, T: Origin> {
    pub cached_mut: CachedMut<'a, T>
}

impl<'a, T: Origin> ASplCachedWriter<'a, T> {
    pub fn new(state: &'a T, nes: &'a mut NonEvmState) -> Self {
        Self {
            cached_mut: CachedMut::new(state, nes)
        }
    }
    pub fn create_idempotent(mut self, ix: &Instruction) -> Result<()> {
        let iter = &mut ix.accounts.iter();
        let _signer = next(iter)?;
        let ata = next(iter)?;
        let owner = next(iter)?;
        let mint = next(iter)?;
        let _ = next(iter)?;
        let spl_program = next(iter)?;

        if spl_program != spl_token_interface::ID && spl_program != spl_token_2022_interface::ID {
            return Err(NonEvmCallError(format!("mint account {} is not owned by spl_token program", mint)))
        }

        let key= ata_(&owner, &mint, &spl_program);
        if key != ata {
            return Err(NonEvmCallError(format!("ata account {} mistmatch  with calculated key {}", ata, key)))
        }

        let ata_acc = self.cached_mut.get_ref(&ata)?;

        if ata_acc.owner == spl_program {
            let data = &ata_acc.data;
            let unpack = StateWithExtensions::<Account2022>::unpack(&data);
            if let Ok(acc_spl) =  unpack {
                if acc_spl.base.owner != owner {
                    return Err(NonEvmCallError(format!("ata {} has invalid spl-owner {}", ata, acc_spl.base.owner)));
                }
                if acc_spl.base.mint != mint {
                    return Err(NonEvmCallError(format!("invalid ata data {}", ata)));
                }
                return Ok(());
            } else {
                return Err(NonEvmCallError(format!("unpack ata.data error: {} {:?}", ata, unpack)))
            }
        }

        if ata_acc.owner != system_program::ID {
            return Err(NonEvmCallError(format!("ata {} has illegal owner {}", ata, ata_acc.owner)))
        }

        let mint_acc = self.cached_mut.state.account(&mint)?;
        // Remap the payload: ata_len is given (owner, data) and can only name the
        // owner, so unremapped it reports the token *program* as the invalid
        // account and sends a reader looking at the wrong thing. The profile()
        // call sites already do this.
        let len = crate::state::mint_profile::ata_len(&mint_acc.owner, &mint_acc.data)
            .map_err(|_| InvalidSplMintAccount(mint))?;
        let rent = Rent::get()?.minimum_balance(len);
        let plan = create_plan(
            &self.cached_mut.state.signer(), &ata, &mint, &owner, &spl_program, len, rent,
        )?;

        let CachedMut {
            nes,
            state
        } = self.cached_mut;

        // Order matters: the account must exist before either initializer, and
        // ImmutableOwner must land before InitializeAccount3 finalizes.
        let system_cached = SystemCached::new(state);
        let spl_cached = SplCached::new(state);
        for ix in &plan {
            if ix.program_id == system_program::ID {
                system_cached.emulate(ix, nes)?;
            } else {
                spl_cached.emulate(ix, nes)?;
            }
        }

        Ok(())
    }
}
/// The instruction sequence the Associated Token Account program issues for a
/// create, in the order it issues them (I7 / DESIGN §4b).
///
/// Pure on purpose: the sequence *is* the contract with the real program, so it
/// is asserted directly, while rent lookup and processor execution stay in the
/// caller. `InitializeImmutableOwner` appears only for Token-2022 — a legacy
/// token account is exactly `Account::LEN` with no TLV region, so there is
/// nowhere to put the extension and the ATA program creates none.
fn create_plan(
    signer: &Pubkey,
    ata: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
    spl_program: &Pubkey,
    len: usize,
    rent: u64,
) -> Result<Vec<Instruction>> {
    let mut plan = vec![solana_system_interface::instruction::create_account(
        signer, ata, rent, len as u64, spl_program,
    )];
    if *spl_program == spl_token_2022_interface::ID {
        plan.push(initialize_immutable_owner(spl_program, ata)?);
    }
    plan.push(initialize_account3(spl_program, ata, mint, owner)?);

    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T22: Pubkey = spl_token_2022_interface::ID;
    const LEGACY: Pubkey = spl_token_interface::ID;
    const USDC_LEGACY: &[u8] = include_bytes!("../../tests/fixtures/usdc_legacy.bin");
    const PYUSD: &[u8] = include_bytes!("../../tests/fixtures/pyusd.bin");

    /// `space` from a System `CreateAccount`: 4-byte enum tag, 8-byte lamports,
    /// then 8-byte space, little-endian.
    fn space_of(data: &[u8]) -> u64 {
        u64::from_le_bytes(data[12..20].try_into().unwrap())
    }

    fn plan_for(owner: &Pubkey, mint_bytes: &[u8]) -> Vec<Instruction> {
        let len = crate::state::mint_profile::ata_len(owner, mint_bytes).unwrap();
        create_plan(
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            owner,
            len,
            1_000_000,
        )
        .unwrap()
    }

    #[test]
    fn legacy_is_two_instructions_at_the_bare_length() {
        let p = plan_for(&LEGACY, USDC_LEGACY);
        assert_eq!(p.len(), 2, "no ImmutableOwner: a legacy account has no TLV region");
        assert_eq!(space_of(&p[0].data), 165);
        assert_eq!(p[1].program_id, LEGACY);
    }

    #[test]
    fn token2022_is_three_instructions_with_immutable_owner_before_init() {
        let p = plan_for(&T22, USDC_LEGACY);
        assert_eq!(p.len(), 3, "the ATA program issues three, not two");
        assert_eq!(space_of(&p[0].data), 170, "plain 2022 = base + ImmutableOwner");
        // InitializeImmutableOwner = 22, InitializeAccount3 = 18. The order is
        // load-bearing: InitializeAccount3 finalizes the account.
        assert_eq!(p[1].data[0], 22, "ImmutableOwner must come second");
        assert_eq!(p[2].data[0], 18, "InitializeAccount3 must come last");
        assert!(p[1..].iter().all(|ix| ix.program_id == T22));
    }

    #[test]
    fn space_tracks_what_the_mint_requires() {
        // The real fixture requires TransferFeeAmount + TransferHookAccount, and
        // the ATA program adds ImmutableOwner on top. A constant would be wrong
        // for at least one of these three.
        assert_eq!(space_of(&plan_for(&T22, PYUSD)[0].data), 187);
        assert_eq!(space_of(&plan_for(&T22, USDC_LEGACY)[0].data), 170);
        assert_eq!(space_of(&plan_for(&LEGACY, USDC_LEGACY)[0].data), 165);
    }

    #[test]
    fn the_created_account_is_owned_by_the_mints_token_program() {
        for (owner, bytes) in [(LEGACY, USDC_LEGACY), (T22, USDC_LEGACY), (T22, PYUSD)] {
            let p = plan_for(&owner, bytes);
            // System CreateAccount's owner field trails space.
            assert_eq!(Pubkey::try_from(&p[0].data[20..52]).unwrap(), owner);
        }
    }
}
