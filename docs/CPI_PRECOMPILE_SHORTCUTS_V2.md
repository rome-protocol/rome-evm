# CpiProgram read shortcuts (v2): design record

> **Status: historical design record.** For the current, authoritative selector list, see [PRECOMPILES.md](PRECOMPILES.md). Of the four primitives discussed below, three shipped on `CpiProgram` (`0xff…08`): `account_u64_at`, `account_lamports` and `pdas_batch_derive`. **`derive_user_ata` was never given a dispatch arm.** The capability it targeted lives on `HelperProgram` (`0xff…09`) as `ata(address,bytes32)` (`0xfeb1c647`). See the STATUS note in `program/src/non_evm/derive_helpers.rs`.
>
> All of these are **reads** (`NonEvmCall::CrossStateEthCall`), not CPIs.

**Builds on:** [CPI_PRECOMPILE_SHORTCUTS.md](CPI_PRECOMPILE_SHORTCUTS.md) (v1: `account_data_at`).

## Goal

Extend the v1 read shortcut with the four primitives v1 deferred, and make the case for each one explicitly.

| # | Primitive | Signature | Estimated saving vs. nearest alternative | Coupling | Shipped? |
|---|---|---|---:|---|---|
| 3 | `account_u64_at` | `(bytes32 pubkey, uint16 offset) → uint64` | ~80–120k vs `account_data_at` + Solidity decode | Solana-generic | Yes (`0xb317d4c1`) |
| 4 | `account_lamports` | `(bytes32 pubkey) → uint64` | ~150k vs `account_info` for existence probes | Solana-generic | Yes (`0xde79ed54`) |
| 5 | `derive_user_ata` | `(address evm_user, bytes32 mint) → bytes32` | ~80k (later measured at ~150k) vs two PDA derivations | Rome + SPL ATA convention | No: see `HelperProgram.ata(address,bytes32)` |
| 6 | `pdas_batch_derive` | `(bytes[][] seed_groups, bytes32 program_id) → (bytes32, uint8)[]` | ~50–80k per PDA in a batch | Solana-generic | Yes (`0x944336f8`) |

All are read-only and add no new precompile address.

## Why they were deferred in v1

v1 applied a strict audit-survival test: does the lifetime saving (CU per call × frequency) outweigh the cost of a permanent ABI commitment? The four primitives here were closer calls. Each saves less per call than v1's primitive, or has spec coupling, or both. What tips each one to "yes" is how often it is called.

## Case for each primitive

### 3. `account_u64_at(pubkey, offset) → uint64`

Sugar over `account_data_at` for the most common shape, a u64 at a known offset. The precompile reads 8 bytes, decodes them as little-endian, and returns the value directly, so Solidity skips its own decode loop.

| Path | CU per wrapper `balanceOf` |
|---|---:|
| `account_info` + EVM Borsh decode | ~322k |
| `account_data_at` + EVM decode | ~220k |
| `account_u64_at` | ~140k |

- **Frequency:** this is on the hottest path. Every wrapped-token balance read, 2× per AMM swap, 4× per `pair.burn`, oracle price reads (also u64), and any Anchor account u64 field.
- **Coupling:** none. The caller supplies the offset.
- **Trade-off:** it is sugar. The same result is possible with `account_data_at` and a Solidity helper; the saving comes from doing the decode in Rust.

### 4. `account_lamports(pubkey) → uint64`

Returns only the account's lamports, skipping the data fetch and the ABI encoding of bytes.

| Path | CU per existence probe |
|---|---:|
| Full `account_info`, then check `lamports != 0` | ~250k |
| `account_data_at(pubkey, 0, 0)` | ~160k |
| `account_lamports` | ~50k |

- **Frequency:** "is this ATA or PDA initialised?" probes on the first transfer to a recipient, bridge pre-flight checks, and adapter initialise-or-deposit gates.
- **Coupling:** none. Lamports is a fundamental account header field.

### 5. `derive_user_ata(evm_user, mint) → bytes32` (not shipped on CpiProgram)

Combines Rome's `EXTERNAL_AUTHORITY` user-PDA derivation with the SPL ATA derivation in one call. It replaces a two-step derivation that runs inside every wrapper `balanceOf`, `transfer`, `transferFrom`, `allowance` and bridge-out.

| Path | CU per ATA derivation |
|---|---:|
| 2× PDA derivation + EVM-side composition | ~180k |
| Single combined derivation | ~100k |

- **Coupling:** the most spec-coupled of the four (the token program and the ATA convention).
- **Outcome:** this capability shipped as `HelperProgram.ata(address)` / `ata(address,bytes32)`, not as a `CpiProgram` selector.

### 6. `pdas_batch_derive(seed_groups[][], program_id) → (pda, bump)[]`

Derives N PDAs (up to 16) against one program in a single dispatch. Generic: any program, any seed shape.

| Path (N = 4) | CU |
|---|---:|
| 4 separate PDA derivations | ~360k |
| One `pdas_batch_derive` | ~200k |

- **Frequency:** contracts that compose existing Solana protocols (DEXes, lending, perps) typically derive 3–6 PDAs per call, such as positions, vaults, fee accounts and LP mints.
- **Coupling:** none.
- It is also the way to derive the **delegatecall-shaped** salted authority PDAs (see [PRECOMPILES.md](PRECOMPILES.md#signing-external_authority-pdas-and-invoke_signed-seeds)).

## Combined projected impact (v1 + v2)

| Operation | Before | v1 only | v1 + v2 |
|---|---:|---:|---:|
| Wallet portfolio render (5 wrapped tokens) | ~1,610k | ~1,160k | ~800k |
| Swap quote (1 pair) | ~644k | ~440k | ~300k |
| Add liquidity | ~2,000k | ~700k | ~500k |
| Remove liquidity (`pair.burn`) | ~2,900k (over budget) | ~1,200k | ~700k |
| Lending market liquidation | ~3,500k (over budget) | ~1,300k | ~900k |

v1 makes the heaviest flows fit; v2 leaves headroom for more protocol complexity.

## Implementation notes

- `account_u64_at` and `account_lamports`: `program/src/non_evm/account_data_extras.rs`.
- `pdas_batch_derive`: `program/src/non_evm/derive_helpers.rs` (outer N ≤ 16).
- All three are wired into `CpiProgram::from_abi` in `program/src/non_evm/cpi.rs` as `CrossStateEthCall`.
- Test coverage: offset overflow, u64-LE round-trip, lamports on funded and zero-lamport accounts, PDA parity with `Pubkey::find_program_address`, rejection of dirty high address bytes, and the batch-length and seed-length caps.

Selectors:

```bash
cast keccak "account_u64_at(bytes32,uint16)"        | head -c 10   # 0xb317d4c1
cast keccak "account_lamports(bytes32)"             | head -c 10   # 0xde79ed54
cast keccak "pdas_batch_derive(bytes[][],bytes32)"  | head -c 10   # 0x944336f8
```
