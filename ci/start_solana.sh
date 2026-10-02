#!/usr/bin/env bash
set -euo pipefail
# ci/rome-keypair.json and ci/upgrade-authority-keypair.json are publicly known
# localnet/CI test keys only; see ci/KEYS.md. Never use them for real deployments.

mkdir -p /test-ledger
: > /test-ledger/validator.log

VALIDATOR=(
  --reset
  --warp-slot 1
  --log-messages-bytes-limit 100000
  --ticks-per-slot 16
  --upgradeable-program /opt/ci/rome-keypair.json /opt/rome_evm.so /opt/ci/upgrade-authority-keypair.json
  --account-dir /opt/ci/dump
  --limit-ledger-size 400000000
  --faucet-sol 500000000
  --ledger /test-ledger
)

solana-test-validator "${VALIDATOR[@]}" >> /test-ledger/validator.log 2>&1 &
pid=$!

# Wait briefly for startup; fail fast if process dies
for _ in $(seq 1 20); do
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "solana-test-validator exited early"
    cat /test-ledger/validator.log
    exit 1
  fi
  sleep 0.5
done

exec tail -n +1 -F /test-ledger/validator.log
