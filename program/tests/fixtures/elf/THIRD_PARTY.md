# Third-party program binaries

The `.so` files in this folder are unmodified builds of Solana Program Library programs,
downloaded from Solana devnet (see `MANIFEST.txt` for program IDs and hashes). They are used
only as test fixtures; they are not part of the Rome EVM program.

| File | Program | Source | License |
|---|---|---|---|
| `tokenkeg.so` | SPL Token | <https://github.com/solana-program/token> | Apache-2.0 |
| `token2022.so` | SPL Token-2022 | <https://github.com/solana-program/token-2022> | Apache-2.0 |
| `ata.so` | SPL Associated Token Account | <https://github.com/solana-program/associated-token-account> | Apache-2.0 |

These files are distributed under the Apache License, Version 2.0 (see `LICENSE-APACHE-2.0`
in this folder), not under this repository's LICENSE. All rights in them remain with their
original authors.
