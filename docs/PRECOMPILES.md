# Precompiles and Solana Interop Surface

This document is the reference for every precompile address the Rome EVM program dispatches, the selectors each one accepts, and how each selector is executed (a pure read, or a real Solana cross-program invocation).

Source of truth is the code: the address constants live in `program/src/non_evm/*_ix.rs` and `program/src/non_evm_cached/*.rs`, the selector constants and dispatch arms live in the `Program::from_abi` / `from_abi_cached` implementations of the same modules, and the address-to-handler routing lives in `program/src/state/handler_non_evm.rs`. Selectors are `keccak256(canonical_signature)[..4]`; every selector in the tables below was checked against both `cast keccak` and the byte constants in the source file listed for that precompile.

---

## Contents

- [Two families of precompiles](#two-families-of-precompiles)
- [Dispatch variants (`NonEvmCall`)](#dispatch-variants-nonevmcall)
- [`CrossStateEthCall` is not a Solana CPI](#crossstateethcall-is-not-a-solana-cpi)
- [Cached track vs. legacy track](#cached-track-vs-legacy-track)
- [Signing: `EXTERNAL_AUTHORITY` PDAs and `invoke_signed` seeds](#signing-external_authority-pdas-and-invoke_signed-seeds)
- [Value transfers to precompiles](#value-transfers-to-precompiles)
- [Address map](#address-map)
- [Cached track](#cached-track)
  - [SystemCached `0xff…04`](#systemcached-0xff04)
  - [SplCached `0xff…05`](#splcached-0xff05)
  - [ASplCached `0xff…06`](#asplcached-0xff06)
  - [WithdrawCached `0xff…0b`](#withdrawcached-0xff0b)
- [Legacy track](#legacy-track)
  - [System `0xff…07`](#system-0xff07)
  - [CpiProgram `0xff…08`](#cpiprogram-0xff08)
  - [HelperProgram `0xff…09`](#helperprogram-0xff09)
  - [Withdraw `0x42…16`](#withdraw-0x4216)
- [Selectors that are not dispatched](#selectors-that-are-not-dispatched)
- [Off-chain emulation](#off-chain-emulation)

---

## Two families of precompiles

1. **Ethereum-standard precompiles** (`program/src/precompile/`): ecrecover, SHA-256, RIPEMD-160, identity, BN254 add/mul/pairing and BLAKE2f at their Ethereum addresses `0x01`-`0x09`. These are pure, deterministic crypto. `modexp` (`0x05`) is present but disabled: a call to it reverts with `big_mod_exp call is disabled` (`program/src/vm/vm.rs`). See the main [README](../README.md#precompiles) for the table.
2. **Rome Solana precompiles** (`program/src/non_evm/` and `program/src/non_evm_cached/`): eight fixed addresses that turn EVM calldata into Solana account reads or Solana instructions. They are the subject of the rest of this document.

Every Rome precompile implements the `Program` trait (`program/src/non_evm/mod.rs`). `from_abi()` (or `from_abi_cached()` for cached-track programs) parses the calldata and returns a `NonEvmCall` variant. That variant determines what happens next.

## Dispatch variants (`NonEvmCall`)

| Variant | What it does |
|---|---|
| `Precompiled(input)` | Deterministic crypto; touches no Solana state (Ethereum-standard precompiles). |
| `EthCall(selector, abi)` | Pure computation or a read of Rome's own state (PDA derivation, program IDs, base58 conversion). |
| `CrossStateEthCall(selector, abi)` | Pure read of another Solana account's bytes that is already loaded in the transaction. |
| `Invoke(ix, seeds)` | One Solana instruction, executed as a CPI and signed by the caller's `EXTERNAL_AUTHORITY` PDA when seeds are present. |
| `Composed(IxList)` | Several Solana instructions plus EVM-side balance diffs, executed together (for example burn-then-transfer for a withdrawal). |

## `CrossStateEthCall` is not a Solana CPI

The `Dispatch` column in the tables below decides the side-effect model. Two variants look similar but behave very differently:

| Dispatch variant | Behaviour | `invoke` / `invoke_signed` syscall? | Sets the `found_cpi` flag? | Allowed in the iterative VM? | Shows up as `Program X invoke [N]` in logs? |
|---|---|---|---|---|---|
| `Invoke` / `Composed` (legacy track) | Real Solana CPI that can change external program state. | Yes | Yes (`found_cpi`) | **No** (fails with `CpiProhibitedInIterativeTx`) | Yes |
| `Invoke` / `Composed` (cached track) | Staged against an in-memory overlay during execution; the queued instructions are issued with `invoke_signed` when the transaction commits. | Yes, at commit | Sets `found_cpi_cached` | Yes | Yes, at commit |
| `EthCall` / `Precompiled` | Pure computation or a read of Rome's own state. | No | No | Yes | No |
| `CrossStateEthCall` | Pure read of a pre-loaded Solana account. No syscall, no side effects. | **No** | **No** | **Yes** | **No** |

The precompile at `0xff…08` is named `CpiProgram` because it began as a CPI dispatcher. The read selectors that were later added to it (`account_info`, `account_data_at`, `account_u64_at`, `account_lamports`, `pdas_batch_derive`) are sometimes called "CPI shortcuts", but **they do not perform a CPI**. They dispatch as `CrossStateEthCall`. Only `invoke` and `invoke_signed` on that address are real CPIs.

In practice, a contract can read Solana state (an oracle account, a token account balance) through `CpiProgram` or `HelperProgram` reads and still run in the iterative VM. Those reads never set `found_cpi`.

## Cached track vs. legacy track

There are two tracks of mutating precompiles:

- **Legacy track:** `System` (`0xff…07`), `CpiProgram` (`0xff…08`), `HelperProgram` (`0xff…09`), `Withdraw` (`0x42…16`). Mutating calls execute as immediate CPIs. They are only allowed in atomic execution.
- **Cached track:** `SystemCached` (`0xff…04`), `SplCached` (`0xff…05`), `ASplCached` (`0xff…06`), `WithdrawCached` (`0xff…0b`). Mutating calls are emulated against an overlay (`NonEvmState`, carried on the journal), and the resulting Solana instructions are queued and issued at the end of the transaction. This gives EVM-revert atomicity over SPL effects and works in the iterative VM. The implementation lives in `program/src/non_evm_cached/`.

**Mutual exclusion is enforced at runtime** by `verify_call` in `program/src/state/handler_non_evm.rs`:

- After a legacy-track `Invoke`/`Composed` has run in a transaction, any cached-track `Invoke`/`Composed` fails, and the reverse also applies.
- After a cached-track `Invoke`/`Composed` has run, a `CrossStateEthCall` read from a **legacy-track** precompile also fails. Do all legacy reads first, or use the cached-track read selectors (`SplCached.account*`, `SplCached.mint_info`).
- The flags are sticky for the whole transaction, not per call frame. A track that fired inside a frame that later reverted still locks the transaction.

The recommended practice is that **a contract commits to one track**, and that a migration from legacy to cached is a contract redeploy, not a runtime branch.

## Signing: `EXTERNAL_AUTHORITY` PDAs and `invoke_signed` seeds

Each EVM address has a Solana "external authority" PDA derived from the `EXTERNAL_AUTHORITY` seed and the address. Precompiles that act for the caller sign as that PDA. For `CpiProgram.invoke_signed`, the caller passes a `bytes32[]` of salts. Each element is one PDA's salt, and Rome builds the seed and appends the bump itself (do not include the bump). The seed prefix depends on how the precompile was reached (`program/src/non_evm/cpi_ix.rs`):

- **Direct `CALL`:** `[EXTERNAL_AUTHORITY, caller, salt]`, and the bare `external_auth(caller)` PDA may sign.
- **`DELEGATECALL` / `CALLCODE`:** `[EXTERNAL_AUTHORITY, caller, running_contract, salt]`, and the bare `external_auth(caller)` PDA is refused (`DelegatecallOwnerAuthority`). A contract acting on behalf of a user gets its own namespace inside that user's PDA space and cannot reach the user's main PDA, its ATAs, or another contract's salted PDAs.

`HelperProgram.pda_with_salt` returns only the direct-call shape. Use `CpiProgram.pdas_batch_derive` to derive the delegatecall shape.

## Value transfers to precompiles

Every Rome precompile method rejects `msg.value > 0` (`not_payable`, error `TransferProhibited`), with one exception: `withdrawal(bytes32)` on `Withdraw` (`0x42…16`) and `WithdrawCached` (`0xff…0b`) is payable and consumes the attached value.

## Address map

| Address | Name | Track | Source |
|---|---|---|---|
| `0xff00000000000000000000000000000000000004` | SystemCached | cached | `program/src/non_evm_cached/system_cached.rs` |
| `0xff00000000000000000000000000000000000005` | SplCached | cached | `program/src/non_evm_cached/spl_cached.rs` |
| `0xff00000000000000000000000000000000000006` | ASplCached | cached | `program/src/non_evm_cached/aspl_cached.rs` |
| `0xff00000000000000000000000000000000000007` | System | legacy (reads only) | `program/src/non_evm/system.rs`, `system_ix.rs` |
| `0xff00000000000000000000000000000000000008` | CpiProgram | legacy | `program/src/non_evm/cpi.rs`, `cpi_ix.rs` |
| `0xff00000000000000000000000000000000000009` | HelperProgram | legacy | `program/src/non_evm/helper.rs`, `helper_ix.rs` |
| `0xff0000000000000000000000000000000000000b` | WithdrawCached | cached | `program/src/non_evm_cached/withdraw_cached.rs` |
| `0x4200000000000000000000000000000000000016` | Withdraw | legacy | `program/src/non_evm/withdraw.rs`, `withdraw_ix.rs` |

`0xff…0a` is not assigned. Calls to it are not routed to any precompile.

---

## Cached track

### SystemCached `0xff…04`

PDA creation and System Program operations (allocate, assign, lamport transfer), staged into the overlay.

| Solidity signature | Selector | Dispatch | Description |
|---|---|---|---|
| `create_pda()` | `0xe0402a8d` | `Invoke` | Create the caller's `external_auth` PDA, rent-exempt, zero data |
| `create_pda(uint64)` | `0x4ceab657` | `Invoke` | Same, funded with the given lamports |
| `create_pda(uint64,bytes32)` | `0x48e2bb86` | `Invoke` | Salt-derived PDA with lamports |
| `create_pda(bytes32,uint64,bytes32)` | `0xcc258bbf` | `Invoke` | Owner-specified, size-N, salt-derived PDA |
| `allocate(uint64,bytes32)` | `0x93225c9f` | `Invoke` | Allocate N bytes on a salt-derived PDA |
| `assign(bytes32,bytes32)` | `0x8ac00bdc` | `Invoke` | Assign a salt-derived PDA to a new owner |
| `transfer(address,uint64)` | `0x5d359fbd` | `Invoke` | Lamport transfer to an EVM-address-derived PDA |
| `transfer(bytes32,uint64)` | `0xfd54d1ea` | `Invoke` | Lamport transfer to a raw Solana pubkey |
| `transfer(bytes32,uint64,bytes32)` | `0x875abfc0` | `Invoke` | Lamport transfer to a raw pubkey from a salt-derived source |

### SplCached `0xff…05`

SPL Token / Token-2022 transfers, approvals, mints and account initialisation staged into the overlay, plus overlay-aware account reads.

| Solidity signature | Selector | Dispatch | Description |
|---|---|---|---|
| `transfer(address,uint256)` | `0xa9059cbb` | `Invoke` | Caller-as-owner transfer to an EVM address's PDA-owned ATA, chain gas mint |
| `transfer(bytes32,uint256)` | `0x6a467394` | `Invoke` | Same, recipient given as a raw Solana pubkey |
| `transfer(address,uint256,bytes32)` | `0x57cfeeee` | `Invoke` | Explicit-mint variant, EVM-address destination |
| `transfer(bytes32,uint256,bytes32)` | `0x7db527f9` | `Invoke` | Explicit-mint variant, raw-pubkey destination |
| `transferFrom(address,address,uint256,bytes32)` | `0x401e3367` | `Invoke` | Authority is `external_auth(caller)`, accepted as the source ATA's owner or as a delegate with sufficient allowance |
| `approve(address,uint256,bytes32)` | `0x8180f2fc` | `Invoke` | Owner `external_auth(caller)` approves delegate `external_auth(spender)` |
| `mint(address,uint256,bytes32)` | `0x1e458bee` | `Invoke` | Caller PDA signs as mint authority; the token program enforces the authority match |
| `init(bytes32,bytes32,bytes32)` | `0x0b0ad508` | `Invoke` | `InitializeAccount3` (ata, mint, owner) |
| `account(address)` | `0x73b9aa91` | `CrossStateEthCall` | Token account state for an EVM address's PDA-derived ATA (chain gas mint) |
| `account(bytes32)` | `0x882358ae` | `CrossStateEthCall` | Token account state for a raw ATA pubkey |
| `account(address,bytes32)` | `0xf9827227` | `CrossStateEthCall` | Same as `account(address)` with an explicit mint |
| `mint_info(bytes32)` | `0xe24bf5d4` | `CrossStateEthCall` | Overlay-aware mint facts: token program, decimals, armed transfer-hook program (zero when absent or inert), current-epoch fee bps, extension-presence bitmap. Same selector as `HelperProgram.mint_info`, so a cached-track contract does not need the legacy read |

`SplCached` has no raw-pubkey-delegate approve. Flows that need one use the legacy `HelperProgram.approve_spl_raw_delegate`.

### ASplCached `0xff…06`

Idempotent Associated Token Account creation.

| Solidity signature | Selector | Dispatch | Description |
|---|---|---|---|
| `create_ata()` | `0xb6d336ed` | `Invoke` | ATA for caller PDA × chain gas mint |
| `create_ata(bytes32)` | `0x81972e35` | `Invoke` | ATA for caller PDA × explicit mint |
| `create_ata(address)` | `0x5a7c3259` | `Invoke` | ATA for an EVM address's PDA × chain gas mint |
| `create_ata(address,bytes32)` | `0x3de2251a` | `Invoke` | ATA for an EVM address's PDA × explicit mint |

`ASplCached` has no raw-pubkey-owner variant. Flows that need one use the legacy `HelperProgram.create_ata_for_key`.

### WithdrawCached `0xff…0b`

Native gas withdrawal legs, staged. This is the cached counterpart of `Withdraw` (`0x42…16`).

| Solidity signature | Selector | Dispatch | Description |
|---|---|---|---|
| `withdrawal(bytes32)` payable | `0x4d8b0ea4` | `Composed` | Burn `msg.value` of native gas and pay the Solana system-account recipient |
| `withdraw_to_pda(uint256)` | `0x7f3124a0` | `Composed` | Burn gas and deposit lamports to the caller's `EXTERNAL_AUTHORITY` PDA |
| `withdraw_to_ata(uint256)` | `0x8059abc0` | `Composed` | Burn gas and deposit the SPL gas token to the caller's PDA-owned ATA |
| `deposit(uint256)` | `0xb6b55f25` | `Composed` | Inverse of `withdraw_to_ata`: move the SPL gas token from the caller's PDA-owned ATA to the chain's wallet ATA and mint the equivalent native gas to the caller |

---

## Legacy track

### System `0xff…07`

PDA derivation, program IDs and base58 utilities. Every method is a pure `EthCall` with no Solana side effects.

| Solidity signature | Selector | Dispatch | Description |
|---|---|---|---|
| `find_program_address(bytes32,(bytes)[])` | `0x27e3edda` | `EthCall` | Solana `find_program_address`: returns `(pda, bump)` |
| `create_program_address(bytes32,(bytes)[],uint8)` | `0xb99f29e1` | `EthCall` | PDA derivation with a known bump |
| `rome_evm_program_id()` | `0xb76fd45b` | `EthCall` | This Rome EVM program's Solana program ID |
| `program_id()` | `0x77764881` | `EthCall` | This precompile's identity |
| `mint_id()` | `0xe132a122` | `EthCall` | The chain's gas-token SPL mint |
| `operator()` | `0x570ca735` | `EthCall` | The Solana signer of the outer transaction |
| `bytes32_to_base58(bytes32)` | `0xfa2b1a5f` | `EthCall` | Pubkey bytes to base58 string |
| `base58_to_bytes32(bytes)` | `0x5df01b72` | `EthCall` | base58 string to pubkey bytes |

### CpiProgram `0xff…08`

A dual-purpose precompile: **2 selectors perform a real CPI** and **5 are pure cross-state reads**. For the design history of the read selectors, see [CPI_PRECOMPILE_SHORTCUTS.md](CPI_PRECOMPILE_SHORTCUTS.md) and [CPI_PRECOMPILE_SHORTCUTS_V2.md](CPI_PRECOMPILE_SHORTCUTS_V2.md).

| Solidity signature | Selector | Dispatch | Description |
|---|---|---|---|
| `invoke(bytes32,(bytes32,bool,bool)[],bytes)` | `0x7480cb86` | `Invoke` | CPI: the caller supplies the target program, account metas and instruction data. The caller's bare authority PDA may sign only on a direct call |
| `invoke_signed(bytes32,(bytes32,bool,bool)[],bytes,bytes32[])` | `0xb94f3733` | `Invoke` | CPI signed by salt-derived authority PDAs (see [Signing](#signing-external_authority-pdas-and-invoke_signed-seeds)) |
| `account_info(bytes32)` | `0xc13465d9` | `CrossStateEthCall` | Full account state (lamports, owner, flags, data) |
| `account_data_at(bytes32,uint16,uint16)` | `0x593762e8` | `CrossStateEthCall` | A slice of an account's data |
| `account_u64_at(bytes32,uint16)` | `0xb317d4c1` | `CrossStateEthCall` | Little-endian u64 at an offset |
| `account_lamports(bytes32)` | `0xde79ed54` | `CrossStateEthCall` | Lamports only |
| `pdas_batch_derive(bytes[][],bytes32)` | `0x944336f8` | `CrossStateEthCall` | Up to 16 PDAs against one program in one call |

The target instruction of `invoke` / `invoke_signed` is checked before it runs (`CpiProgram::verify` in `cpi_ix.rs`).

### HelperProgram `0xff…09`

User-facing helpers: ATA and PDA creation; SPL transfers, approvals and mints signed by the caller's `EXTERNAL_AUTHORITY` PDA; gas-to-lamports conversion; ATA deposit; and ERC-20-style reads. Every method is non-payable.

| Solidity signature | Selector | Dispatch | Description |
|---|---|---|---|
| `create_ata(address)` | `0x5a7c3259` | `Invoke` | ATA for the user's PDA under the chain gas mint |
| `create_ata(address,bytes32)` | `0x3de2251a` | `Invoke` | ATA for the user's PDA under an explicit mint |
| `create_ata_for_key(bytes32,bytes32)` | `0xd258a69d` | `Invoke` | Idempotent ATA for an arbitrary raw Solana owner pubkey |
| `create_pda(address)` | `0xff3556ca` | `Invoke` | Create the user's `EXTERNAL_AUTHORITY` PDA (rent-exempt) |
| `create_pda(address,uint64)` | `0x58e88298` | `Invoke` | Same, funded with extra lamports |
| `swap_gas_to_lamports(uint64)` | `0x6e3f24e0` | `Invoke` | Burn native gas and credit lamports to the caller's PDA |
| `transfer_lamports(address,uint64)` | `0x5fe71665` | `Invoke` | Lamports from the caller's PDA to another user's PDA |
| `transfer_spl(address,uint64)` | `0xb12be5ba` | `Invoke` | Chain-gas SPL from the caller PDA's ATA to the destination user's ATA |
| `transfer_spl(bytes32,uint64)` | `0xba3a5eac` | `Invoke` | Same, to an explicit ATA |
| `transfer_spl(address,uint64,bytes32)` | `0x53b505e0` | `Invoke` | Explicit-mint SPL to the destination user's ATA |
| `transfer_spl(bytes32,uint64,bytes32)` | `0xb6977879` | `Invoke` | Explicit-mint SPL to an explicit ATA |
| `transfer_spl(bytes32,bytes32,uint64,bytes32)` | `0x766b362a` | `Invoke` | Caller-supplied source ATA; the caller PDA must be its owner or a sufficient delegate |
| `transfer_spl(address,address,uint64,bytes32)` | `0xe479df56` | `Invoke` | Address-keyed delegate variant: derives both ATAs, signs as `external_auth(caller)` (owner or delegate) |
| `transfer_spl_to_signer(uint64,bytes32)` | `0x46efa679` | `Invoke` | Caller PDA's ATA to the outer Solana signer's own ATA for that mint |
| `approve_spl(address,uint64,bytes32)` | `0xabf6f675` | `Invoke` | Caller PDA approves the spender's PDA via `approve_checked` |
| `approve_spl_raw_delegate(bytes32,bytes32,uint64,bytes32,uint8)` | `0x7881d453` | `Invoke` | `approve_checked` with a raw-pubkey delegate; the caller supplies decimals (SPL Token only) |
| `mint_spl(address,uint64,bytes32)` | `0xd795522b` | `Invoke` | Caller PDA as mint authority mints to the destination user's ATA |
| `create_mint_account(bytes32)` | `0xe97d3291` | `Invoke` | System `CreateAccount` for a salt-derived mint PDA owned by SPL Token |
| `init_spl_mint(bytes32,uint8,bytes32,bool,bytes32)` | `0x4f75e987` | `Invoke` | SPL `InitializeMint2` on a pre-allocated mint |
| `create_and_init_mint(uint8,bytes32,bool,bytes32,bytes32)` | `0x20972d0f` | `Composed` | `CreateAccount` + `InitializeMint2` in one call |
| `deposit_from_ata(uint256)` | `0x4479b709` | `Composed` | Pull the SPL gas token from the caller's ATA into native gas |
| `pda(address)` | `0x8854a299` | `EthCall` | The user's `EXTERNAL_AUTHORITY` PDA |
| `pda_with_salt(address,bytes32)` | `0x5c6d04b3` | `EthCall` | Salt-derived authority PDA (direct-call shape) |
| `ata(address)` | `0x31db4f82` | `EthCall` | ATA of the user's PDA for the chain gas mint |
| `ata(address,bytes32)` | `0xfeb1c647` | `EthCall` | ATA of the user's PDA for an explicit mint |
| `user_balance(address,bytes32)` | `0xdd0119c8` | `CrossStateEthCall` | The user's ATA `amount`; 0 if the ATA does not exist |
| `allowance_of(address,address,bytes32)` | `0xed72dbc8` | `CrossStateEthCall` | The owner ATA's `delegated_amount`, returned only when the on-chain delegate equals `external_auth(spender)`, otherwise 0 |
| `mint_info(bytes32)` | `0xe24bf5d4` | `CrossStateEthCall` | Mint facts (token program, decimals, armed hook program, current-epoch fee bps, extension bitmap). `feeBps` indicates that a fee is armed; it is not the fee amount |

### Withdraw `0x42…16`

Native gas withdrawal from Rome to Solana.

| Solidity signature | Selector | Dispatch | Description |
|---|---|---|---|
| `withdrawal(bytes32)` payable | `0x4d8b0ea4` | `Composed` | Burn `msg.value` of native gas and pay a Solana system-account recipient. The only payable legacy method |
| `withdraw_to_pda(uint256)` | `0x7f3124a0` | `Composed` | Burn gas and deposit to the caller's `EXTERNAL_AUTHORITY` PDA |
| `withdraw_to_ata(uint256)` | `0x8059abc0` | `Composed` | Burn gas and deposit the SPL gas token to the caller PDA's ATA |

---

## Selectors that are not dispatched

The following were proposed or shipped in earlier designs and are **not** accepted by the current program. A call fails with `Unimplemented`:

- `spl_transfer_checked_v1(...)` on `CpiProgram` (`0x351aa22f`) and `derive_user_ata(address,bytes32)` on `CpiProgram`. Their capabilities live on `HelperProgram` (`transfer_spl(*)` and `ata(address,bytes32)`).
- The 4-argument Token-2022 variants `approve_spl(address,uint64,bytes32,bytes32)` (`0xc9884b1e`), `mint_spl(address,uint64,bytes32,bytes32)` (`0x406ee21b`) and `transfer_spl(address,address,uint64,bytes32,bytes32)` (`0x7b11c48f`) on `HelperProgram`.
- An Ed25519 verification precompile formerly at `0xff…0a`.

## Off-chain emulation

The `emulator` crate compiles the same `non_evm` and `non_evm_cached` code and runs real Solana programs (SPL Token, Token-2022, Associated Token Account and arbitrary upgradeable programs) through the [`mollusk`](https://github.com/rome-protocol/mollusk) SVM harness. Their ELFs are loaded from chain at emulation time, so `eth_call` and gas estimation see the same precompile surface as on-chain execution. When adding or changing a selector, keep the program and emulator in lockstep, and lock the selector constant to `keccak256(signature)[..4]` with a unit test (the pattern in `program/src/non_evm/helper.rs`).
