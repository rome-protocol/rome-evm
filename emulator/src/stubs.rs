use {
    crate::account_storage::AccountStorage,
    rome_evm::error::Result,
    solana_account::Account,
    solana_program::{
        pubkey::Pubkey,
        clock::Clock, rent::Rent, sysvar, program_stubs::SyscallStubs,
    },
    std::sync::Arc,
};

#[derive(Default)]
pub struct Stubs {
    rent: Rent,
    clock: Clock,
}

impl Stubs {
    /// Build the syscall stubs from chain (clock/rent). Signature is
    /// unchanged — callers that only install the stubs (e.g. the CLI) use this.
    pub fn from_chain(rpc: Arc<dyn AccountStorage>) -> Result<Box<Self>> {
        Ok(Self::from_chain_with_accounts(rpc)?.0)
    }

    /// Like `from_chain`, but ALSO returns the raw sysvar accounts it loaded.
    /// The emulator `State` records them into `Emulation.original` so downstream
    /// consumers carry them: the iterative confirm-path (rome-sdk) rebuilds an
    /// `AccountStorage` from that set and re-runs `from_chain` against it — if
    /// the sysvars aren't there it DEFAULTS them (slot 0 / default `Rent`) and
    /// installs those process-globally, surfacing as OZ
    /// `CheckpointUnorderedInsertion` (block.number 0) and swap "PDA doesn't
    /// have enough lamports for rent".
    pub fn from_chain_with_accounts(rpc: Arc<dyn AccountStorage>) -> Result<(Box<Self>, Vec<(Pubkey, Account)>)> {
        let keys = [sysvar::clock::ID, sysvar::rent::ID];
        let accounts = rpc.get_multiple_accounts(&keys)?;

        let mut stubs = Stubs::default();
        let mut fetched = Vec::new();
        for (&key, acc) in keys.iter().zip(accounts.iter()) {
            let Some(acc) = acc else {
                continue;
            };

            match key {
                sysvar::clock::ID => {
                    stubs.clock = bincode::serde::decode_from_slice(&acc.data, bincode::config::legacy())?.0;
                }
                sysvar::rent::ID => {
                    stubs.rent = bincode::serde::decode_from_slice(&acc.data, bincode::config::legacy())?.0;
                }
                _ => unreachable!(),
            }

            // Agave 4.x: `sol_get_sysvar` no longer routes through this file's
            // `SyscallStubs` impl and fails closed off-chain otherwise. Mirror
            // the bytes into the fork's host-only registry too.
            // https://github.com/rome-protocol/solana-get-sysvar
            solana_get_sysvar::set_sysvar_bytes(key, acc.data.clone());

            fetched.push((key, acc.clone()));
        }

        // spl-token-2022 11.0.0 pins its own `solana-sysvar 3.x` (semver-
        // incompatible with our `solana-program` 4.x line), so it reads
        // Clock/Rent through a genuinely separate stub registry — neither
        // `solana_get_sysvar` above nor `solana_program::program_stubs`
        // reaches it. Install here too, via a small dedicated struct.
        solana_sysvar::program_stubs::set_syscall_stubs(Box::new(LegacySysvarStubs {
            rent: stubs.rent.clone(),
            clock: stubs.clock.clone(),
        }));

        Ok((Box::new(stubs), fetched))
    }
}

/// Serves `solana-sysvar` 3.x's own independent `program_stubs` registry
/// (see the install call above).
struct LegacySysvarStubs {
    rent: Rent,
    clock: Clock,
}

impl solana_sysvar::program_stubs::SyscallStubs for LegacySysvarStubs {
    fn sol_get_rent_sysvar(&self, pointer: *mut u8) -> u64 {
        unsafe {
            #[allow(clippy::cast_ptr_alignment)]
            let rent = pointer.cast::<Rent>();
            *rent = self.rent.clone();
        }
        0
    }
    fn sol_get_clock_sysvar(&self, pointer: *mut u8) -> u64 {
        unsafe {
            #[allow(clippy::cast_ptr_alignment)]
            let clock = pointer.cast::<Clock>();
            *clock = self.clock.clone();
        }
        0
    }
}

impl SyscallStubs for Stubs {
    fn sol_get_rent_sysvar(&self, pointer: *mut u8) -> u64 {
        unsafe {
            #[allow(clippy::cast_ptr_alignment)]
            let rent = pointer.cast::<Rent>();
            *rent = self.rent.clone();
        }
        0
    }
    fn sol_get_clock_sysvar(&self, pointer: *mut u8) -> u64 {
        unsafe {
            #[allow(clippy::cast_ptr_alignment)]
            let clock = pointer.cast::<Clock>();
            *clock = self.clock.clone();
        }
        0
    }
}
