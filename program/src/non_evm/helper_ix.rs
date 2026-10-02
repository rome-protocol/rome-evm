use {
    solana_program::{
        instruction::Instruction, rent::Rent, sysvar::Sysvar, pubkey::Pubkey,
    },
    solana_system_interface::{
        instruction::{
            create_account, transfer as system_transfer
        },
        program as system_program,
    },
    crate::{
        error::Result, origin::Origin, error::RomeProgramError::*,
        H160, pda::Seed, non_evm::aux::{get_h160, get_u8_unchecked, len_ge}, handler::CallInterrupt,
        state::aux::{checked_transfer, mint_owner_decimals, mint_profile}, wei_to_u64, Diff,
        state::mint_profile::MintProfile,
    },
    super::{
        get_pubkey, get_u64, get_u256, IxList,
    },
    spl_associated_token_account_interface::{
        instruction::create_associated_token_account_idempotent,
        address::get_associated_token_address_with_program_id as ata
    },
};

pub struct HelperProgram<'a, T: Origin> {
    pub state: &'a T,
}

impl<'a, T: Origin> HelperProgram<'a, T> {
    pub const ADDRESS: H160 = H160([
        0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x09,
    ]);
    pub fn new(state: &'a T) -> Self {
        Self {
            state
        }
    }
    pub fn create_ata(&self, abi: &[u8]) -> Result<(Instruction, Vec<Seed>)> {
        let (user, _) = get_h160(abi)?;
        let mint = self.mint_internal()?;
        self.create_ata_internal(&user, &mint)
    }

    pub fn create_ata_with_mint(&self, abi: &[u8]) -> Result<(Instruction, Vec<Seed>)> {
        let (user, right) = get_h160(abi)?;
        let (mint, _) = get_pubkey(right)?;
        self.create_ata_internal(&user, &mint)
    }

    pub fn create_pda(&self, abi: &[u8]) -> Result<(Instruction, Vec<Seed>)> {
        let (user, _) = get_h160(abi)?;
        self.create_pda_internal(&user, None)
    }

    pub fn create_pda_with_lamports(&self, abi: &[u8]) -> Result<(Instruction, Vec<Seed>)> {
        let (user, right) = get_h160(abi)?;
        let (lamports, _) = get_u64(right)?;
        self.create_pda_internal(&user, Some(lamports))
    }

    pub fn swap_gas_to_lamports(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (lamports, _) = get_u64(abi)?;
        let (pda, _) = self.state.base().pda.external_auth(&params.context.caller);
        let ix = system_transfer(&self.state.signer(), &pda, lamports);
        Ok((ix, vec![]))
    }

    pub fn transfer_lamports(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to_addr, right) = get_h160(abi)?;
        let (lamports, _) = get_u64(right)?;

        let (from, seed) = self.state.base().pda.external_auth(&params.context.caller);
        let acc = self.state.account(&from)?;
        let rent = Rent::get()?.minimum_balance(acc.data.len());

        if acc.lamports < lamports.checked_add(rent).ok_or(CalculationOverflow)? {
            return Err(NonEvmCallError(format!("transfer_lamports(): insufficient balance of external_pda {}", acc.lamports)))
        }

        let (to, _) = self.state.base().pda.external_auth(&to_addr);
        let ix = system_transfer(&from, &to, lamports);

        Ok((ix, vec![seed]))
    }

    pub fn transfer_spl(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to_addr, right) = get_h160(abi)?;
        let (tokens, _) = get_u64(right)?;
        let mint = self.mint_internal()?;
        self.transfer_spl_internal(tokens, &mint, &to_addr, params)
    }

    pub fn transfer_spl_to_ata(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to_ata, right) = get_pubkey(abi)?;
        let (tokens, _) = get_u64(right)?;
        let mint = self.mint_internal()?;
        let profile = mint_profile(self.state, &mint)?;

        self.transfer_spl_token_internal_(tokens, &mint, &to_ata, params, &profile)
    }

    pub fn transfer_spl_with_mint(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to_addr, right) = get_h160(abi)?;
        let (tokens, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;

        self.transfer_spl_internal(tokens, &mint, &to_addr, params)
    }

    pub fn transfer_spl_with_mint_to_ata(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to_ata, right) = get_pubkey(abi)?;
        let (tokens, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;
        let profile = mint_profile(self.state, &mint)?;

        self.transfer_spl_token_internal_(tokens, &mint, &to_ata, params, &profile)
    }

    /// `transfer_spl(bytes32 src_ata, bytes32 to_ata, uint64 tokens, bytes32 mint)`
    ///
    /// Delegate variant: source ATA is supplied by the caller (vs. derived
    /// from caller's external_auth PDA in the other overloads). Signs as
    /// `external_auth(caller)` — SPL Token Program accepts this PDA as the
    /// transfer_checked authority when it is either the source ATA's owner
    /// OR its delegate with `delegated_amount >= tokens`. This is the
    /// replacement for the removed `spl_transfer_checked_v1` CPI shortcut
    /// and is what `SPL_ERC20.transferFrom` relies on.
    pub fn transfer_spl_from_ata(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (from_ata, right) = get_pubkey(abi)?;
        let (to_ata, right) = get_pubkey(right)?;
        let (tokens, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;
        let profile = mint_profile(self.state, &mint)?;

        let from_ata_acc = self.state.account(&from_ata)?;
        if from_ata_acc.owner != spl_token_interface::ID && from_ata_acc.owner != spl_token_2022_interface::ID {
            return Err(NonEvmCallError(format!("source ata {} is not owned by SPL-program", from_ata)));
        }
        let to_ata_acc = self.state.account(&to_ata)?;
        if to_ata_acc.owner != spl_token_interface::ID && to_ata_acc.owner != spl_token_2022_interface::ID {
            return Err(NonEvmCallError(format!("destination ata {} is not owned by SPL-program", to_ata)));
        }

        let (authority, seed) = self.state.base().pda.external_auth(&params.context.caller);
        let ix = checked_transfer(&profile, &mint, &from_ata, &to_ata, &authority, &[], tokens)?;

        Ok((ix, vec![seed]))
    }

    pub fn pda(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (addr, _) = get_h160(abi)?;
        let (pda, _) = self.state.base().pda.external_auth(&addr);
        Ok(pda.as_ref().to_vec())
    }

    /// pda_with_salt(address user, bytes32 salt) → bytes32
    /// Derives `find_program_address([EXTERNAL_AUTHORITY, user, salt], rome_program_id)`.
    /// Single-dispatch replacement for the Solidity-side 2-call composition
    /// `rome_evm_program_id() + find_program_address(seeds)`.
    /// EthCall — no Solana account read, just one find_program_address syscall.
    pub fn pda_with_salt(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (addr, rest) = get_h160(abi)?;
        len_ge!(rest, 32);
        let salt = &rest[..32];
        let (pda, _) = self.state.base().pda.external_auth_with_salt(&addr, salt);
        Ok(pda.as_ref().to_vec())
    }

    pub fn ata_with_mint(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (addr, right) = get_h160(abi)?;
        let (mint, _) = get_pubkey(right)?;
        self.ata_internal(&addr, &mint)
    }

    pub fn ata(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (addr, _) = get_h160(abi)?;
        let mint = self.mint_internal()?;
        self.ata_internal(&addr, &mint)
    }

    pub fn deposit_from_ata(&self, abi: &[u8], params: &CallInterrupt) -> Result<IxList> {
        let mint = self.mint_internal()?;
        let profile = mint_profile(self.state, &mint)?;
        let wei = get_u256(abi)?;
        let tokens = wei_to_u64(wei, profile.decimals)?;

        let from = params.context.caller;
        let (from_pda, seed) = self.state.base().pda.external_auth(&from);
        let from_ata: Pubkey = ata(&from_pda, &mint, &profile.program);

        let (to_pda, _) = self.state.base().pda.sol_wallet();
        let to_ata: Pubkey = ata(&to_pda, &mint, &profile.program);

        let ix: Instruction =
            checked_transfer(&profile, &mint, &from_ata, &to_ata, &from_pda, &[], tokens)?;
        let ixs = vec![(ix, vec![seed])];
        let diff = vec![(params.context.caller, Diff::TransferTo {balance: wei})];

        let list = IxList{
            ixs,
            diff,
        };

        Ok(list)
    }
    pub fn transfer_spl_to_signer(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (tokens, right) = get_u64(abi)?;
        let (mint, _) = get_pubkey(right)?;
        let profile = mint_profile(self.state, &mint)?;
        let to = self.state.signer();
        let to_ata: Pubkey = ata(&to, &mint, &profile.program);

        self.transfer_spl_token_internal_(tokens, &mint, &to_ata, params, &profile)
    }

    fn ata_internal(&self, addr: &H160, mint: &Pubkey) -> Result<Vec<u8>> {
        let (pda, _) = self.state.base().pda.external_auth(addr);
        let spl_program = self.spl_program_internal(mint)?;
        let ata = ata(&pda, &mint, &spl_program);
        Ok(ata.as_ref().to_vec())
    }

    fn create_ata_internal(&self, user: &H160, mint: &Pubkey) -> Result<(Instruction, Vec<Seed>)> {
        let spl_program = self.spl_program_internal(mint)?;
        let (ext_pda, _) = self.state.base().pda.external_auth(user);
        let ix = create_associated_token_account_idempotent(&self.state.signer(), &ext_pda, mint, &spl_program);

        Ok((ix, vec![]))
    }

    /// create_ata_for_key(bytes32 wallet, bytes32 mint)
    /// Idempotent ATA-create for an arbitrary raw Solana pubkey as owner.
    /// Operator pays rent (signer = state.signer()). The owner is NOT
    /// derived via external_auth — it is the caller-supplied pubkey verbatim.
    /// Unblocks rome-solidity bridgeOutToSolana / ensureRecipientAta /
    /// create_token_account raw-pubkey-recipient flows.
    pub fn create_ata_for_key(&self, abi: &[u8]) -> Result<(Instruction, Vec<Seed>)> {
        let (wallet, rest) = get_pubkey(abi)?;
        let (mint, _) = get_pubkey(rest)?;
        let spl_program = self.spl_program_internal(&mint)?;
        let ix = create_associated_token_account_idempotent(
            &self.state.signer(),
            &wallet,
            &mint,
            &spl_program,
        );
        Ok((ix, vec![]))
    }

    fn create_pda_internal(&self, user: &H160, lamports: Option<u64>) -> Result<(Instruction, Vec<Seed>)> {
        let (pda, seed) = self.state.base().pda.external_auth(user);

        let rent = Rent::get()?.minimum_balance(0);
        if let Some(lamports_) = lamports {
            if lamports_ < rent {
                return Err(NonEvmCallError(format!("create_pda(): specified number of lamports less the Rent exemption {}", lamports_)))
            }
        }
        let ix = create_account(
            &self.state.signer(),
            &pda,
            lamports.unwrap_or(rent),
            0,
            &system_program::ID,
        );

        Ok((ix, vec![seed]))
    }

    fn transfer_spl_internal(
        &self,
        tokens: u64,
        mint: &Pubkey,
        to_addr: &H160,
        params: &CallInterrupt,
    ) -> Result<(Instruction, Vec<Seed>)> {
        let profile = mint_profile(self.state, mint)?;
        let (to, _) = self.state.base().pda.external_auth(to_addr);
        let to_ata= ata(&to, mint, &profile.program);

        self.transfer_spl_token_internal_(tokens, mint, &to_ata, params, &profile)
    }

    fn transfer_spl_token_internal_(
        &self,
        tokens: u64,
        mint: &Pubkey,
        to_ata: &Pubkey,
        params: &CallInterrupt,
        profile: &MintProfile,
    ) -> Result<(Instruction, Vec<Seed>)> {
        let (from, seed) = self.state.base().pda.external_auth(&params.context.caller);
        let from_ata = ata(&from, mint, &profile.program);
        let to_ata_acc = self.state.account(&to_ata)?;

        if to_ata_acc.owner != spl_token_interface::ID && to_ata_acc.owner != spl_token_2022_interface::ID {
            return Err(NonEvmCallError(format!("destination ata {} is not owned by SPL-program", to_ata)));
        }

        let ix = checked_transfer(profile, mint, &from_ata, to_ata, &from, &[], tokens)?;

        Ok((ix, vec![seed]))
    }

    // approve_spl — caller-PDA-as-owner sets delegate_pda(spender) on the
    // owner-ATA. Replaces v1's `SplTokenLib.approve` + raw
    // `delegatecall(CpiProgram.invoke_signed)`. Two-level shape mirrors
    // `transfer_spl_with_mint_to_ata` family.
    pub fn approve_spl(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (spender_addr, right) = get_h160(abi)?;
        let (amount, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;
        let (spl_program, decimals) = mint_owner_decimals(self.state, &mint)?;

        self.approve_spl_token_internal_(amount, &mint, &spender_addr, params, &spl_program, decimals)
    }

    /// create_mint_account(bytes32 salt)
    /// Allocates a salt-derived SPL Mint account via System CreateAccount CPI.
    /// Funder = caller's external_auth PDA; new account = external_auth_with_salt(caller, salt).
    /// Space = 82 (SPL_MINT_LEN), owner = SPL Token, lamports = rent-floor for 82 bytes.
    /// Two PDA signers: funder + new account.
    pub fn create_mint_account(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (salt, _) = get_pubkey(abi)?;
        let caller = params.context.caller;
        let (from_pda, from_seed) = self.state.base().pda.external_auth(&caller);
        let (mint_pda, mint_seed) = self.state.base().pda.external_auth_with_salt(&caller, salt.as_ref());

        const SPL_MINT_LEN: u64 = 82;
        let rent = Rent::get()?.minimum_balance(SPL_MINT_LEN as usize);

        let ix = create_account(
            &from_pda,
            &mint_pda,
            rent,
            SPL_MINT_LEN,
            &spl_token_interface::ID,
        );

        Ok((ix, vec![from_seed, mint_seed]))
    }

    /// init_spl_mint(bytes32 mint, uint8 decimals, bytes32 mint_authority,
    ///                bool has_freeze_authority, bytes32 freeze_authority)
    /// SPL Token InitializeMint2 CPI against a pre-allocated mint account.
    /// No PDA signer needed — mint_authority/freeze_authority are stored as
    /// account data, not runtime signers. Mint account must be writable.
    pub fn init_spl_mint(&self, abi: &[u8]) -> Result<(Instruction, Vec<Seed>)> {
        let (mint, right) = get_pubkey(abi)?;
        len_ge!(right, 32);
        let decimals = get_u8_unchecked(&right[..32])?;
        let right = &right[32..];
        let (mint_authority, right) = get_pubkey(right)?;
        len_ge!(right, 32);
        let has_freeze = get_u8_unchecked(&right[..32])? != 0;
        let right = &right[32..];
        let (freeze_authority, _) = get_pubkey(right)?;

        let freeze_opt = if has_freeze { Some(&freeze_authority) } else { None };

        let ix = spl_token_2022_interface::instruction::initialize_mint2(
            &spl_token_interface::ID,
            &mint,
            &mint_authority,
            freeze_opt,
            decimals,
        )?;

        Ok((ix, vec![]))
    }

    /// create_and_init_mint(uint8 decimals, bytes32 mint_authority,
    ///                       bool has_freeze_authority, bytes32 freeze_authority,
    ///                       bytes32 salt) → bytes32 mint
    /// Composed: System CreateAccount + SPL InitializeMint2 in one dispatch.
    /// Saves one Rome DoTx envelope + one Solidity delegatecall vs calling
    /// create_mint_account + init_spl_mint separately.
    pub fn create_and_init_mint(&self, abi: &[u8], params: &CallInterrupt) -> Result<IxList> {
        len_ge!(abi, 32);
        let decimals = get_u8_unchecked(&abi[..32])?;
        let right = &abi[32..];
        let (mint_authority, right) = get_pubkey(right)?;
        len_ge!(right, 32);
        let has_freeze = get_u8_unchecked(&right[..32])? != 0;
        let right = &right[32..];
        let (freeze_authority, right) = get_pubkey(right)?;
        let (salt, _) = get_pubkey(right)?;

        let caller = params.context.caller;
        let (from_pda, from_seed) = self.state.base().pda.external_auth(&caller);
        let (mint_pda, mint_seed) = self.state.base().pda.external_auth_with_salt(&caller, salt.as_ref());

        // The rent is paid by the caller's PDA, so the authorities written into
        // the mint have to be that same PDA. Under DELEGATECALL the caller is the
        // outer user, and authorities read from the ABI would let the delegated
        // code take control of a mint the caller funded.
        if mint_authority != from_pda {
            return Err(NonEvmCallError(
                "create_and_init_mint(): mint_authority must be the caller's external-authority PDA".to_string(),
            ));
        }
        if has_freeze && freeze_authority != from_pda {
            return Err(NonEvmCallError(
                "create_and_init_mint(): freeze_authority must be the caller's external-authority PDA".to_string(),
            ));
        }

        const SPL_MINT_LEN: u64 = 82;
        let rent = Rent::get()?.minimum_balance(SPL_MINT_LEN as usize);

        let create_ix = create_account(
            &from_pda,
            &mint_pda,
            rent,
            SPL_MINT_LEN,
            &spl_token_interface::ID,
        );

        let freeze_opt = if has_freeze { Some(&freeze_authority) } else { None };
        let init_ix = spl_token_2022_interface::instruction::initialize_mint2(
            &spl_token_interface::ID,
            &mint_pda,
            &mint_authority,
            freeze_opt,
            decimals,
        )?;

        Ok(IxList {
            ixs: vec![
                (create_ix, vec![from_seed, mint_seed]),
                (init_ix, vec![]),
            ],
            diff: vec![],
        })
    }

    /// approve_spl_raw_delegate(bytes32 src_ata, bytes32 delegate, uint64 amount,
    ///                          bytes32 mint, uint8 decimals)
    /// SPL `approve_checked` with caller-supplied raw-pubkey delegate (not
    /// derived via external_auth). Used by RomeBridgeWithdraw.approveBurnETH
    /// where the delegate is the Wormhole Token Bridge authority_signer PDA —
    /// a raw Solana pubkey with no EVM-address equivalent.
    ///
    /// Caller passes `decimals` to skip the on-chain mint read (~30-50K CU
    /// saving vs reading via mint_owner_decimals). SPL Token runtime validates
    /// decimals match — wrong value reverts the CPI (no Rust boundary check
    /// per CU-conscious rule).
    ///
    /// spl_program hardcoded to SPL Token. Token-2022 raw-delegate flows are
    /// a future selector if/when they emerge.
    ///
    /// Signer = external_auth(caller). Caller must be the on-chain ATA owner
    /// OR a valid delegate; SPL runtime catches mismatch.
    pub fn approve_spl_raw_delegate(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (src_ata, right) = get_pubkey(abi)?;
        let (delegate, right) = get_pubkey(right)?;
        let (amount, right) = get_u64(right)?;
        let (mint, right) = get_pubkey(right)?;
        len_ge!(right, 32);
        let decimals = get_u8_unchecked(&right[..32])?;

        let (owner_pda, seed) = self.state.base().pda.external_auth(&params.context.caller);
        let spl_program = spl_token_interface::ID;

        let ix = spl_token_2022_interface::instruction::approve_checked(
            &spl_program,
            &src_ata,
            &mint,
            &delegate,
            &owner_pda,
            &[],
            amount,
            decimals,
        )?;

        Ok((ix, vec![seed]))
    }

    fn approve_spl_token_internal_(
        &self,
        amount: u64,
        mint: &Pubkey,
        spender_addr: &H160,
        params: &CallInterrupt,
        spl_program: &Pubkey,
        decimals: u8,
    ) -> Result<(Instruction, Vec<Seed>)> {
        let (owner_pda, seed) = self.state.base().pda.external_auth(&params.context.caller);
        let owner_ata = ata(&owner_pda, mint, spl_program);
        let (delegate_pda, _) = self.state.base().pda.external_auth(spender_addr);

        let ix = spl_token_2022_interface::instruction::approve_checked(
            spl_program,
            &owner_ata,
            mint,
            &delegate_pda,
            &owner_pda,
            &[],
            amount,
            decimals,
        )?;

        Ok((ix, vec![seed]))
    }

    // mint_spl — caller-PDA signs as the mint authority. SPL Token runtime
    // enforces caller-PDA == on-chain mint authority; no Rust boundary check.
    pub fn mint_spl(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to_addr, right) = get_h160(abi)?;
        let (amount, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;
        let (spl_program, decimals) = mint_owner_decimals(self.state, &mint)?;

        self.mint_spl_token_internal_(amount, &mint, &to_addr, params, &spl_program, decimals)
    }

    fn mint_spl_token_internal_(
        &self,
        amount: u64,
        mint: &Pubkey,
        to_addr: &H160,
        params: &CallInterrupt,
        spl_program: &Pubkey,
        decimals: u8,
    ) -> Result<(Instruction, Vec<Seed>)> {
        let (authority_pda, seed) = self.state.base().pda.external_auth(&params.context.caller);
        let (to_pda, _) = self.state.base().pda.external_auth(to_addr);
        let to_ata = ata(&to_pda, mint, spl_program);

        let ix = spl_token_2022_interface::instruction::mint_to_checked(
            spl_program,
            mint,
            &to_ata,
            &authority_pda,
            &[],
            amount,
            decimals,
        )?;

        Ok((ix, vec![seed]))
    }

    // transfer_spl(from, to, amount, mint[, token_program]) — addr-keyed
    // delegate variant. Source ATA derived from `from`; dest ATA from `to`.
    // Signs as `external_auth(caller)`; SPL Token accepts the PDA as
    // transfer_checked authority when it is either the source ATA's owner
    // OR delegate with delegated_amount >= amount. Distinct from existing
    // 0x766b362a (transfer_spl_from_ata) which takes pre-derived ATAs.
    //
    // Caller contract (SPL-delegate-model semantics, NOT enforced at the
    // Rust boundary — SPL Token runtime catches violations):
    // - When `caller == from_addr`: authority IS the source ATA's owner →
    //   succeeds unconditionally (owner-driven self-transfer).
    // - When `caller != from_addr`: caller MUST have been previously
    //   approved as delegate on `from_addr`'s ATA for this mint, via
    //   `approve_spl(spender=caller, amount, mint)` called by `from_addr`.
    //   SPL `approve_checked` stores `external_auth(spender)` as the
    //   delegate; this selector signs as `external_auth(caller)`; the SPL
    //   runtime compares them and revokes if mismatched.
    // Same pattern as existing 0x766b362a — proven in production via
    // v1 SPL_ERC20.transferFrom.
    pub fn transfer_spl_from_to(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (from_addr, right) = get_h160(abi)?;
        let (to_addr, right) = get_h160(right)?;
        let (amount, right) = get_u64(right)?;
        let (mint, _) = get_pubkey(right)?;
        let profile = mint_profile(self.state, &mint)?;

        self.transfer_spl_from_to_token_internal_(amount, &mint, &from_addr, &to_addr, params, &profile)
    }

    fn transfer_spl_from_to_token_internal_(
        &self,
        amount: u64,
        mint: &Pubkey,
        from_addr: &H160,
        to_addr: &H160,
        params: &CallInterrupt,
        profile: &MintProfile,
    ) -> Result<(Instruction, Vec<Seed>)> {
        let (from_pda, _) = self.state.base().pda.external_auth(from_addr);
        let from_ata = ata(&from_pda, mint, &profile.program);
        let (to_pda, _) = self.state.base().pda.external_auth(to_addr);
        let to_ata = ata(&to_pda, mint, &profile.program);
        let (authority_pda, seed) = self.state.base().pda.external_auth(&params.context.caller);

        let ix =
            checked_transfer(profile, mint, &from_ata, &to_ata, &authority_pda, &[], amount)?;

        Ok((ix, vec![seed]))
    }

    // user_balance(addr, mint) → uint64
    // Reads SPL TokenAccount.amount (u64 LE at offset 64) on the user's ATA.
    //
    // Caller contract:
    // - Caller (SDK / EVM tx builder) MUST include the derived ATA in the
    //   tx accounts list. If the ATA is omitted, `state.account()`
    //   propagates `AccountNotFound` rather than returning 0. This matches
    //   the existing `CpiProgram.account_u64_at` convention (see
    //   non_evm/account_data_extras.rs).
    // - When the ATA is included but does not exist on-chain, Solana
    //   provides a zero-lamport placeholder; `lamports == 0` short-circuits
    //   to 0 (fresh-chain probe pattern, matches v1 `SPL_ERC20.balanceOf`).
    // - Mint must exist; propagates `AccountNotFound` if not.
    //
    // Layout: SPL TokenAccount LEN=165, amount at offset 64..72. The guard
    // `data.len() < 165` rejects partially-allocated accounts that the SPL
    // runtime would otherwise reject — defense for malformed account
    // collisions; not load-bearing for legitimate ATAs (always 165 bytes).
    // mint_info(bytes32) -> (bytes32 tokenProgram, uint8 decimals,
    //                        bytes32 hookProgram, uint16 feeBps, uint32 extensions)
    //
    // hookProgram and feeBps report the ARMED state — zero when the extension
    // is absent OR present-but-inert. `extensions` reports mere presence, one
    // bit per ExtensionType discriminant, for a caller that wants to refuse a
    // family outright.
    pub fn mint_info(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (mint, _) = get_pubkey(abi)?;
        let acc = self.state.account(&mint)?;

        if acc.lamports == 0 {
            return Err(InvalidSplMintAccount(mint));
        }
        let p = crate::state::mint_profile::profile(&acc.owner, &acc.data)
            .map_err(|_| InvalidSplMintAccount(mint))?;

        Ok(p.to_abi())
    }

    pub fn user_balance(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (account_addr, right) = get_h160(abi)?;
        let (mint, _) = get_pubkey(right)?;

        let (pda, _) = self.state.base().pda.external_auth(&account_addr);
        let spl_program = self.state.account(&mint)?.owner;
        let user_ata = ata(&pda, &mint, &spl_program);

        let acc = self.state.account(&user_ata)?;
        let amount = if acc.lamports == 0 || acc.data.len() < 165 {
            0u64
        } else {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&acc.data[64..72]);
            u64::from_le_bytes(buf)
        };

        let mut out = [0u8; 32];
        out[24..32].copy_from_slice(&amount.to_be_bytes());
        Ok(out.to_vec())
    }

    // allowance_of(owner, spender, mint) → uint64
    // Reads owner-ATA delegate fields. HARD REQ: returns 0 unless
    // `delegate == external_auth(spender)`. Without this check, routers
    // observe non-zero allowance for one spender while another is the
    // actual on-chain delegate, then transferFrom reverts at runtime —
    // ERC-20 semantic violation. The check is NOT redundant with runtime:
    // SPL Token's transfer_checked only catches "delegate mismatch" AT
    // transfer time; ERC-20 allowance() is a pre-transfer probe.
    //
    // Caller contract (same as user_balance):
    // - Caller MUST include the derived owner-ATA in the tx accounts list;
    //   missing account propagates AccountNotFound, not 0.
    // - When ATA is included but does not exist on-chain, lamports == 0
    //   short-circuits to 0.
    //
    // SPL TokenAccount layout (LEN=165):
    //   72..76:   delegate tag (u32 LE; 0=None, 1=Some)
    //   76..108:  delegate pubkey (when tag==1)
    //   121..129: delegated_amount (u64 LE)
    // Guard `data.len() < 165` rejects malformed accounts (Token-2022-compat
    // since extensions begin at byte 165).
    pub fn allowance_of(&self, abi: &[u8]) -> Result<Vec<u8>> {
        let (owner_addr, right) = get_h160(abi)?;
        let (spender_addr, right) = get_h160(right)?;
        let (mint, _) = get_pubkey(right)?;

        let (owner_pda, _) = self.state.base().pda.external_auth(&owner_addr);
        let (spender_pda, _) = self.state.base().pda.external_auth(&spender_addr);
        let spl_program = self.state.account(&mint)?.owner;
        let owner_ata = ata(&owner_pda, &mint, &spl_program);

        let acc = self.state.account(&owner_ata)?;
        let amount = if acc.lamports == 0 || acc.data.len() < 165 {
            0u64
        } else {
            let tag = u32::from_le_bytes([acc.data[72], acc.data[73], acc.data[74], acc.data[75]]);
            if tag != 1 {
                0u64
            } else {
                let mut delegate_bytes = [0u8; 32];
                delegate_bytes.copy_from_slice(&acc.data[76..108]);
                if Pubkey::from(delegate_bytes) != spender_pda {
                    0u64
                } else {
                    let mut buf = [0u8; 8];
                    buf.copy_from_slice(&acc.data[121..129]);
                    u64::from_le_bytes(buf)
                }
            }
        };

        let mut out = [0u8; 32];
        out[24..32].copy_from_slice(&amount.to_be_bytes());
        Ok(out.to_vec())
    }

    fn mint_internal(&self) -> Result<Pubkey> {
        let mint = self.state.base().owner_info().mint;
        mint.ok_or(NonEvmCallError("solidify function is only applicable for a rollup based on SPL".to_string()))
    }
    fn spl_program_internal(&self, mint: &Pubkey) -> Result<Pubkey> {
        let spl_program = self.state.account(mint)?.owner;

        if spl_program != spl_token_interface::ID && spl_program != spl_token_2022_interface::ID {
            return Err(NonEvmCallError(format!("solidify function uses mint {} that is not owned by SPL-program", mint)));
        }

        Ok(spl_program)
    }
 }

#[cfg(test)]
mod create_and_init_mint_authority_tests {
    use {
        super::*,
        crate::{
            context::AccountLock, state::base::Base, state::Account as RomeAccount, Allocate, U256,
            non_evm_cached::test_support::install_rent,
        },
        evm::Context,
        solana_program::{account_info::AccountInfo, pubkey::Pubkey},
    };

    const PROGRAM_ID: Pubkey = Pubkey::new_from_array([7u8; 32]);

    struct TestOrigin<'a> {
        base: Base<'a>,
        signer: Pubkey,
    }

    impl<'a> TestOrigin<'a> {
        fn new() -> Self {
            install_rent();
            Self { base: Base::new(&PROGRAM_ID, 1, None), signer: Pubkey::new_unique() }
        }
    }

    impl Origin for TestOrigin<'_> {
        fn nonce(&self, _: &H160) -> Result<Option<u64>> { unimplemented!() }
        fn balance(&self, _: &H160) -> Result<Option<U256>> { unimplemented!() }
        fn code(&self, _: &H160) -> Result<Option<Vec<u8>>> { unimplemented!() }
        fn valids(&self, _: &H160) -> Result<Option<Vec<u8>>> { unimplemented!() }
        fn storage(&self, _: &H160, _: &U256) -> Result<Option<U256>> { unimplemented!() }
        fn inc_nonce<L: AccountLock>(&self, _: &H160, _: &L) -> Result<()> { unimplemented!() }
        fn add_balance<L: AccountLock>(&self, _: &H160, _: &U256, _: &L) -> Result<()> { unimplemented!() }
        fn sub_balance<L: AccountLock>(&self, _: &H160, _: &U256, _: &L) -> Result<()> { unimplemented!() }
        fn set_code<L: AccountLock>(&self, _: &H160, _: &[u8], _: &[u8], _: &L) -> Result<()> { unimplemented!() }
        fn set_storage<L: AccountLock>(&self, _: &H160, _: &U256, _: &U256, _: &L) -> Result<()> { unimplemented!() }
        fn base(&self) -> &Base<'_> { &self.base }
        fn account(&self, _: &Pubkey) -> Result<RomeAccount> { unimplemented!() }
        fn with_account_info<F, R>(&self, _: &Pubkey, _: F) -> Result<R>
        where F: FnOnce(&AccountInfo) -> Result<R> { unimplemented!() }
        fn invoke_signed(&self, _: &Instruction, _: Vec<Seed>, _: bool) -> Result<()> { unimplemented!() }
        fn invoke_signed_unchecked(&self, _: &Instruction, _: Vec<Seed>) -> Result<()> { unimplemented!() }
        fn signer(&self) -> Pubkey { self.signer }
        fn wallet(&self) -> Result<Pubkey> { unimplemented!() }
        fn treasure(&self, _: u64) -> Result<Pubkey> { unimplemented!() }
        fn owner(&self, _: &Pubkey) -> Result<Pubkey> { unimplemented!() }
        fn ed25519_data(&self) -> Result<Vec<u8>> { unimplemented!() }
    }

    impl Allocate for TestOrigin<'_> {
        fn alloc_balance<L: AccountLock>(&self, _: &H160, _: &L) -> Result<()> { unimplemented!() }
        fn alloc_slots<L: AccountLock>(&self, _: &Pubkey, _: &Seed, _: usize, _: &L, _: &H160) -> Result<bool> { unimplemented!() }
        fn alloc_slots_unchecked(&self, _: &Pubkey, _: &Seed, _: usize, _: &H160) -> Result<()> { unimplemented!() }
        fn alloc_contract<L: AccountLock>(&self, _: &H160, _: &[u8], _: &[u8], _: &L) -> Result<bool> { unimplemented!() }
    }

    const CALLER: H160 = H160([0xca; 20]);

    fn word(bytes: &[u8]) -> [u8; 32] {
        let mut w = [0u8; 32];
        w[32 - bytes.len()..].copy_from_slice(bytes);
        w
    }

    /// abi = decimals | mint_authority | has_freeze | freeze_authority | salt
    fn abi(mint_authority: &Pubkey, has_freeze: bool, freeze_authority: &Pubkey) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&word(&[6]));
        v.extend_from_slice(mint_authority.as_ref());
        v.extend_from_slice(&word(&[u8::from(has_freeze)]));
        v.extend_from_slice(freeze_authority.as_ref());
        v.extend_from_slice(&word(&[0x01]));
        v
    }

    fn call(input: Vec<u8>) -> CallInterrupt {
        CallInterrupt {
            code_address: HelperProgram::<TestOrigin>::ADDRESS,
            transfer: None,
            input,
            is_static: false,
            context: Context {
                address: HelperProgram::<TestOrigin>::ADDRESS,
                caller: CALLER,
                apparent_value: U256::zero(),
            },
        }
    }

    /// The caller's PDA funds the mint rent, so the authority written into that
    /// mint must be the caller's own PDA. Under DELEGATECALL the caller is the
    /// outer user, so an attacker-chosen authority would hand a third party
    /// control of a mint the victim paid for (Halborn FIND-003 residual).
    #[test]
    fn a_foreign_mint_authority_is_refused() {
        let state = TestOrigin::new();
        let helper = HelperProgram::new(&state);
        let attacker = Pubkey::new_unique();

        let res = helper.create_and_init_mint(&abi(&attacker, false, &Pubkey::default()), &call(vec![]));

        assert!(res.is_err(), "a mint_authority that is not the caller's PDA must be refused");
    }

    /// Same rule for the freeze authority when one is requested.
    #[test]
    fn a_foreign_freeze_authority_is_refused() {
        let state = TestOrigin::new();
        let helper = HelperProgram::new(&state);
        let (caller_pda, _) = state.base().pda.external_auth(&CALLER);
        let attacker = Pubkey::new_unique();

        let res = helper.create_and_init_mint(&abi(&caller_pda, true, &attacker), &call(vec![]));

        assert!(res.is_err(), "a freeze_authority that is not the caller's PDA must be refused");
    }

    /// The shape the live factory sends (mint authority = HelperProgram.pda(msg.sender),
    /// no freeze authority) must keep working unchanged.
    #[test]
    fn the_callers_own_pda_as_authority_is_accepted() {
        let state = TestOrigin::new();
        let helper = HelperProgram::new(&state);
        let (caller_pda, _) = state.base().pda.external_auth(&CALLER);

        let res = helper.create_and_init_mint(&abi(&caller_pda, false, &Pubkey::default()), &call(vec![]));

        assert!(res.is_ok(), "the factory's own call shape must still be accepted");
    }
}
