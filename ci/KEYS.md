# CI test keypairs

The keypairs in this directory are **localnet / CI test keys only**:

| File | Public key | Used for |
|---|---|---|
| `ci/rome-keypair.json` | `CmobH2vR6aUtQ8x4xd1LYNiH6k2G7PFT5StTgWqvy2VU` | program id of the Rome EVM program loaded by `ci/start_solana.sh` into `solana-test-validator` |
| `ci/upgrade-authority-keypair.json` | `8q76RPN5Tm6thVoQAUFhUP2diddGgtDLA6B6eShSazB2` | upgrade authority of that localnet program |

Their private keys are committed to a public repository and are therefore
**publicly known**. Anyone can sign with them.

- Never use them for a real deployment (devnet, testnet or mainnet).
- Never fund them with anything of value on a public cluster.
- Never reuse them as a program id, upgrade authority, registration key or
  operator key outside a throwaway local validator.

The file paths are referenced by other tooling, so the files are kept under
these names.
