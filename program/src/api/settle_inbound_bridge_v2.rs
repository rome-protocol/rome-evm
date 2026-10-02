use {
    crate::{
        api::{split_u64, split_h160, split_hash},
        context::{AccountLock, ContextAt},
        error::{Result, RomeProgramError::*},
        origin::Origin,
        wei_to_u64,
        state::{State, aux::{checked_transfer, mint_profile}},
        msg, U256,
    },
    solana_program::{
        account_info::AccountInfo,
        clock::Clock,
        keccak::hashv,
        pubkey::Pubkey,
        secp256k1_recover::secp256k1_recover,
        sysvar::Sysvar,
    },
    spl_associated_token_account_interface::address::get_associated_token_address_with_program_id as ata,
    spl_token_2022_interface::{
        extension::StateWithExtensions, state::Account as SplAccount,
    },
};
// User-authorized inbound bridge settlement (v2).
//
// v1 (slot 15) trusts a caller-supplied `chain_id` gated only by
// `bridge_settler_key`. Because the user's PDA-ATA is chain-agnostic
// (`external_auth` has no chain_id in its seed), a griefing settler can
// redirect a bridged deposit to a different chain on the same program
// with the same gas mint. v2 closes that: the USER signs an EIP-712
// `SettleAuthorization` binding `(destinationChainId, mint, amount,
// sourceChain, sourceTxHash, deadline)`; ANY caller may submit; the
// program recomputes the digest from its own parameters and requires
// `secp256k1_recover(digest, sig)` to yield `user`. A tampered chain_id
// (or any other field) changes the recomputed digest, so recovery yields
// a different address and the settle refuses with `SignerNotUser`.
//
// Signature mechanism: direct `secp256k1_recover` syscall in
// this instruction — the same primitive `tx/tx.rs` uses on every EVM tx.
// The syscall does NOT enforce low-s (verified against its source docs),
// so EIP-2 canonical form is enforced here explicitly.
//
// Encoding (157 bytes):
//   chain_id            : u64 LE   — destination Rome chain id
//   user                : H160 (20B)
//   bridged_amount      : u64 LE   (mint base units)
//   source_chain        : u64 LE   — replay-key component (source EVM chain id)
//   source_tx_hash      : H256 (32B)
//   deadline            : u64 LE   — unix seconds; sig invalid after
//   source_evm_chain_id : u64 LE   — EIP-712 domain.chainId (where the user signed)
//   sig_r               : 32B
//   sig_s               : 32B
//   sig_v               : u8       — Ethereum recovery byte (27 | 28)

/// keccak256("SettleAuthorization(uint64 destinationChainId,bytes32 mint,uint64 amount,uint64 sourceChain,bytes32 sourceTxHash,uint64 deadline)")
/// Golden vector; pinned by a unit test.
pub const SETTLE_AUTHORIZATION_TYPEHASH: [u8; 32] = [
    0xcc, 0x0b, 0x60, 0x54, 0xaa, 0xb1, 0x50, 0x32, 0x41, 0xe0, 0x11, 0x3f, 0xa2, 0x9f, 0x28, 0x84,
    0x67, 0x17, 0x58, 0x18, 0x28, 0x41, 0xfc, 0x8d, 0x81, 0x14, 0x3b, 0x12, 0x86, 0x71, 0xd6, 0xb4,
];

/// keccak256("EIP712Domain(string name,string version,uint256 chainId,bytes32 salt)")
pub const EIP712_DOMAIN_TYPEHASH: [u8; 32] = [
    0xa6, 0x04, 0xff, 0xf5, 0xa2, 0x7d, 0x59, 0x51, 0xf3, 0x34, 0xcc, 0xda, 0x7a, 0xbf, 0xf3, 0x28,
    0x6a, 0x8a, 0xf2, 0x9c, 0xae, 0xeb, 0x19, 0x6a, 0x6f, 0x2b, 0x40, 0xa1, 0xdc, 0xe7, 0x61, 0x2b,
];


fn split_u8(data: &[u8]) -> Result<(u8, &[u8])> {
    let (b, rest) = data.split_first().ok_or(InvalidInstructionData)?;
    Ok((*b, rest))
}
fn split_32<'a>(data: &'a [u8]) -> Result<([u8; 32], &'a [u8])> {
    if data.len() < 32 {
        return Err(InvalidInstructionData);
    }
    let (head, rest) = data.split_at(32);
    let mut out = [0u8; 32];
    out.copy_from_slice(head);
    Ok((out, rest))
}

/// See the 157-byte encoding in the module header.
#[allow(clippy::type_complexity)]
pub fn args(
    data: &[u8],
) -> Result<(u64, evm::H160, u64, u64, evm::H256, u64, u64, [u8; 32], [u8; 32], u8)> {
    let (chain, data) = split_u64(data)?;
    let (user, data) = split_h160(data)?;
    let (bridged_amount, data) = split_u64(data)?;
    let (source_chain, data) = split_u64(data)?;
    let (source_tx_hash, data) = split_hash(data)?;
    let (deadline, data) = split_u64(data)?;
    let (source_evm_chain_id, data) = split_u64(data)?;
    let (sig_r, data) = split_32(data)?;
    let (sig_s, data) = split_32(data)?;
    let (sig_v, data) = split_u8(data)?;
    if !data.is_empty() {
        return Err(InvalidInstructionData);
    }
    Ok((chain, user, bridged_amount, source_chain, source_tx_hash, deadline, source_evm_chain_id, sig_r, sig_s, sig_v))
}

/// abi.encode of a u64 as a left-padded 32-byte word.
fn word_u64(v: u64) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&v.to_be_bytes());
    w
}

/// Recompute the EIP-712 digest the user signed (spec §Digest):
///   keccak256(0x1901 || domainSeparator || hashStruct(message))
#[allow(clippy::too_many_arguments)]
pub fn eip712_digest(
    program_id: &Pubkey,
    source_evm_chain_id: u64,
    destination_chain_id: u64,
    mint: &Pubkey,
    amount: u64,
    source_chain: u64,
    source_tx_hash: &evm::H256,
    deadline: u64,
) -> [u8; 32] {
    // domainSeparator = keccak256(abi.encode(
    //   EIP712DOMAIN_TYPEHASH, keccak256("Rome Bridge Settlement"),
    //   keccak256("1"), source_evm_chain_id (uint256),
    //   keccak256(program_id_bytes) (bytes32) ))
    let name_hash = hashv(&[b"Rome Bridge Settlement"]).to_bytes();
    let version_hash = hashv(&[b"1"]).to_bytes();
    let mut chain_word = [0u8; 32];
    chain_word[24..].copy_from_slice(&source_evm_chain_id.to_be_bytes());
    let salt = hashv(&[program_id.as_ref()]).to_bytes();
    let domain_separator = hashv(&[
        &EIP712_DOMAIN_TYPEHASH,
        &name_hash,
        &version_hash,
        &chain_word,
        &salt,
    ]).to_bytes();

    // hashStruct = keccak256(abi.encode(
    //   TYPEHASH, destinationChainId, mint, amount, sourceChain,
    //   sourceTxHash, deadline ))
    let struct_hash = hashv(&[
        &SETTLE_AUTHORIZATION_TYPEHASH,
        &word_u64(destination_chain_id),
        mint.as_ref(),
        &word_u64(amount),
        &word_u64(source_chain),
        source_tx_hash.as_bytes(),
        &word_u64(deadline),
    ]).to_bytes();

    hashv(&[&[0x19, 0x01], &domain_separator, &struct_hash]).to_bytes()
}

/// Pure authorization check — `now` injected for testability; the ix
/// handler wraps with `Clock::get()`. Order per spec: deadline → low-s →
/// recovery byte → recover → keccak → address compare.
#[allow(clippy::too_many_arguments)]
pub fn check_user_authorization_at(
    now: u64,
    program_id: &Pubkey,
    user: &evm::H160,
    mint: &Pubkey,
    destination_chain_id: u64,
    bridged_amount: u64,
    source_chain: u64,
    source_tx_hash: &evm::H256,
    deadline: u64,
    source_evm_chain_id: u64,
    sig_r: &[u8; 32],
    sig_s: &[u8; 32],
    sig_v: u8,
) -> Result<()> {
    // 1. Deadline.
    if now > deadline {
        return Err(SignatureExpired);
    }

    // 2. Low-s canonical form (EIP-2). The secp256k1_recover syscall does
    //    NOT enforce it — without this an s' = n - s malleation recovers to
    //    the same address. Big-endian compare against n/2.
    if sig_s.as_slice() > crate::config::SECP256K1_N_HALF.as_slice() {
        return Err(NonCanonicalSignature);
    }

    // 3. Recovery byte (Ethereum convention 27/28 → recovery_id 0/1).
    let recovery_id = sig_v.checked_sub(27).ok_or(InvalidRecoveryByte)?;
    if recovery_id > 1 {
        return Err(InvalidRecoveryByte);
    }

    // 4. Recompute the digest from THIS program's parameters (not caller-
    //    supplied) — the binding that makes a redirected field un-signable.
    let digest = eip712_digest(
        program_id, source_evm_chain_id, destination_chain_id, mint,
        bridged_amount, source_chain, source_tx_hash, deadline,
    );

    // 5. Recover the signer and compare eth_addr = keccak256(pubkey)[12..].
    let mut rs = [0u8; 64];
    rs[..32].copy_from_slice(sig_r);
    rs[32..].copy_from_slice(sig_s);
    let pubkey = secp256k1_recover(&digest, recovery_id, &rs).map_err(|_| SignerNotUser)?;
    let recovered = hashv(&[&pubkey.to_bytes()]).to_bytes();
    if recovered[12..] != *user.as_bytes() {
        return Err(SignerNotUser);
    }
    Ok(())
}

pub fn settle_inbound_bridge_v2<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    let (chain, user, bridged_amount, source_chain, source_tx_hash, deadline, source_evm_chain_id, sig_r, sig_s, sig_v) = args(data)?;
    msg!(
        "Instruction: settle_inbound_bridge_v2 user={:?} amount={} src_chain={} src_tx={:?} dest_chain={}",
        user, bridged_amount, source_chain, source_tx_hash, chain,
    );

    if bridged_amount == 0 {
        return Err(Custom("settle_inbound_bridge_v2: bridged_amount=0".to_string()));
    }

    let state = State::new(program_id, accounts, chain)?;

    // The chain's configured native mint — also the mint the user authorized.
    let mint = state.base.owner_info().mint.ok_or_else(|| {
        Unimplemented("settle_inbound_bridge_v2 requires chain_mint_id".to_string())
    })?;

    // AUTHORIZATION: replaces v1's `check_signer(bridge_settler_key)`. Any
    // caller may submit; the user's EIP-712 signature over
    // (chain, mint, amount, source_chain, source_tx_hash, deadline) must
    // recover to `user`. A redirected chain_id changes the digest → refuses.
    let now = Clock::get()?.unix_timestamp as u64;
    check_user_authorization_at(
        now, program_id, &user, &mint, chain, bridged_amount, source_chain,
        &source_tx_hash, deadline, source_evm_chain_id, &sig_r, &sig_s, sig_v,
    )?;

    // Replay protection — identical to v1: once-only per (chain, source_chain, source_tx_hash).
    let (br_key, _) = state.pda.bridge_processed_key(source_chain, source_tx_hash.as_fixed_bytes());
    let info_bridge = state.info_any(&br_key)?;
    if info_bridge.owner == program_id {
        return Err(ReplayProtection);
    }
    let _bridge_marker = state.info_bridge_processed(source_chain, source_tx_hash.as_fixed_bytes(), true)?;

    let profile = mint_profile(&state, &mint)?;
    let (spl_program, decimals) = (profile.program, profile.decimals);

    // Source: user's PDA-owned ATA (where CCTP/WH delivered). Dest: gas pool ATA.
    let (user_pda, user_seed) = state.base.pda.external_auth(&user);
    let from_ata = ata(&user_pda, &mint, &spl_program);
    let (wallet, _wallet_seed) = state.base.pda.sol_wallet();
    let to_ata = ata(&wallet, &mint, &spl_program);

    // ON-CHAIN BOUND: bridged_amount cannot exceed the user's actual balance.
    let from_ata_acc = state.account(&from_ata)?;
    let ata_balance = StateWithExtensions::<SplAccount>::unpack(&from_ata_acc.data)?.base.amount;
    if bridged_amount > ata_balance {
        return Err(Custom(format!(
            "settle_inbound_bridge_v2: bridged_amount {} > ATA balance {}",
            bridged_amount, ata_balance,
        )));
    }

    let ix =
        checked_transfer(&profile, &mint, &from_ata, &to_ata, &user_pda, &[], bridged_amount)?;
    state.invoke_signed(&ix, vec![user_seed], false)?;

    // Credit BalancePda by bridged_amount scaled to native wei.
    let pow = u32::from(crate::RSOL_DECIMALS.saturating_sub(decimals));
    let value_wei = U256::from(bridged_amount)
        .checked_mul(U256::exp10(pow as usize))
        .ok_or(CalculationOverflow)?;
    debug_assert_eq!(wei_to_u64(value_wei, decimals)?, bridged_amount);

    let context = ContextAt::new(&state);
    context.lock()?;
    state.add_balance(&user, &value_wei, &context)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // ── Golden fixture (generated 2026-07-04 via viem hashTypedData +
    // privateKeyToAccount sign; typeHash cross-checked equal to the spec's
    // locked Phase-0.2 vector). Signer = hardhat account #0. ──
    const FIX_PROGRAM_ID: &str = "RPTWwELXAY4KC9ZPHhaxp7Sq1hHtU3HNEgLbSegCcWf";
    const FIX_MINT: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";
    const FIX_DEST_CHAIN: u64 = 200_010;
    const FIX_AMOUNT: u64 = 1_000_000;
    const FIX_SOURCE_CHAIN: u64 = 11_155_111;
    const FIX_SOURCE_EVM_CHAIN: u64 = 11_155_111;
    const FIX_DEADLINE: u64 = 1_751_630_000;
    const FIX_SOURCE_TX: [u8; 32] = [0xab; 32];
    const FIX_DIGEST: &str = "0237b5760e2d135e1d9d2049e365d73023bfed0a49f8392a1bf8ecbef0d9996e";
    const FIX_SIGNER: &str = "f39fd6e51aad88f6f4ce6ab8827279cfffb92266";
    const FIX_R: &str = "497895150e6d4bd0399b353f49bc4332080bcdd1dba36806338d8e0c30753707";
    const FIX_S: &str = "7f63f6c03f40fa8570e68ec29492d1dca619c0f87be30d83ae057a9b125babc1";
    const FIX_V: u8 = 27;

    fn hex32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        hex::decode_to_slice(s, &mut out).expect("hex32");
        out
    }
    fn fix_user() -> evm::H160 {
        evm::H160::from_slice(&hex::decode(FIX_SIGNER).unwrap())
    }
    fn fix_ids() -> (Pubkey, Pubkey) {
        (Pubkey::from_str(FIX_PROGRAM_ID).unwrap(), Pubkey::from_str(FIX_MINT).unwrap())
    }
    fn valid_check(now: u64, dest: u64, user: evm::H160, s: [u8; 32], v: u8) -> Result<()> {
        let (program_id, mint) = fix_ids();
        check_user_authorization_at(
            now, &program_id, &user, &mint, dest, FIX_AMOUNT, FIX_SOURCE_CHAIN,
            &evm::H256::from(FIX_SOURCE_TX), FIX_DEADLINE, FIX_SOURCE_EVM_CHAIN,
            &hex32(FIX_R), &s, v,
        )
    }

    #[test]
    fn typehash_constants_match_their_canonical_strings() {
        let t = hashv(&[b"SettleAuthorization(uint64 destinationChainId,bytes32 mint,uint64 amount,uint64 sourceChain,bytes32 sourceTxHash,uint64 deadline)"]);
        assert_eq!(t.to_bytes(), SETTLE_AUTHORIZATION_TYPEHASH, "type-string drift");
        let d = hashv(&[b"EIP712Domain(string name,string version,uint256 chainId,bytes32 salt)"]);
        assert_eq!(d.to_bytes(), EIP712_DOMAIN_TYPEHASH, "domain-string drift");
    }

    #[test]
    fn eip712_digest_matches_the_cross_language_golden_vector() {
        let (program_id, mint) = fix_ids();
        let digest = eip712_digest(
            &program_id, FIX_SOURCE_EVM_CHAIN, FIX_DEST_CHAIN, &mint, FIX_AMOUNT,
            FIX_SOURCE_CHAIN, &evm::H256::from(FIX_SOURCE_TX), FIX_DEADLINE,
        );
        assert_eq!(digest, hex32(FIX_DIGEST), "Rust digest != viem hashTypedData");
    }

    #[test]
    fn authorization_accepts_the_golden_signature() {
        valid_check(FIX_DEADLINE - 60, FIX_DEST_CHAIN, fix_user(), hex32(FIX_S), FIX_V)
            .expect("golden sig must verify");
    }

    #[test]
    fn tampered_destination_chain_fails_as_signer_mismatch() {
        // A redirected chain_id changes the recomputed digest → recovery
        // yields a different address — the exact attack v2 exists to kill.
        match valid_check(FIX_DEADLINE - 60, FIX_DEST_CHAIN + 1, fix_user(), hex32(FIX_S), FIX_V) {
            Err(SignerNotUser) => {}
            other => panic!("expected SignerNotUser, got {:?}", other),
        }
    }

    #[test]
    fn expired_deadline_fails() {
        match valid_check(FIX_DEADLINE + 1, FIX_DEST_CHAIN, fix_user(), hex32(FIX_S), FIX_V) {
            Err(SignatureExpired) => {}
            other => panic!("expected SignatureExpired, got {:?}", other),
        }
    }

    #[test]
    fn high_s_fails_as_non_canonical() {
        // s' = n - s recovers to the same address with flipped v — the
        // malleation the syscall does NOT reject (verified V4). Must fail
        // BEFORE recovery.
        let n: [u8; 32] = hex32("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141");
        let s = hex32(FIX_S);
        let mut high_s = [0u8; 32];
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let d = i16::from(n[i]) - i16::from(s[i]) - borrow;
            let (val, b) = if d < 0 { (d + 256, 1) } else { (d, 0) };
            high_s[i] = val as u8;
            borrow = b;
        }
        match valid_check(FIX_DEADLINE - 60, FIX_DEST_CHAIN, fix_user(), high_s, 28) {
            Err(NonCanonicalSignature) => {}
            other => panic!("expected NonCanonicalSignature, got {:?}", other),
        }
    }

    #[test]
    fn invalid_recovery_byte_fails() {
        for v in [0u8, 1, 26, 29, 255] {
            match valid_check(FIX_DEADLINE - 60, FIX_DEST_CHAIN, fix_user(), hex32(FIX_S), v) {
                Err(InvalidRecoveryByte) => {}
                other => panic!("v={}: expected InvalidRecoveryByte, got {:?}", v, other),
            }
        }
    }

    #[test]
    fn wrong_signer_fails() {
        let other = evm::H160::from_slice(&[0x11; 20]);
        match valid_check(FIX_DEADLINE - 60, FIX_DEST_CHAIN, other, hex32(FIX_S), FIX_V) {
            Err(SignerNotUser) => {}
            e => panic!("expected SignerNotUser, got {:?}", e),
        }
    }

    // ── Wire format (157B) ──

    fn build_data() -> Vec<u8> {
        let mut d = Vec::with_capacity(157);
        d.extend_from_slice(&FIX_DEST_CHAIN.to_le_bytes());
        d.extend_from_slice(fix_user().as_bytes());
        d.extend_from_slice(&FIX_AMOUNT.to_le_bytes());
        d.extend_from_slice(&FIX_SOURCE_CHAIN.to_le_bytes());
        d.extend_from_slice(&FIX_SOURCE_TX);
        d.extend_from_slice(&FIX_DEADLINE.to_le_bytes());
        d.extend_from_slice(&FIX_SOURCE_EVM_CHAIN.to_le_bytes());
        d.extend_from_slice(&hex32(FIX_R));
        d.extend_from_slice(&hex32(FIX_S));
        d.push(FIX_V);
        d
    }

    #[test]
    fn args_round_trips_the_157_byte_layout() {
        let data = build_data();
        assert_eq!(data.len(), 157, "spec wire size");
        let (chain, user, amount, source_chain, source_tx, deadline, source_evm, r, s, v) =
            args(&data).expect("parse");
        assert_eq!(chain, FIX_DEST_CHAIN);
        assert_eq!(user, fix_user());
        assert_eq!(amount, FIX_AMOUNT);
        assert_eq!(source_chain, FIX_SOURCE_CHAIN);
        assert_eq!(source_tx, evm::H256::from(FIX_SOURCE_TX));
        assert_eq!(deadline, FIX_DEADLINE);
        assert_eq!(source_evm, FIX_SOURCE_EVM_CHAIN);
        assert_eq!(r, hex32(FIX_R));
        assert_eq!(s, hex32(FIX_S));
        assert_eq!(v, FIX_V);
    }

    #[test]
    fn args_rejects_trailing_and_short_data() {
        let mut long = build_data();
        long.push(0xff);
        assert!(args(&long).is_err());
        let data = build_data();
        assert!(args(&data[..156]).is_err());
        assert!(args(&[]).is_err());
    }
}
