#!/usr/bin/env bash
# Verify a locally-built rome_evm.so against the on-chain deployed program.
#
# Usage:
#   verify-program.sh <PROGRAM_ID> <path/to/rome_evm.so> [RPC_URL] [--mask]
#
#   default        exact compare: get-program-hash (on-chain) vs
#                  get-executable-hash (local file). Matches only when the
#                  deployed bytes ARE this build.
#   --mask         dump the program, zero out the build-varying metadata
#                  (COMPILE_DATETIME + GIT_HASH from the ROME_BUILD_INFO blob,
#                  every copy), then compare. Use when the two were built at
#                  different times / git states but the SAME toolchain.
#                  It does NOT hide a different compiler — that is a real
#                  code difference and must fail.
#
# Exit: 0 match, 1 differ, 2 usage/tooling error.
set -euo pipefail

PROGRAM_ID="${1:?PROGRAM_ID required}"
SO="${2:?path to rome_evm.so required}"
RPC="${3:-localhost}"
MODE="${4:-}"

command -v solana-verify >/dev/null || { echo "solana-verify not found"; exit 2; }
[ -f "$SO" ] || { echo "file not found: $SO"; exit 2; }

if [ "$MODE" != "--mask" ]; then
  ON=$(solana-verify get-program-hash   "$PROGRAM_ID" -u "$RPC")
  LOC=$(solana-verify get-executable-hash "$SO")
  echo "on-chain : $ON"
  echo "local    : $LOC"
  [ "$ON" = "$LOC" ] && { echo "MATCH"; exit 0; }
  echo "DIFFER"; exit 1
fi

TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
solana program dump "$PROGRAM_ID" "$TMP/onchain.so" -u "$RPC" >/dev/null

mask() {  # $1 = src .so, $2 = dst .so ; zeroes datetime + git (all copies)
  python3 - "$1" "$2" <<'PY'
import sys
d = open(sys.argv[1], "rb").read()
s = d.find(b"<<ROME_BUILD_INFO>>"); e = d.find(b"<<END_ROME_BUILD_INFO>>")
if s != -1 and e != -1:
    p = d[s:e].split(b"\n")          # marker, version, git, rustc, datetime, features
    for f in (p[4], p[2]):           # datetime, git (skip if empty)
        if f:
            d = d.replace(f, b"0" * len(f))
open(sys.argv[2], "wb").write(d)
PY
}

mask "$TMP/onchain.so" "$TMP/onchain.masked.so"
mask "$SO"             "$TMP/local.masked.so"
ON=$(solana-verify get-executable-hash "$TMP/onchain.masked.so")
LOC=$(solana-verify get-executable-hash "$TMP/local.masked.so")
echo "on-chain masked : $ON"
echo "local    masked : $LOC"
[ "$ON" = "$LOC" ] && { echo "MATCH (datetime+git masked)"; exit 0; }
echo "DIFFER (real code/toolchain difference — masking cannot hide it)"; exit 1
