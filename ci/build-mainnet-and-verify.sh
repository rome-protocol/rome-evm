#!/usr/bin/env bash
# Reproducible-build check for the deployed mainnet program.
#
# Clones rome-evm at a pinned ref (default: the audited release tag) and
# mollusk at a pinned commit, builds rome_evm.so with the `mainnet` feature
# using that ref's own ci/Dockerfile, extracts it, and compares it against the
# deployed mainnet program with verify-program.sh --mask (datetime + git hash
# masked, so a same-toolchain rebuild matches despite build-time metadata).
#
# Self-contained: both repos are cloned fresh over public HTTPS into
# ./rome-evm-audited-build in the CURRENT folder (kept after the run, refreshed
# on re-run). No pre-existing local checkout is referenced. Requires docker and
# git; solana-verify, the solana CLI and python3 are needed by verify-program.sh.
#
# Usage:  build-mainnet-and-verify.sh [PROGRAM_ID] [RPC_URL] [OUT_PATH]
#   PROGRAM_ID  default RomePq9X3iAoTr813HR7uRarafFDUJty6GdiEceYpzX (mainnet primary)
#   RPC_URL     default $RPC_URL, else https://api.mainnet-beta.solana.com (public)
#
# Env overrides:
#   REF=<tag|branch|sha>   rome-evm ref to build (default audited-mainnet-2026-09-22,
#                          the audited release tag).
#                          A full 40-char commit SHA is accepted.
#   MOLLUSK_REF=<sha|ref>  mollusk ref (default a1372d3e9ca4750eb44436ea88f0fae922d7b1f6)
#   REPO_URL=<git url>     default https://github.com/rome-protocol/rome-evm.git
#   MOLLUSK_URL=<git url>  default https://github.com/rome-protocol/mollusk.git
#   RPC_URL=<url>          RPC endpoint when not given as the 2nd argument
#   CTX=<dir>              clone/build dir (default ./rome-evm-audited-build)
#   VERIFY=<path>          verify-program.sh (default: the one in the cloned ref)
# Exit: propagates verify-program.sh (0 match, 1 differ, 2 setup error).
set -euo pipefail

REF="${REF:-audited-mainnet-2026-09-22}"
MOLLUSK_REF="${MOLLUSK_REF:-a1372d3e9ca4750eb44436ea88f0fae922d7b1f6}"
FEATURES=mainnet
REPO_URL="${REPO_URL:-https://github.com/rome-protocol/rome-evm.git}"
MOLLUSK_URL="${MOLLUSK_URL:-https://github.com/rome-protocol/mollusk.git}"
PROGRAM_ID="${1:-RomePq9X3iAoTr813HR7uRarafFDUJty6GdiEceYpzX}"
RPC="${2:-${RPC_URL:-https://api.mainnet-beta.solana.com}}"
CTX="${CTX:-$PWD/rome-evm-audited-build}"
REF_TAG=$(printf '%s' "$REF" | tr -c 'A-Za-z0-9._-' '-')
OUT="${3:-$PWD/rome_evm-$REF_TAG-$FEATURES.so}"
IMAGE="rome-evm:$(printf '%s' "$REF_TAG" | tr 'A-Z' 'a-z' | cut -c1-100)-$FEATURES"
MARKER=".rome-evm-audited-build"

command -v docker >/dev/null || { echo "docker not found"; exit 2; }
command -v git    >/dev/null || { echo "git not found"; exit 2; }

# Refuse to delete anything that is not a build dir this script created.
prepare_ctx() {
  local dir="$1"
  case "$dir" in
    ""|"/"|"$HOME"|"$HOME/") echo "refusing to use CTX='$dir'"; exit 2 ;;
  esac
  if [ -e "$dir" ]; then
    if [ -d "$dir" ] && [ -f "$dir/$MARKER" ]; then
      rm -rf "$dir"
    elif [ -d "$dir" ] && [ -z "$(ls -A "$dir")" ]; then
      rmdir "$dir"
    else
      echo "refusing to delete '$dir': it was not created by this script (no $MARKER marker)"
      exit 2
    fi
  fi
  mkdir -p "$dir"
  : > "$dir/$MARKER"
}

# Fetch exactly one ref (tag, branch or full commit SHA) into a fresh checkout.
fetch_ref() {
  local url="$1" ref="$2" dest="$3"
  git init -q "$dest"
  git -C "$dest" remote add origin "$url"
  git -C "$dest" fetch -q --depth 1 origin "$ref"
  git -C "$dest" -c advice.detachedHead=false checkout -q FETCH_HEAD
}

echo ">> clone into current folder: $CTX  (from GitHub; kept after run)"
prepare_ctx "$CTX"
fetch_ref "$REPO_URL"    "$REF"         "$CTX/rome-evm"
fetch_ref "$MOLLUSK_URL" "$MOLLUSK_REF" "$CTX/mollusk"
rm -rf "$CTX/mollusk/target"
SHA=$(git -C "$CTX/rome-evm" rev-parse HEAD)
MOLLUSK_SHA=$(git -C "$CTX/mollusk" rev-parse HEAD)
echo "   rome-evm@$REF -> $SHA"
echo "   mollusk@$MOLLUSK_REF -> $MOLLUSK_SHA"

# verify-program.sh comes from the cloned ref (ci/verify-program.sh).
VERIFY="${VERIFY:-$CTX/rome-evm/ci/verify-program.sh}"
[ -f "$VERIFY" ] || { echo "ci/verify-program.sh is not in $REF of rome-evm (set VERIFY=<path>)"; exit 2; }
chmod +x "$VERIFY"

echo ">> docker build FEATURES=$FEATURES — runs 'cargo test --locked' + SBF build, SLOW"
DOCKER_BUILDKIT=1 docker build --platform linux/amd64 \
  -f "$CTX/rome-evm/ci/Dockerfile" \
  --build-arg FEATURES="$FEATURES" \
  --target build \
  -t "$IMAGE" \
  "$CTX"

echo ">> extract rome_evm.so -> $OUT"
cid=$(docker create "$IMAGE")
docker cp "$cid:/opt/rome-evm/target/deploy/rome_evm.so" "$OUT"
docker rm -f "$cid" >/dev/null
ls -la "$OUT"

echo ">> verify (--mask) against $PROGRAM_ID on $RPC"
exec "$VERIFY" "$PROGRAM_ID" "$OUT" "$RPC" --mask
