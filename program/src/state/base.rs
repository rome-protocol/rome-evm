use {
    super::pda::{Pda, Seed},
    crate::{accounts::OwnerInfo, error::RomeProgramError::*, error::*},
    evm::{H160, U256},
    solana_program::{account_info::MAX_PERMITTED_DATA_INCREASE, pubkey::Pubkey},
    std::{cell::RefCell, rc::Rc},
};

macro_rules! impl_alloc_fn {
    ($alloc:ident, $dealloc:ident, $func:ident) => {
        pub fn $alloc(&self) -> usize {
            *self.$alloc.borrow()
        }

        pub fn $func(&self, len: usize) -> Result<()> {
            if len > 0 {
                if self.$dealloc() > 0 {
                    return Err(AllocationError(
                        "found allocations and deallocations".to_string(),
                    ));
                }
                let mut val = self.$alloc.borrow_mut();
                *val = val.saturating_add(len);
            }

            Ok(())
        }
    };
}

pub struct Base<'a> {
    pub program_id: &'a Pubkey,
    alloc: RefCell<usize>,
    dealloc: RefCell<usize>,
    alloc_payed: RefCell<usize>,
    dealloc_payed: RefCell<usize>,
    pub pda: Pda<'a>,
    pub syscall: Rc<Syscall>,
    pub lamports_fee: RefCell<u64>,
    pub lamports_refund: RefCell<u64>,
    pub owner_info: Option<OwnerInfo>,
    pub found_cpi: RefCell<bool>,
}

impl<'a> Base<'a> {
    pub fn new(program_id: &'a Pubkey, chain: u64, owner_info: Option<OwnerInfo>) -> Self {
        let syscall = Rc::new(Syscall::new());

        Self {
            program_id,
            alloc: RefCell::new(0),
            dealloc: RefCell::new(0),
            alloc_payed: RefCell::new(0),
            dealloc_payed: RefCell::new(0),
            pda: Pda::new(program_id, chain, Rc::clone(&syscall)),
            syscall,
            lamports_fee: RefCell::new(0),
            lamports_refund: RefCell::new(0),
            owner_info,
            found_cpi: RefCell::new(false),
        }
    }

    pub fn owner_info(&self) -> &OwnerInfo {
        self.owner_info.as_ref().expect("owner_info expected")
    }

    pub fn single_state(&self) -> bool {
        self.owner_info().single_state
    }
    pub fn alloc_limit(&self) -> usize {
        MAX_PERMITTED_DATA_INCREASE.saturating_sub(self.alloc())
    }

    impl_alloc_fn!(alloc, dealloc, inc_alloc);
    impl_alloc_fn!(dealloc, alloc, inc_dealloc);
    impl_alloc_fn!(alloc_payed, dealloc_payed, inc_alloc_payed);
    impl_alloc_fn!(dealloc_payed, alloc_payed, inc_dealloc_payed);

    pub fn set_alloc_payed(&self, len: usize) {
        *self.alloc_payed.borrow_mut() = len;
    }

    pub fn slot_to_key(&self, address: &H160, slot: &U256) -> (Pubkey, Seed, u8) {
        let (index_be, sub_ix) = Pda::storage_index(slot);
        let (base, _) = self.pda.balance_key(address);
        let (key, seed) = self.pda.storage_key(&base, index_be);

        (key, seed, sub_ix)
    }

    #[cfg(not(target_os = "solana"))]
    pub fn reset(&self) {
        *self.alloc.borrow_mut() = 0;
        *self.dealloc.borrow_mut() = 0;
        *self.alloc_payed.borrow_mut() = 0;
        *self.dealloc_payed.borrow_mut() = 0;
        self.syscall.reset();
        self.pda.reset();
        *self.lamports_fee.borrow_mut() = 0;
        *self.lamports_refund.borrow_mut() = 0;
        *self.found_cpi.borrow_mut() = false;
    }

    pub fn get_fees(&self) -> (u64, u64) {
        (*self.lamports_fee.borrow(), *self.lamports_refund.borrow())
    }
    pub fn set_fees(&self, fee: u64, refund: u64) -> Result<()> {
        *self.lamports_fee.borrow_mut() = fee;
        *self.lamports_refund.borrow_mut() = refund;
        Ok(())
    }

    pub fn add_fee(&self, lamports: u64) -> Result<()> {
        let mut val = self.lamports_fee.borrow_mut();
        *val = val.checked_add(lamports).ok_or(CalculationOverflow)?;
        Ok(())
    }

    pub fn add_refund(&self, lamports: u64) -> Result<()> {
        // Bug fix 2026-05-10: previously wrote to `lamports_fee`, which
        // double-billed the user any time a CPI returned lamports to the
        // operator (e.g., closing an SPL account from Solidity via the CPI
        // precompile). `gas_transfer` reads `(fee, refund) = get_fees()` and
        // computes user_charge = fee.saturating_sub(refund); when refunds
        // landed on the wrong field, the saturating_sub had nothing to
        // subtract, and the inflated fee was billed instead of cancelled.
        // Refunds must therefore accumulate in `lamports_refund`.
        let mut val = self.lamports_refund.borrow_mut();
        *val = val.checked_add(lamports).ok_or(CalculationOverflow)?;
        Ok(())
    }

    pub fn reset_fees(&self) {
        *self.lamports_fee.borrow_mut() = 0;
        *self.lamports_refund.borrow_mut() = 0;
    }
}

#[derive(Clone)]
pub struct Syscall {
    cnt: RefCell<u64>,
}

impl Syscall {
    pub fn inc(&self) {
        *self.cnt.borrow_mut() += 1;
    }
    pub fn count(&self) -> u64 {
        *self.cnt.borrow()
    }
    pub fn new() -> Self {
        Self {
            cnt: RefCell::new(0),
        }
    }
    #[cfg(not(target_os = "solana"))]
    pub fn reset(&self) {
        *self.cnt.borrow_mut() = 0;
    }
}

impl Default for Syscall {
    fn default() -> Self {
        Syscall::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    /// Pre-fix: `add_refund` wrote to `lamports_fee`, so calling it after
    /// `add_fee` made `get_fees()` return the SUM rather than separate
    /// fee + refund counters. This test would have caught it.
    #[test]
    fn add_refund_writes_to_lamports_refund_not_lamports_fee() {
        let program_id = Pubkey::new_unique();
        let base = Base::new(&program_id, 1u64, None);

        base.add_fee(1_000).unwrap();
        base.add_refund(300).unwrap();

        let (fee, refund) = base.get_fees();
        assert_eq!(fee, 1_000, "add_refund must NOT increment lamports_fee");
        assert_eq!(refund, 300, "add_refund must increment lamports_refund");
    }

    /// End-to-end semantics check: gas_transfer's `fee.saturating_sub(refund)`
    /// must yield the NET operator cost, not the sum. With the pre-fix bug,
    /// `refund` stayed at 0 forever and the user was double-billed.
    #[test]
    fn refund_cancels_fee_via_saturating_sub() {
        let program_id = Pubkey::new_unique();
        let base = Base::new(&program_id, 1u64, None);

        // Operator paid 1,000,000 lamports for an account allocation,
        // then a CPI returned 900,000 lamports (account closed inside CPI).
        base.add_fee(1_000_000).unwrap();
        base.add_refund(900_000).unwrap();

        let (fee, refund) = base.get_fees();
        let user_charge = fee.saturating_sub(refund);
        assert_eq!(
            user_charge, 100_000,
            "user should be billed only the NET operator cost (fee - refund)"
        );
    }

    /// Belt-and-braces: when refund > fee (theoretically possible if the only
    /// CPI returned more lamports than were taken), saturating_sub clamps at
    /// zero — the user is charged nothing, never negative.
    #[test]
    fn refund_exceeds_fee_clamps_to_zero() {
        let program_id = Pubkey::new_unique();
        let base = Base::new(&program_id, 1u64, None);

        base.add_fee(100).unwrap();
        base.add_refund(500).unwrap();

        let (fee, refund) = base.get_fees();
        let user_charge = fee.saturating_sub(refund);
        assert_eq!(
            user_charge, 0,
            "saturating_sub clamps at zero when refund > fee"
        );
    }

    /// Multiple add_fee/add_refund calls accumulate independently.
    #[test]
    fn multiple_calls_accumulate_in_separate_fields() {
        let program_id = Pubkey::new_unique();
        let base = Base::new(&program_id, 1u64, None);

        base.add_fee(100).unwrap();
        base.add_fee(200).unwrap();
        base.add_refund(50).unwrap();
        base.add_refund(75).unwrap();
        base.add_fee(300).unwrap();

        let (fee, refund) = base.get_fees();
        assert_eq!(fee, 600, "fee should accumulate across calls");
        assert_eq!(refund, 125, "refund should accumulate across calls");
        assert_eq!(fee.saturating_sub(refund), 475);
    }

    /// `reset_fees` zeros both fields.
    #[test]
    fn reset_fees_zeros_both_fields() {
        let program_id = Pubkey::new_unique();
        let base = Base::new(&program_id, 1u64, None);

        base.add_fee(1_000).unwrap();
        base.add_refund(500).unwrap();
        base.reset_fees();

        assert_eq!(base.get_fees(), (0, 0));
    }
}
