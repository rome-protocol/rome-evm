# Security audits

Rome EVM has been assessed three times by [Halborn](https://www.halborn.com/).
The executive summary of each engagement is published here in full.

| Report | Engagement | Findings | Outcome |
|---|---|---|---|
| `Rome_EVM_Executive_Summary_Halborn_ONE_2025.pdf` | Apr 28 – Jun 28, 2025 | 9 — 1 critical, 1 medium, 4 low, 3 informational | 6 solved, 1 risk accepted, 2 partially solved |
| `Rome_EVM_Code_Updates_Executive_Summary_Halborn_ONE_2025.pdf` | Jul 29 – Aug 16, 2025 | 4 — no critical or high | 3 solved, 1 risk accepted |
| `Rome_EVM_Diff_Review_Executive_Summary_Halborn_ONE_2026.pdf` | Aug 21 – Sep 5, 2026 | 86 — 0 critical, 1 high, 5 medium, 32 low, 48 informational | every high and medium remediated; 24 solved, 21 risk accepted, 37 acknowledged, 4 partially solved |

## Which revision the reports cover

The reports cite commit ids and pull requests in Rome's internal development
repository, where the reviewed work happened. Those ids are not part of this
repository's history and the links in the reports do not resolve publicly, so
the mapping is recorded here.

The 2026 diff review assessed the program and emulator at `8ceb53a`, together
with Rome's `evm` interpreter fork at `bd199cc1`. Halborn then reviewed the
remediations through `b637afc4`, with `evm` pinned to `8941806` — the same
`evm` revision this repository's `Cargo.lock` pins.

**This repository ships the remediation-reviewed program.** Its `program/` tree
is identical to the remediation-reviewed tree except for comments; no code line
differs. The audited snapshot is tagged
[`audited-mainnet-2026-09-22`](https://github.com/rome-protocol/rome-evm/releases/tag/audited-mainnet-2026-09-22).
Commits on `main` after that tag touch `emulator/`, `docs/` and `audit/` only,
so the statement holds for `main` as well — but the tag is the exact audited
revision, and the build verification below is defined against it.

## Deployed program verification

On September 29, 2026 Halborn independently rebuilt the program with the pinned
toolchain and locked dependencies and compared the result against the program
deployed on Solana mainnet:

```
Program ID       RomePq9X3iAoTr813HR7uRarafFDUJty6GdiEceYpzX
Deployment slot  449766730
Snapshot slot    451579910 (finalized)
```

**The executable code matched byte for byte.** The complete binaries matched
once the compilation timestamp and git revision — the only build-varying
metadata — were masked:

```
SHA-256 (deployed, before masking)  665c967bd9cb61f809dbf97410a94d355ab674f7bc4d9083daadff7da48cf8db
SHA-256 (deployed and rebuilt, after masking)  720b61f436ab22db3185e600e930323ad881fd59d61418726193947af9ed01f2
```

The masking procedure is documented in the diff-review report and implemented
by `ci/verify-program.sh`, which is published here unmodified from the revision
Halborn rebuilt. To reproduce the whole check on a clean machine — fresh clones,
containerised build, on-chain comparison — run:

```sh
ci/build-mainnet-and-verify.sh
```

See [Verifying the Mainnet Build](../README.md#verifying-the-mainnet-build) for
prerequisites and options.
