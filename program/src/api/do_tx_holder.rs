use {
    crate::{
        context::ContextAt,
        error::{Result, RomeProgramError::*},
        split_fee, split_hash, split_u64,
        state::State,
        vm::{vm_atomic::MachineAt, Execute, VmAt},
        Holder, origin::Origin, SIG_VERIFY_COST, Data, msg, TRANSMIT_CHUNK_SIZE,
    },
    evm::{H160, H256},
    solana_program::{account_info::AccountInfo, pubkey::Pubkey},
    
};

// holder_index | tx_hash | chain_id | Option<fee_recipient> | pri_fee
pub fn args(data: &[u8]) -> Result<(u64, H256, u64, Option<H160>, u64)> {
    let (holder, data) = split_u64(data)?;
    let (hash, data) = split_hash(data)?;
    let (chain, data) = split_u64(data)?;
    let (fee_addr, pri_fee, _) = split_fee(data)?;

    Ok((holder, hash, chain, fee_addr, pri_fee))
}

/// Pure fee calc, factored out of [`transmit_fee`] so it's testable without an
/// `AccountInfo` — one signature/chunk charged per `TRANSMIT_CHUNK_SIZE`-byte
/// leg the SDK stages the tx across (`ceil(len / TRANSMIT_CHUNK_SIZE)`).
pub fn transmit_fee_from_len(len: u64) -> Result<u64> {
    let cnt = len.div_ceil(TRANSMIT_CHUNK_SIZE);
    SIG_VERIFY_COST.checked_mul(cnt).ok_or(CalculationOverflow)
}

pub fn transmit_fee (info: &AccountInfo) -> Result<u64> {
    let len = Holder::from_account(info)?.len() as u64;
    transmit_fee_from_len(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins fee == SIG_VERIFY_COST * ceil(len/chunk) so estimate == charge
    /// survives the chunk-size change.
    #[test]
    fn transmit_fee_matches_ceil_div_chunk() {
        let chunk = TRANSMIT_CHUNK_SIZE;
        for len in [1, chunk - 1, chunk, chunk + 1, 3 * chunk] {
            let expected = SIG_VERIFY_COST * len.div_ceil(chunk);
            assert_eq!(
                transmit_fee_from_len(len).unwrap(),
                expected,
                "len={len} chunk={chunk}"
            );
        }
    }
}

pub fn add_transmit_fee<T: Origin>(state: &T, info: &AccountInfo) -> Result<()> {
    let lamports = transmit_fee(info)?;
    state.base().add_fee(lamports)
}

pub fn do_tx_holder<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: Atomic transaction from holder");

    let (holder, hash, chain, fee_addr, pri_fee) = args(data)?;
    let state = State::new(program_id, accounts, chain)?;

    let info = state.info_tx_holder(holder, false)?;
    add_transmit_fee(&state, info)?;

    let rlp = Holder::rlp(info, hash, chain)?;

    let context = ContextAt::new(&state);
    let mut vm = VmAt::new(&state, &rlp, fee_addr, pri_fee, &context)?;

    vm.consume(MachineAt::Lock)
}
