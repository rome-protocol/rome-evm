use {
    crate::{
        error::Result, error::RomeProgramError::NonEvmCallError, origin::Origin,
        non_evm::{
            u64_to_bytes32,  get_h160, get_pubkey, get_u64,
        },
        non_evm_cached::{CachedState, CachedMut,},
        state::{pda::Seed, NonEvmState, aux::checked_transfer, },
        handler::CallInterrupt,
    },
    solana_program::{
        program_error::ProgramError,
        instruction::Instruction, pubkey::Pubkey, account_info::AccountInfo,
    },
    spl_associated_token_account_interface::{
        address::get_associated_token_address_with_program_id as ata
    },
    spl_token::processor::Processor,
    spl_token_2022::processor::Processor as Processor2022,
    std::{
        mem::size_of,
    },
    spl_token_2022_interface::{
        instruction::initialize_account3,
        extension::StateWithExtensions, state::{Account as Account2022},
    },
};

#[repr(C, packed)]
#[derive(Default)]
pub struct SplAccount {
    pub mint: Pubkey,
    pub owner: Pubkey,
    pub amount: [u8; 32],
    pub delegate: Pubkey,
    pub state: [u8; 32],
    pub is_native: [u8; 32],
    pub native_value: [u8; 32],
    pub delegated_amount: [u8; 32],
    pub close_authority: Pubkey,
}

#[derive(Clone)]
pub struct SplCachedReader<'a, T: Origin> {
    pub cached: CachedState<'a, T>,
}

impl<'a, T: Origin> SplCachedReader<'a, T> {
    pub fn new(state: &'a T, ne_state: Option<&'a NonEvmState>) -> Self {
        Self {
            cached: CachedState::new(state, ne_state)
        }
    }
    pub fn transfer(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to_addr, right) = get_h160(abi)?;
        let (tokens, _) = get_u64(right)?;
        let (to, _) = self.cached.base().pda.external_auth(&to_addr);
        let mint= self.cached.mint()?;
        self.transfer_(params, &to, &mint, tokens)
    }
    pub fn transfer_b32(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to, right) = get_pubkey(abi)?;
        let (tokens, _) = get_u64(right)?;
        let mint= self.cached.mint()?;
        self.transfer_(params, &to, &mint, tokens)
    }
    pub fn transfer_mint(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to_addr, right) = get_h160(abi)?;
        let (tokens, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;
        let (to, _) = self.cached.base().pda.external_auth(&to_addr);
        self.transfer_(params, &to, &mint, tokens)
    }
    pub fn transfer_mint_b32(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to, right) = get_pubkey(abi)?;
        let (tokens, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;
        self.transfer_(params, &to, &mint, tokens)
    }
    pub fn transfer_from(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (from_addr, right) = get_h160(abi)?;
        let (to_addr, right) = get_h160(right)?;
        let (tokens, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;
        let profile = self.cached.mint_profile(&mint)?;
        refuse_armed_hook(&profile, &mint)?;
        let spl_program = profile.program;
        let (from, _) = self.cached.base().pda.external_auth(&from_addr);
        let (to, _) = self.cached.base().pda.external_auth(&to_addr);
        let (authority, seed) = self.cached.base().pda.external_auth(&params.context.caller);
        let from_ata = ata(&from, &mint, &spl_program);
        let to_ata = ata(&to, &mint, &spl_program);

        let ix = checked_transfer(&profile, &mint, &from_ata, &to_ata, &authority, &[], tokens)?;

        Ok((ix, vec![seed]))
    }
    pub fn approve(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (spender_addr, right) = get_h160(abi)?;
        let (amount, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;
        let p = self.cached.mint_profile(&mint)?;
        let (spl_program, decimals) = (p.program, p.decimals);
        let (owner, seed) = self.cached.base().pda.external_auth(&params.context.caller);
        let (delegate, _) = self.cached.base().pda.external_auth(&spender_addr);
        let owner_ata = ata(&owner, &mint, &spl_program);

        let ix = spl_token_2022_interface::instruction::approve_checked(
            &spl_program,
            &owner_ata,
            &mint,
            &delegate,
            &owner,
            &[],
            amount,
            decimals,
        )?;

        Ok((ix, vec![seed]))
    }
    pub fn mint(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to_addr, right) = get_h160(abi)?;
        let (amount, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;
        let p = self.cached.mint_profile(&mint)?;
        let (spl_program, decimals) = (p.program, p.decimals);
        let (to, _) = self.cached.base().pda.external_auth(&to_addr);
        let (authority, seed) = self.cached.base().pda.external_auth(&params.context.caller);
        let to_ata = ata(&to, &mint, &spl_program);

        let ix = spl_token_2022_interface::instruction::mint_to_checked(
            &spl_program,
            &mint,
            &to_ata,
            &authority,
            &[],
            amount,
            decimals,
        )?;

        Ok((ix, vec![seed]))
    }
    pub fn init(&self, abi: &[u8]) -> Result<(Instruction, Vec<Seed>)> {
        let (ata, right) = get_pubkey(abi)?;
        let (mint, right) = get_pubkey(right)?;
        let (owner, _) = get_pubkey(right)?;
        let spl_program = self.cached.owner(&mint)?;
        let ix = initialize_account3(&spl_program, &ata, &mint, &owner)?;

        Ok((ix, vec![]))
    }
    fn transfer_(&self, params: &CallInterrupt, to: &Pubkey, mint: &Pubkey, tokens: u64) -> Result<(Instruction, Vec<Seed>)> {
        let profile = self.cached.mint_profile(mint)?;
        refuse_armed_hook(&profile, mint)?;
        let spl_program = profile.program;
        let (from, seed) = self.cached.base().pda.external_auth(&params.context.caller);
        let from_ata= ata(&from, &mint, &spl_program);
        let to_ata= ata(&to, &mint, &spl_program);

        let ix = checked_transfer(&profile, mint, &from_ata, &to_ata, &from, &[], tokens)?;

        Ok((ix, vec![seed]))
    }
    pub fn account(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (addr, _) = get_h160(abi)?;
        let (pda, _) = self.cached.base().pda.external_auth(&addr);
        let mint= self.cached.mint()?;
        let spl_program= self.cached.owner(&mint)?;
        let ata= ata(&pda, &mint, &spl_program);
        self.account_(&ata)
    }
    pub fn account_b32(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (ata, _) = get_pubkey(abi)?;
        self.account_(&ata)
    }
    pub fn account_mint(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (addr, right) = get_h160(abi)?;
        let (mint, _) = get_pubkey(right)?;
        let (pda, _) = self.cached.base().pda.external_auth(&addr);
        let spl_program= self.cached.owner(&mint)?;
        let ata= ata(&pda, &mint, &spl_program);
        self.account_(&ata)
    }

    // Same capability and selector as the legacy home, read through the
    // overlay so a mint mutated earlier in this tx is observed.
    pub fn mint_info(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (mint, _) = get_pubkey(abi)?;
        Ok(self.cached.mint_profile(&mint)?.to_abi())
    }

    fn account_(&self, ata: &Pubkey) -> Result<Vec<u8>> {
        let acc = self.cached.account(&ata)?;

        let mut  vec = vec![0_u8; size_of::<SplAccount>()];
        let ptr = vec.as_mut_ptr().cast::<SplAccount>();
        let dst = unsafe { &mut *ptr };

        let spl = StateWithExtensions::<Account2022>::unpack(&acc.data)?.base;

        dst.mint = spl.mint;
        dst.owner = spl.owner;
        u64_to_bytes32(spl.amount, &mut dst.amount);
        dst.delegate = spl.delegate.unwrap_or_default();
        *dst.state.last_mut().unwrap() = spl.state as u8;
        *dst.is_native.last_mut().unwrap() = spl.is_native.is_some().into();
        u64_to_bytes32(spl.is_native.unwrap_or_default(), &mut dst.native_value);
        u64_to_bytes32(spl.delegated_amount, &mut dst.delegated_amount);
        dst.close_authority = spl.close_authority.unwrap_or_default();

        Ok(vec)
    }
}

pub struct SplCachedWriter<'a,  T: Origin> {
    pub cached_mut: CachedMut<'a,  T>
}

impl<'a,  T: Origin> SplCachedWriter<'a, T> {
    pub fn new(state: &'a T, nes: &'a mut NonEvmState) -> Self {
        Self {
            cached_mut: CachedMut::new(state, nes)
        }
    }
    pub fn transfer_checked(&mut self, ix: &Instruction, amount: u64, decimals: u8) -> Result<()> {
        let mut info = self.cached_mut.infos(ix)?;
        Self::sign_authority(&mut info)?;
        let _ = Processor::process_transfer(&ix.program_id, &info, amount, Some(decimals))?;

        Ok(())
    }
    pub fn transfer2022_checked(&mut self, ix: &Instruction) -> Result<()> {
        let mut info = self.cached_mut.infos(ix)?;
        Self::sign_authority(&mut info)?;
        let _ = Processor2022::process(&ix.program_id, &info, &ix.data)?;

        Ok(())
    }
    pub fn approve_checked(&mut self, ix: &Instruction, amount: u64, decimals: u8) -> Result<()> {
        let mut info = self.cached_mut.infos(ix)?;
        Self::sign_authority(&mut info)?;
        let _ = Processor::process_approve(&ix.program_id, &info, amount, Some(decimals))?;

        Ok(())
    }
    pub fn approve2022_checked(&mut self, ix: &Instruction) -> Result<()> {
        let mut info = self.cached_mut.infos(ix)?;
        Self::sign_authority(&mut info)?;
        let _ = Processor2022::process(&ix.program_id, &info, &ix.data)?;

        Ok(())
    }
    pub fn mint_to_checked(&mut self, ix: &Instruction, amount: u64, decimals: u8) -> Result<()> {
        let mut info = self.cached_mut.infos(ix)?;
        Self::sign_authority(&mut info)?;
        let _ = Processor::process_mint_to(&ix.program_id, &info, amount, Some(decimals))?;

        Ok(())
    }
    pub fn mint_to2022_checked(&mut self, ix: &Instruction) -> Result<()> {
        let mut info = self.cached_mut.infos(ix)?;
        Self::sign_authority(&mut info)?;
        let _ = Processor2022::process(&ix.program_id, &info, &ix.data)?;

        Ok(())
    }
    pub fn init(&mut self, ix: &Instruction, owner: Pubkey) -> Result<()> {
        let info = self.cached_mut.infos(ix)?;
        let _ = Processor::process_initialize_account3(&ix.program_id, &info, owner)?;
        Ok(())
    }
    // The ATA program issues InitializeImmutableOwner before InitializeAccount3
    // for every Token-2022 account it creates, so the overlay must too (I7) —
    // otherwise the staged account lacks an extension the committed one carries.
    // Runs the vendored processor, like every other arm.
    pub fn init_immutable_owner2022(&mut self, ix: &Instruction) -> Result<()> {
        let info = self.cached_mut.infos(ix)?;
        let _ = Processor2022::process_initialize_immutable_owner(&info)?;
        Ok(())
    }
    pub fn init2022(&mut self, ix: &Instruction, owner: &Pubkey) -> Result<()> {
        let info = self.cached_mut.infos(ix)?;
        let _ = Processor2022::process_initialize_account3(&info, owner)?;
        Ok(())
    }
    fn sign_authority (info: &mut [AccountInfo]) -> Result<()> {
        let auth = info
            .last_mut()
            .ok_or(ProgramError::NotEnoughAccountKeys)?; // is not reachable

        auth.is_signer = true;
        Ok(())
    }
}

/// The cached track stages a transfer by running the token processor in-process.
/// For a mint with an ARMED transfer hook that processor reaches `invoke_execute`
/// and a real CPI — inside `emulate_ix`, the one mutating path with no iterative
/// gate — so an iterative transaction would take an irreversible external side
/// effect mid-iteration. The overlay therefore refuses what it cannot reproduce
/// (I7), rather than attempting it and relying on the hook to fail.
///
/// An unarmed hook is inert and must pass: its program_id is zero, so the
/// processor's `get_program_id()` returns `None` and no CPI is reached.
/// Refusing on presence would refuse most real Token-2022 mints.
///
/// `aux::checked_transfer` refuses the same thing as a backstop, for a different
/// reason — a legacy CPI fires without the accounts the hook declares. This one
/// stays because it fails before any ATA is derived and because that reason is
/// specific to running the processor in-process; it is not a duplicate to remove.
fn refuse_armed_hook(profile: &crate::state::mint_profile::MintProfile, mint: &Pubkey) -> Result<()> {
    if let Some(hook) = profile.hook_program {
        return Err(NonEvmCallError(format!(
            "mint {mint} has an armed transfer hook {hook}; \
             hooked transfers are not supported on the cached track"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::mint_profile::profile;

    const T22: Pubkey = spl_token_2022_interface::ID;
    const LEGACY: Pubkey = spl_token_interface::ID;
    const USDC_LEGACY: &[u8] = include_bytes!("../../tests/fixtures/usdc_legacy.bin");
    const PYUSD: &[u8] = include_bytes!("../../tests/fixtures/pyusd.bin");

    fn armed_hook_bytes(program: Pubkey) -> Vec<u8> {
        // Walk the TLV to the TransferHook payload and arm its program_id.
        let mut d = PYUSD.to_vec();
        let mut i = 166;
        loop {
            let t = u16::from_le_bytes([d[i], d[i + 1]]);
            let len = u16::from_le_bytes([d[i + 2], d[i + 3]]) as usize;
            if t == spl_token_2022::extension::ExtensionType::TransferHook as u16 {
                let at = i + 4 + 32; // authority, then program_id
                d[at..at + 32].copy_from_slice(program.as_ref());
                return d;
            }
            i += 4 + len;
        }
    }

    #[test]
    fn legacy_and_plain_2022_pass() {
        assert!(refuse_armed_hook(&profile(&LEGACY, USDC_LEGACY).unwrap(), &Pubkey::new_unique()).is_ok());
        assert!(refuse_armed_hook(&profile(&T22, USDC_LEGACY).unwrap(), &Pubkey::new_unique()).is_ok());
    }

    #[test]
    fn a_present_but_unarmed_hook_passes() {
        // The fixture carries a TransferHook whose program_id is zero. Its
        // processor reaches no CPI, so refusing it would refuse a mint that
        // transfers perfectly well — and most real 2022 mints look like this.
        assert!(refuse_armed_hook(&profile(&T22, PYUSD).unwrap(), &Pubkey::new_unique()).is_ok());
    }

    #[test]
    fn an_armed_hook_is_refused() {
        let hook = Pubkey::new_unique();
        let p = profile(&T22, &armed_hook_bytes(hook)).unwrap();
        let err = refuse_armed_hook(&p, &Pubkey::new_unique()).unwrap_err();
        assert!(
            format!("{err:?}").contains(&hook.to_string()),
            "the error must name the hook so the failure is diagnosable, got {err:?}"
        );
    }
}
