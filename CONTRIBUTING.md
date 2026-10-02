# Contributing

Thank you for your interest in Rome EVM.

## Pull requests

**External pull requests are not accepted at this time.** This repository is published as source-available so that the code running on-chain can be read, built and verified. Development happens internally, and the program here corresponds to an audited, deployed binary (see below). Pull requests opened against this repository will be closed without review.

## Bug reports

Bug reports are welcome. Open a GitHub issue that includes:

- what you did (commands, transaction, contract, or a minimal reproduction)
- what you expected and what happened instead
- the commit or tag you were using, and the network or cluster if relevant

**Security issues must not be reported as public issues.** Follow [SECURITY.md](SECURITY.md) instead.

## License

This software is source-available under the terms in [LICENSE](LICENSE): free for personal, non-commercial use only. Commercial use requires written permission from Rome Protocol.

## Developer guide

You are welcome to build and test the code locally for personal, non-commercial purposes.

### Layout

The `mollusk` crate is a required path dependency (`../../mollusk` from `program/` and `emulator/`), so it must be checked out next to this repository:

```bash
mkdir rome && cd rome
git clone https://github.com/rome-protocol/rome-evm.git
git clone https://github.com/rome-protocol/mollusk.git
# rome/
# ├── rome-evm/
# └── mollusk/
```

### Toolchain

- Rust 1.93.1 (pinned in `rust-toolchain.toml`; rustup selects it automatically)
- Solana / Agave CLI v4.3.0 with `cargo-build-sbf`
- SBF platform-tools v1.57

The exact environment is defined in [`ci/Dockerfile`](ci/Dockerfile).

### Network feature

No network feature is enabled by default. Every build and test must pass exactly one of `ci`, `testnet` or `mainnet`. Use `ci` for local work. It selects the devnet/CI registration key and is meant for local and test use only.

### Build

```bash
cd rome-evm/program
cargo build-sbf --no-default-features --features ci,custom-heap \
  --arch v3 --tools-version v1.57 -- --locked
# output: rome-evm/target/deploy/rome_evm.so
```

Host builds are fine for development, but they are not byte-reproducible. To reproduce a deployed binary, use the Docker build (`ci/Dockerfile`) as described in the README.

### Test

```bash
cd rome-evm
cargo test --workspace --locked --features rome-evm/ci,emulator/ci
```

This runs the unit tests in `program/src` and `emulator/src` and the integration tests in `program/tests/` and `emulator/tests/`. Some of these execute real SBF programs (vendored ELFs in `program/tests/fixtures/`) through the `mollusk` SVM harness.

To run a local validator with the program preloaded, build the runtime image from the parent directory:

```bash
docker build --platform linux/amd64 -f rome-evm/ci/Dockerfile --build-arg FEATURES=ci -t rome-evm:local .
docker run --rm -p 8899:8899 rome-evm:local
```

The keypairs in `ci/` are public localnet/CI test keys. Never use them anywhere else (see [`ci/KEYS.md`](ci/KEYS.md)).

### Changes to the program alter the audited binary

The mainnet program is built from `program/`, `emulator/`, `macros/`, `Cargo.lock` and `rust-toolchain.toml` with a pinned toolchain, and the result is checked against the audited release (`audited-mainnet-2026-09-22`). **Any change to those inputs, including dependency bumps, toolchain or platform-tools changes, and feature-flag changes, produces a binary that differs from the audited, deployed one** and would need its own review and audit before deployment. Keep this in mind when you experiment with a fork: a locally modified build is not the audited program.

Comment-only edits under `program/` can change the binary too: panic and assert messages embed source line numbers, so adding or removing a line shifts every location after it. Keep comment edits line-for-line, and re-run the mainnet verification after any change to `program/`.

When changing behaviour, also keep in mind:

- The program and the emulator share the instruction list (`entrypoint!` in `program/src/lib.rs` and `emulator/src/lib.rs`). A handler's position is its dispatch byte, so slots are never reordered, and new slots are only appended.
- Precompile selectors must equal `keccak256(signature)[..4]`. [`docs/PRECOMPILES.md`](docs/PRECOMPILES.md) documents the full surface.
