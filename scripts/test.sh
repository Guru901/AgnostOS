#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_WORKDIR="$(mktemp -d)"
RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-nightly-2026-08-01}"
trap 'rm -rf "$TEST_WORKDIR"' EXIT

# The repository uses `.cargo/config.toml` to build `core`/`alloc` for the
# UEFI target. Cargo discovers that configuration from the manifest's parent
# directories, even when invoked from another working directory. Host tests
# must therefore run from a clean copy that does not contain `.cargo`.
cp "$PROJECT_ROOT/Cargo.toml" "$PROJECT_ROOT/Cargo.lock" "$TEST_WORKDIR/"
cp -R "$PROJECT_ROOT/src" "$PROJECT_ROOT/tests" "$TEST_WORKDIR/"

if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo is not installed or is not in your PATH."
    exit 1
fi

# Run outside the repository so Cargo does not load its UEFI-only build-std
# configuration. These are host tests; `uefi-bin` is intentionally excluded.
cd "$TEST_WORKDIR"
cargo "+$RUST_TOOLCHAIN" test --features mouse
cargo "+$RUST_TOOLCHAIN" test --no-default-features --features custom-allocator,mouse

if command -v qemu-system-x86_64 >/dev/null 2>&1; then
    python3 "$PROJECT_ROOT/scripts/qemu-input-smoke.py"
    python3 "$PROJECT_ROOT/scripts/qemu-fault-smoke.py"
else
    echo "Skipping QEMU smoke tests: qemu-system-x86_64 is not installed."
fi
