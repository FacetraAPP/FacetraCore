#!/bin/bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cd "$ROOT"

if command -v rustup >/dev/null 2>&1; then
    export PATH="$(rustup run 1.90.0 rustc --print sysroot)/bin:$PATH"
fi

cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
Tools/check-contract.sh
cargo test --locked -p facetra-contract -p facetra-render
cargo test --release --locked -p facetra-pipeline -j 4

if [ "${FACETRA_TEST_RENDER:-}" = 1 ]; then
    cargo test --locked -p facetra-render \
        renders_a_png_when_explicitly_enabled -- --nocapture
fi
