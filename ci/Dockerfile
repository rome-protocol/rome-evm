# syntax=docker/dockerfile:1.7

# Self-contained rome-evm build: every binary is installed or compiled here, and
# no romeprotocol/* image is inherited. Both the SBF toolchain and the
# test-validator come from the upstream anza release.
#
# Tradeoff: the build downloads GitHub release assets (the anza installer and
# platform-tools) on every cache miss, so it needs reliable access to GitHub.
# Retry on a transient download failure.
#
# amd64-only, and the --platform pins are load-bearing: anza publishes no
# aarch64-unknown-linux-gnu assets for v4.3.0 (installer and tarball both 404),
# so an unpinned build on an arm64 host fails at the installer.

ARG RUST_VERSION=1.93.1
ARG SOLANA_VERSION=v4.3.0
ARG PLATFORM_TOOLS_VERSION=v1.57

# ---------------------------------------------------------------- toolchain --
FROM --platform=linux/amd64 debian:bookworm-slim AS toolchain
ARG RUST_VERSION
ARG SOLANA_VERSION
ARG PLATFORM_TOOLS_VERSION
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get -o Acquire::Retries=5 update -qq && \
    apt-get -o Acquire::Retries=5 install -y -qq --no-install-recommends \
        ca-certificates curl git build-essential pkg-config libssl-dev libssl3 && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

ENV PATH=/root/.cargo/bin:/root/.local/share/solana/install/active_release/bin:$PATH

# Host Rust. RUST_VERSION must track rome-evm's rust-toolchain.toml: if it
# drifts, rustup re-downloads the pinned channel inside the build stage instead.
RUN curl --proto '=https' --tlsv1.2 -sSfL https://sh.rustup.rs | \
        sh -s -- -y --profile minimal --default-toolchain "${RUST_VERSION}" && \
    rustc --version

RUN sh -c "$(curl --proto '=https' --tlsv1.2 -sSfL https://release.anza.xyz/${SOLANA_VERSION}/install)" && \
    solana --version && \
    cargo-build-sbf --version

# Warm platform-tools at an EXPLICIT version. platform-tools rustc, not the host
# Rust above, emits rome_evm.so's bytes — it is the pin that decides the program
# hash. cargo-build-sbf defaults it from the Solana release, so leaving it
# implicit ties the audited binary to a default that moves when SOLANA_VERSION
# does. A throwaway crate is enough to trigger the install.
RUN cargo new --lib /tmp/warmup && \
    printf '\n[lib]\ncrate-type = ["cdylib", "lib"]\n' >> /tmp/warmup/Cargo.toml && \
    (cd /tmp/warmup && cargo-build-sbf --arch v3 --tools-version "${PLATFORM_TOOLS_VERSION}") && \
    rm -rf /tmp/warmup

# An image tag is a label, not a guarantee of the toolchain inside it. This file
# is the truth:
#   docker run --rm <image> cat /etc/rome-toolchain
RUN printf 'solana_version=%s\nplatform_tools_pin=%s\nsolana=%s\ncargo_build_sbf=%s\nrustc=%s\n' \
      "${SOLANA_VERSION}" "${PLATFORM_TOOLS_VERSION}" \
      "$(solana --version 2>&1 | head -1)" \
      "$(cargo-build-sbf --version 2>&1 | tr '\n' ' ')" \
      "$(rustc --version 2>&1)" | tee /etc/rome-toolchain

# -------------------------------------------------------------------- build --
FROM toolchain AS build
ARG FEATURES
ARG PLATFORM_TOOLS_VERSION

# rome-evm and its extracted mollusk crate must be siblings in the build
# context — the path dep `mollusk = { path = "../../mollusk" }` in
# program/Cargo.toml and emulator/Cargo.toml resolves relative to this layout.
#
# .git must survive into the image: program/build.rs shells `git rev-parse HEAD`
# to stamp GIT_HASH, which rome_buildInfo reports. A .dockerignore excluding it
# does not fail the build — it silently bakes an empty hash. Note a
# .dockerignore here would be dead anyway: Docker reads it from the CONTEXT
# root, which is the parent holding both checkouts, not this repo.
COPY rome-evm /opt/rome-evm
COPY mollusk /opt/mollusk

WORKDIR /opt/rome-evm/program
# `ci` is no longer a default feature of the program crate; enable it explicitly
# so the test step runs with the same feature set as before.
RUN cargo test --locked --features ci

# --arch v3 is required: the SBPFv3-required cluster rejects the default v0 output.
# `-- --locked` passes straight through to cargo build (cargo-build-sbf's
# cargo_args), so an audited build cannot silently resolve a dependency set other
# than the committed Cargo.lock.
RUN cargo-build-sbf \
        --features ${FEATURES:-ci},custom-heap --no-default-features \
        --arch v3 --tools-version "${PLATFORM_TOOLS_VERSION}" \
        -- --locked && \
    cd /opt/rome-evm/target/deploy && \
    sha256sum rome_evm.so | tee rome_evm.so.sha256

# ------------------------------------------------------------------ runtime --
FROM --platform=linux/amd64 debian:bookworm-slim AS runtime

LABEL org.opencontainers.image.source="https://github.com/rome-protocol/rome-evm" \
      org.opencontainers.image.title="rome-evm" \
      org.opencontainers.image.description="Rome EVM on-chain program + solana-test-validator harness" \
      org.opencontainers.image.licenses="LicenseRef-Rome-Protocol" \
      org.opencontainers.image.vendor="Rome Protocol"

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get -o Acquire::Retries=5 update -qq && \
    apt-get -o Acquire::Retries=5 upgrade -y -qq --no-install-recommends && \
    apt-get -o Acquire::Retries=5 install -y -qq --no-install-recommends \
        bash ca-certificates libssl3 libudev1 procps && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

# solana-test-validator is the only daemon ci/start_solana.sh runs; `solana` and
# solana-keygen are what an external test harness shells in for airdrops. The
# production agave-validator daemon is not in the anza tarball and is not used
# here, so nothing needs building from agave source.
COPY --from=toolchain /root/.local/share/solana/install/active_release/bin/solana               /usr/local/bin/
COPY --from=toolchain /root/.local/share/solana/install/active_release/bin/solana-test-validator /usr/local/bin/
COPY --from=toolchain /root/.local/share/solana/install/active_release/bin/solana-keygen         /usr/local/bin/
COPY --from=toolchain /etc/rome-toolchain /etc/rome-toolchain

COPY --from=build /opt/rome-evm/target/deploy/rome_evm.so        /opt/
COPY --from=build /opt/rome-evm/target/deploy/rome_evm.so.sha256 /opt/
COPY --from=build /opt/rome-evm/ci /opt/ci

RUN (getent group rome || groupadd --system rome) && \
    (id -u rome >/dev/null 2>&1 || useradd --system --gid rome --create-home --shell /bin/bash rome) && \
    chown -R rome:rome /opt && \
    mkdir -p /test-ledger && chown rome:rome /test-ledger

USER rome:rome

# Pin the working directory to /. The CI test harness copies keypairs in with
# `docker cp <f> <ctr>:./` — which always resolves to / — then reads them back
# with `docker exec <ctr> solana airdrop <amt> ./<f>`, which resolves relative to
# WORKDIR. The two only agree when WORKDIR is /.
WORKDIR /

ENTRYPOINT [ "/opt/ci/start_solana.sh" ]
