# Security Policy

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues, pull requests or discussions.**

Report them privately through GitHub's private vulnerability reporting:

1. Go to the **Security** tab of this repository.
2. Click **Report a vulnerability**.
3. Describe the issue, the affected component and version or commit, and steps or a proof of concept to reproduce it. If you know the impact, include it.

We will acknowledge your report and keep you informed as we investigate. Please give us reasonable time to investigate and remediate before any public disclosure, and do not exploit the issue on mainnet or against other users' funds or data.

## Scope

In scope:

| Component | Location |
|---|---|
| Rome EVM on-chain program, Solana mainnet-beta: `RomePq9X3iAoTr813HR7uRarafFDUJty6GdiEceYpzX` | `program/` |
| Off-chain emulator (used for `eth_call` and gas estimation) | `emulator/` |
| Build-metadata macros | `macros/` |
| Reproducible-build tooling | `ci/Dockerfile`, `ci/build-mainnet-and-verify.sh`, `ci/verify-program.sh` |

Out of scope:

- The contracts in `solidity/`. They are unaudited reference/example code and are not intended for production use.
- The keypairs in `ci/` (`ci/rome-keypair.json`, `ci/upgrade-authority-keypair.json`). These are **publicly known localnet/CI test keys** by design (see [`ci/KEYS.md`](ci/KEYS.md)), so their disclosure is not a vulnerability.
- The documented EVM semantics differences listed under "EVM Semantics Divergences" in the [README](README.md#evm-semantics-divergences), such as the absence of a gas schedule and the 2300-gas stipend. These are known design properties. A report showing an impact beyond what is documented is still welcome.
- Third-party dependencies, unless the issue is exploitable through Rome EVM. Report other dependency issues upstream.

## Audit

The mainnet program was audited by Halborn. The audit report is available on request through the same private reporting channel.

The deployed mainnet binary corresponds to the audited source at tag `audited-mainnet-2026-09-22`. Anyone can check this with `ci/build-mainnet-and-verify.sh`; see "Verifying the Mainnet Build" in the [README](README.md#verifying-the-mainnet-build).

## Upgrade authority

The upgrade authority of the mainnet program is a Squads multisig:
[`DML4U1jMpWCu3uFiVxQTtXLzor1wtSRLRaEczXKqEz4n`](https://app.squads.so/squads/DML4U1jMpWCu3uFiVxQTtXLzor1wtSRLRaEczXKqEz4n/home).

No single key can upgrade the deployed program.

## Bug bounty

There is currently no bug bounty program, and no reward is promised for reports. We appreciate responsible disclosure and, with your permission, are happy to credit reporters.
