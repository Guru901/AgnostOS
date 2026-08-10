#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_WORKDIR="$(mktemp -d)"
trap 'rm -rf "$TEST_WORKDIR"' EXIT

if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo is not installed or is not in your PATH."
    exit 1
fi

# Run outside the repository so Cargo does not load its UEFI-only build-std
# configuration. These are host tests; `uefi-bin` is intentionally excluded.
cd "$TEST_WORKDIR"
cargo +nightly test --manifest-path "$PROJECT_ROOT/Cargo.toml" --features mouse
cargo +nightly test --manifest-path "$PROJECT_ROOT/Cargo.toml" --no-default-features --features custom-allocator,mouse
