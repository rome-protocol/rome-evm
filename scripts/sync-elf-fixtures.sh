#!/usr/bin/env bash
# Re-fetch the vendored deployed-program snapshots that t22_differential
# measures against, and regenerate their manifest. With --check, compare
# instead of write.
#
# The harness byte-compares Rome's in-process vendored processors against these
# ELFs. A snapshot whose upstream has moved turns those assertions into
# statements about history, so this is the ONLY sanctioned way to update them —
# and the diff must be reviewed alongside any change in the harness's
# expectations.
#
# --check is meant for a drift-detection job. It lives here so that the drift
# alarm and the generator compute the hash with the same code: an alarm that
# hashes differently from the generator reports drift that isn't there, or
# misses drift that is, and either way it is worse than no alarm.
#
# Usage:
#   scripts/sync-elf-fixtures.sh --rpc <url>            # re-fetch + rewrite manifest
#   scripts/sync-elf-fixtures.sh --check --rpc <url>    # compare only, write nothing
# The RPC must serve the cluster the manifest was fetched from (Solana devnet).
set -euo pipefail

DIR="$(cd "$(dirname "$0")/.." && pwd)/program/tests/fixtures/elf"
RPC="${RPC:-}"
CHECK=0

while [ $# -gt 0 ]; do
  case "$1" in
    --rpc)   RPC="${2:?--rpc needs a url}"; shift 2 ;;
    --check) CHECK=1; shift ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$RPC" ]; then
  echo "No --rpc given (or RPC env var); pass the Solana devnet RPC URL explicitly." >&2
  echo "Refusing to guess an endpoint." >&2
  exit 1
fi

declare -a NAMES=(token2022 tokenkeg ata)
declare -a IDS=(
  TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb
  TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
  ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL
)

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# sha256 of a file, base58-encoded — the format solana_program::hash::Hash
# renders, which is what program/tests/elf_fixtures.rs compares against.
sha58() {
  python3 - "$1" <<'PY'
import hashlib, sys
A = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
b = hashlib.sha256(open(sys.argv[1], 'rb').read()).digest()
n = int.from_bytes(b, 'big')
s = ''
while n:
    n, r = divmod(n, 58)
    s = A[r] + s
print('1' * (len(b) - len(b.lstrip(b'\0'))) + s)
PY
}

dump() {
  # </dev/null so `solana` cannot eat the manifest the caller is reading.
  solana program dump "$1" "$2" --url "$RPC" >/dev/null </dev/null
}

if [ "$CHECK" = 1 ]; then
  FAIL=0
  SEEN=0
  while read -r NAME PID WANT; do
    case "$NAME" in ''|'#'*) continue ;; esac
    SEEN=$((SEEN + 1))
    dump "$PID" "$TMP/$NAME.so"
    GOT="$(sha58 "$TMP/$NAME.so")"
    if [ "$GOT" != "$WANT" ]; then
      echo "::error::$NAME ($PID) drifted — manifest $WANT, deployed $GOT"
      FAIL=1
    else
      echo "$NAME ok"
    fi
  done < "$DIR/MANIFEST.txt"

  if [ "$SEEN" = 0 ]; then
    echo "::error::MANIFEST.txt lists no programs — an empty manifest checks nothing" >&2
    exit 1
  fi
  if [ "$FAIL" = 1 ]; then
    echo "::error::A vendored ELF no longer matches its deployed program. The differential harness is now measuring history. Re-fetch with scripts/sync-elf-fixtures.sh and re-read its expectations."
    exit 1
  fi
  echo "$SEEN program(s) match the manifest"
  exit 0
fi

for i in "${!NAMES[@]}"; do
  echo "dumping ${NAMES[$i]} (${IDS[$i]})"
  dump "${IDS[$i]}" "$TMP/${NAMES[$i]}.so"
done

{
  echo "# Vendored deployed-program snapshots that t22_differential measures against."
  echo "# Fetched from Solana devnet $(date -u +%Y-%m-%d) (the cluster Rome devnet + testnet both settle to)."
  echo "#"
  echo "# name        program_id                                     sha256 (base58, solana_program::hash)"
  echo "# Update via scripts/sync-elf-fixtures.sh — never by hand."
  echo
  for i in "${!NAMES[@]}"; do
    cp "$TMP/${NAMES[$i]}.so" "$DIR/${NAMES[$i]}.so"
    printf '%-12s %-44s %s\n' "${NAMES[$i]}" "${IDS[$i]}" "$(sha58 "$DIR/${NAMES[$i]}.so")"
  done
} > "$DIR/MANIFEST.txt"

echo
echo "Updated $DIR/MANIFEST.txt"
echo "Now: cargo test --test elf_fixtures --test t22_differential"
echo "and review whether the harness's byte-equality expectations still hold."
