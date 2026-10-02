# CpiProgram read shortcuts (v1): design record

> **Status: historical design record.** The surface described here has shipped and has since changed. For the current, authoritative list of selectors and how each is dispatched, see [PRECOMPILES.md](PRECOMPILES.md).
>
> **The name "CPI shortcut" is historical and misleading.** `account_data_at`, the read primitive that shipped from this design, is a **pure cross-state read**. It dispatches as `NonEvmCall::CrossStateEthCall` (see the `ACCOUNT_DATA_AT` arm in `program/src/non_evm/cpi.rs`), so it makes no syscall, sets no `found_cpi` flag, and has no iterative-VM restriction. It sits on the `CpiProgram` precompile (`0xff…08`) next to `invoke` / `invoke_signed` for organisational reasons, not because it performs a CPI.
>
> The write-side primitive proposed here, `spl_transfer_checked_v1`, was **not** kept on `CpiProgram`. Its capability lives on `HelperProgram` (`0xff…09`) as the `transfer_spl(*)` family. `spl_transfer_checked_v1` (`0x351aa22f`) is not dispatched by the current program.

## Goal

Add the smallest set of low-level precompile primitives to `CpiProgram` so that Solidity contracts which wrap SPL tokens (AMM pairs, ERC-20 wrappers over SPL mints, oracle adapters, lending markets) fit inside Solana's per-transaction compute budget (1.4M CU), without growing the program's permanent ABI surface or tying Rome to one SPL Token version forever.

## Background: where the compute went

Measurements taken with `solana confirm -v` on a Rome test chain:

| Probe | Total CU |
|---|---:|
| Baseline self-transfer of 1 wei | ~99k |
| Wrapper `ensure_token_account` (ATA already exists) | ~337k |
| Wrapper `transfer(self, 0)`: full Solidity wrapper + 1 SPL CPI | ~787k |
| `CpiProgram.invoke` directly with the same SPL CPI | ~101k |
| AMM `pair.sync` (2× `balanceOf` + update) | ~803k |

Breakdown of the wrapper `transfer` (~687k above baseline):

- SPL Token CPI itself: ~186 CU
- precompile dispatcher: ~2k CU
- **Solidity wrapper scaffolding: ~685k CU**, made up of:
  - `ensure_token_account` fast path (`account_info` + storage): ~238k
  - Borsh-marshalling `AccountMeta[4]` and a 10-byte instruction buffer in EVM bytecode: ~250k
  - delegatecall + EVM-side bridge to the precompile: ~50k
  - argument setup, return decoding, event emission: ~147k

Breakdown of the wrapper `balanceOf` (~322k):

- 2× PDA derivations for the user's ATA: ~160k
- `account_info` returning a 6-tuple: ~120k
- Borsh decoding of the token amount in EVM: ~80k
- entry and dispatch overhead: ~30k

Projected from these figures, an AMM `pair.burn` (4× `balanceOf` + 2× wrapper `transfer` + housekeeping) came to roughly **2.9M CU**, about twice Solana's ceiling. Removing liquidity from any pool with a wrapped-SPL side therefore failed with `exceeded CUs meter`.

## The insight

The SPL Token CPIs themselves cost about 210 CU per `pair.burn`. Almost all of the rest is EVM-side scaffolding around those CPIs: marshalling `AccountMeta` arrays in Solidity, ABI-encoding 6-tuples, and running bit-shift decode loops in EVM bytecode. Moving that work into Rust makes it effectively free.

## The proposal: two primitives

Both were proposed as new selectors on the existing `CpiProgram` precompile. Neither adds a new address, and both are backward-compatible with existing callers of `invoke`, `invoke_signed` and `account_info`.

| # | Primitive | Signature | Estimated saving | Coupling |
|---|---|---|---:|---|
| 1 | `account_data_at` | `(bytes32 pubkey, uint16 offset, uint16 length) → bytes` | ~60–100k per read | Fully generic: any Solana account, any field |
| 2 | `spl_transfer_checked_v1` | `(bytes32 src_ata, bytes32 mint, bytes32 dst_ata, uint64 amount, uint8 decimals, bytes32[] salts) → bool` | ~500k per transfer | Versioned: SPL Token only |

The `_v1` suffix made the token-program coupling explicit. A Token-2022 variant would get a separate selector and would never be a branch inside this one.

## Why only two: the audit-survival test

Each candidate primitive had to clear three bars:

1. **Net positive over its lifetime:** CU saved × call frequency must outweigh the audit and maintenance cost of a permanent ABI commitment.
2. **Not tied to a spec likely to change**, or, if tied to one, versioned so that an upgrade path exists.
3. **Demonstrated need:** a measured hot path or a clear architectural blocker, not a guess.

Four other primitives were considered and initially deferred: `account_u64_at`, `account_lamports`, `derive_user_ata` and `pdas_batch_derive`. The reasons were that they were sugar, niche, coupled to the ATA convention, or speculative respectively. The follow-up design in [CPI_PRECOMPILE_SHORTCUTS_V2.md](CPI_PRECOMPILE_SHORTCUTS_V2.md) revisited them.

## Expected impact

With both primitives in place, the projections were:

| Operation | Before | After |
|---|---:|---:|
| Wallet portfolio render (5 wrapped tokens, `balanceOf` reads) | ~1,610k | ~1,160k |
| Swap quote (1 pair, 2× `balanceOf`) | ~644k | ~440k |
| Add liquidity (2 transfers + mint) | ~2,000k | ~700k |
| **Remove liquidity (`pair.burn`)** | **~2,900k (over budget)** | **~1,200k (fits)** |
| Lending market liquidation | ~3,500k (over budget) | ~1,300k (fits) |

## Implementation notes

- `account_data_at` is implemented in `program/src/non_evm/account_data.rs` and wired into `CpiProgram::from_abi` / `cross_state_call` in `program/src/non_evm/cpi.rs` as `CrossStateEthCall`.
- Each selector needs the same handling in the emulator so that `eth_call` and gas estimation agree with on-chain execution. The emulator compiles the same `non_evm` code.
- Selectors are `keccak256(signature)[..4]`:

```bash
cast keccak "account_data_at(bytes32,uint16,uint16)" | head -c 10   # 0x593762e8
```
