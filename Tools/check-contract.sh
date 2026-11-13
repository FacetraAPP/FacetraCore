#!/bin/bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$ROOT"

if command -v rustup >/dev/null 2>&1; then
    export PATH="$(rustup run 1.90.0 rustc --print sysroot)/bin:$PATH"
fi

if [ "${1:-}" = --write ]; then
    cargo run --locked -p facetra-contract --bin generate-schema -- --write
else
    cargo run --locked -p facetra-contract --bin generate-schema
    python3 "$ROOT/Tools/check-contract-baseline.py"
    cargo test --locked -p facetra-contract
    python3 "$ROOT/Tools/check-contract-dependencies.py"
fi
