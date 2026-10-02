use {
    crate::{
        context::ContextIt,
        error::{Result, RomeProgramError::Custom,},
        split_fee, split_u64,
        state::State,
        tx::tx::Tx,
        vm::{vm_iterative::MachineIt::FromStateHolder, Execute, VmIt},
        H160, H256, msg, OPCODE_LIMIT_MIN,
    },
    solana_program::{account_info::AccountInfo, keccak, pubkey::Pubkey},
};

// unique | session | holder_index | limit | Option<fee_recipient> | pri_fee | tx
#[allow(clippy::type_complexity)]
pub fn args(data: &[u8]) -> Result<(u64, u64, u64, Option<H160>, u64, &[u8])> {
    let (_, data) = split_u64(data)?;
    let (session, data) = split_u64(data)?;
    let (holder, data) = split_u64(data)?;
    let (limit, data) = split_u64(data)?;
    let (fee_addr, pri_fee, tx) = split_fee(data)?;

    Ok((session, holder, limit, fee_addr, pri_fee, tx))
}

pub fn verify_opcode_limit(limit: u64) -> Result<()> {
    if limit < OPCODE_LIMIT_MIN {
        return Err(Custom("CU limit is not enough".to_string()))
    }
    Ok(())
}

pub fn do_tx_iterative<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: Iterative transaction");
    
    let (session, holder, limit, fee_addr, pri_fee, rlp) = args(data)?;
    verify_opcode_limit(limit)?;
    let hash = H256::from(keccak::hash(rlp).to_bytes());
    let chain_id = Tx::chain_id_from_rlp(rlp)?;

    let state = State::new(program_id, accounts, chain_id)?;
    let state_holder = state.info_state_holder(holder, true)?;

    let context = ContextIt::new(
        &state,
        rlp,
        hash,
        session,
        fee_addr,
        pri_fee,
        None,
        state_holder,
    )?;
    let mut vm = VmIt::new(&state, &context, limit)?;
    vm.consume(FromStateHolder)
}

#[cfg(test)]
mod tests {
    use super::*;

    // unique | session | holder | limit | pay_fee(0) | pri_fee | tx
    fn build_args_data(limit: u64) -> Vec<u8> {
        let mut data = vec![];
        data.extend_from_slice(&0u64.to_le_bytes()); // unique
        data.extend_from_slice(&0u64.to_le_bytes()); // session
        data.extend_from_slice(&0u64.to_le_bytes()); // holder
        data.extend_from_slice(&limit.to_le_bytes()); // limit
        data.push(0u8); // pay_fee = 0 -> no fee_addr
        data.extend_from_slice(&0u64.to_le_bytes()); // pri_fee
        data.extend_from_slice(b"rlp"); // tx (opaque past this point)
        data
    }

    /// Drives the boundary off OPCODE_LIMIT_MIN itself, not a copied literal, so
    /// a change to the const moves this test with it. limit=0 is the
    /// find-008 exploit input: a zero-opcode leg that still refreshed the
    /// account lock's TTL with no forward progress.
    #[test]
    fn verify_opcode_limit_boundary() {
        assert!(verify_opcode_limit(0).is_err());
        assert!(verify_opcode_limit(OPCODE_LIMIT_MIN - 1).is_err());
        assert!(verify_opcode_limit(OPCODE_LIMIT_MIN).is_ok());
        assert!(verify_opcode_limit(OPCODE_LIMIT_MIN + 1).is_ok());
    }

    // rome-sdk rome-evm-client/src/tx/iterative.rs::OPCODE_START = 1500 is
    // the real starting per-leg opcode limit the SDK sends. Pinned as a
    // literal (not imported cross-repo) so that if OPCODE_LIMIT_MIN is ever
    // raised above the SDK's start value, this test catches every real
    // first-leg call being rejected.
    #[test]
    fn verify_opcode_limit_admits_sdk_opcode_start() {
        const SDK_OPCODE_START: u64 = 1500;
        assert!(verify_opcode_limit(SDK_OPCODE_START).is_ok());
    }

    /// The rejection happens at PARSE, before any lock is taken or state
    /// touched: args() happily parses a limit=0 buffer (it's just bytes),
    /// and verify_opcode_limit -- called immediately after args() in
    /// do_tx_iterative, ahead of any State::new/lock -- is what actually
    /// refuses it. A legit limit parses and clears the floor the same way.
    #[test]
    fn zero_limit_parses_then_is_rejected_by_verify_opcode_limit() {
        let zero_limit_data = build_args_data(0);
        let (_, _, limit, _, _, _) = args(&zero_limit_data).unwrap();
        assert_eq!(limit, 0);
        assert!(verify_opcode_limit(limit).is_err());

        let legit_data = build_args_data(OPCODE_LIMIT_MIN);
        let (_, _, limit, _, _, _) = args(&legit_data).unwrap();
        assert!(verify_opcode_limit(limit).is_ok());
    }
}
