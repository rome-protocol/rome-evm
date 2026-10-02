#!/usr/bin/env bash
#
# Regenerate the Token-2022 mint fixtures in ci/dump/.
#
# The validator loads every JSON in this directory at genesis (--account-dir in
# start_solana.sh), so these mints are what the CI stack can exercise. They are
# built by the real spl-token CLI against the real Token-2022 program rather
# than by hand-writing TLV, and the ATA length each one produces is measured off
# an account the real Associated Token Account program created — that measured
# length is the expectation `program/tests/mint_profile.rs` asserts against.
#
# Needs a throwaway validator, not a Rome stack:
#
#   solana-test-validator --reset --ledger /tmp/tv --rpc-port 18899 \
#     --faucet-port 19900 --dynamic-port-range 18010-18050 --quiet &
#   ci/gen-t22-fixtures.sh
#
# Re-running rewrites every file with fresh mint addresses, so only run it when
# a fixture actually has to change; the committed JSON is the fixture.
set -uo pipefail

U=${U:-http://127.0.0.1:18899}
DUMP="$(cd "$(dirname "$0")" && pwd)/dump"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# Fixed so a regenerated hook mint still reads as armed against a known key.
# Not a deployed program: the refusal under test fires before any hook CPI.
HOOK_PID=4MmjxCJqt348WzDf2sCGgtWwpQPAGeAMja5ZQxRwgWhz

# Mint authority is the integration test suite's CI signer (a localnet-only test
# key). Without this the fixtures are read-only in CI — nobody holds the
# authority — so no transfer, balance or delivered-amount test can be written
# against them.
CI_SIGNER=DdyKBQ5cRqH4N95JtMPF2diyyqmo8L9VL2x1siPUL5DW

solana-keygen new --no-bip39-passphrase --force -s -o "$WORK/payer.json" >/dev/null 2>&1
PAYER=$(solana-keygen pubkey "$WORK/payer.json")
solana airdrop 100 "$PAYER" -u "$U" >/dev/null 2>&1 || { echo "airdrop failed — is the validator up at $U?" >&2; exit 1; }

T="spl-token -u $U --fee-payer $WORK/payer.json"

space() {
  solana account "$1" -u "$U" --output json 2>/dev/null \
    | python3 -c 'import sys,json;print(json.load(sys.stdin)["account"]["space"])' 2>/dev/null
}

mint() { # name decimals [create-token flags...]
  local name=$1 dec=$2; shift 2
  local kp="$WORK/$name.json"
  solana-keygen new --no-bip39-passphrase --force -s -o "$kp" >/dev/null 2>&1
  local m; m=$(solana-keygen pubkey "$kp")

  $T create-token --program-2022 --decimals "$dec" --mint-authority "$CI_SIGNER" "$@" -- "$kp" >/dev/null 2>&1
  # Created by the real ATA program, so its length is ground truth, not a guess.
  $T create-account "$m" --owner "$PAYER" >/dev/null 2>&1
  local a; a=$($T address --token "$m" --owner "$PAYER" --verbose 2>/dev/null | awk '/Associated token address/{print $NF}')

  local ml al
  ml=$(space "$m"); al=$(space "$a")
  if [ -z "$ml" ] || [ -z "$al" ]; then echo "FAILED to create $name" >&2; return 1; fi

  solana account "$m" -u "$U" --output-file "$DUMP/spl2022_mint_${name}.json" --output json-compact >/dev/null
  printf '  %-20s mint_len=%-5s ata_len=%-5s %s\n' "$name" "$ml" "$al" "$m"
  echo "    (\"$m\", $dec, HOOK, FEE, $al), // $name" >> "$WORK/consts.txt"
}

echo "Regenerating ci/dump Token-2022 fixtures against $U"
echo "  (ata_len below is what program/tests/mint_profile.rs must reproduce)"
# Transfer-relevant families first — these change what a transfer does.
mint plain              6
mint fee                6 --transfer-fee-basis-points 50 --transfer-fee-maximum-fee 1000000
mint hook_unarmed       6 --enable-transfer-hook
mint hook_armed         6 --transfer-hook "$HOOK_PID"
mint fee_hook           6 --transfer-fee-basis-points 50 --transfer-fee-maximum-fee 1000000 --transfer-hook "$HOOK_PID"
mint nontransferable    6 --enable-non-transferable
mint permanent_delegate 9 --enable-permanent-delegate

# Accounts of this mint are created FROZEN, so the overlay has to reproduce the
# state the ATA program sets and not just its length.
mint frozen_default     6 --enable-freeze --default-account-state frozen
# The only family that adds an account-side extension of its own (PausableAccount),
# so its ATA is 174 rather than 170. It is also the check that our vendored
# spl-token-2022 knows the same extension set the deployed program does: if ata_len
# reads 170 here, the interpreter is behind the chain.
mint pausable           6 --enable-pause
# One display-only family, kept as a regression guard rather than as coverage. The
# stored amount is untouched and Rome reports the stored amount, which is what every
# Solana client treats as the truth; this exists so nobody later "fixes" a balance
# read into a UI amount. `--ui-amount-multiplier 2` makes such a fix visible.
mint scaled_ui          6 --ui-amount-multiplier 2
#
# Deliberately NOT fixtured: interest-bearing (same display-only class as scaled_ui,
# so one of the two is enough), our own metadata mint (ai16z already covers metadata
# for the read-only cases, and nothing needs a fundable metadata mint), mint-close
# authority and freeze-authority-only. None changes a transfer, a balance or an
# account length — all four measured 170, i.e. no information — and each would cost
# a genesis account on every CI run. A freeze authority is worth one fact rather
# than a fixture: it lives in the base layout, so such a mint is still 82 bytes.

echo
echo "Paste-ready rows for the integration test suite's T22_MINTS table"
echo "  (fill HOOK/FEE per row: hook_armed+fee_hook are armed hooks, fee+fee_hook are 50 bps)"
cat "$WORK/consts.txt"
