#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUTPUT="${1:-$PROJECT_ROOT/flamegraph.html}"
RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-nightly-2026-08-01}"

for command in cargo qemu-system-x86_64 python3; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Error: $command is required and must be on PATH." >&2
        exit 1
    fi
done
if [[ ! -f "$PROJECT_ROOT/bios/OVMF.4m.fd" ]]; then
    echo "Error: missing bios/OVMF.4m.fd" >&2
    exit 1
fi

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
ESP="$WORK_DIR/esp"
TRACE="$WORK_DIR/profile.trace"
mkdir -p "$ESP/EFI/BOOT"

cd "$PROJECT_ROOT"
echo "Building a manually instrumented profiling image..."
cargo "+$RUST_TOOLCHAIN" build --release --target x86_64-unknown-uefi \
    --features uefi-bin,profiling
cp target/x86_64-unknown-uefi/release/agnostos.efi "$ESP/EFI/BOOT/BOOTX64.EFI"

echo "Starting AgnostOS. Only functions marked with #[profiled] are recorded."
echo "Type 'shutdown' in the guest to export the recorded calls and exit QEMU."
set +e
qemu-system-x86_64 \
    -bios ./bios/OVMF.4m.fd \
    -drive "format=raw,file=fat:rw:$ESP" \
    -chardev "file,id=profiletrace,path=$TRACE" \
    -device isa-debugcon,iobase=0xe9,chardev=profiletrace \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04
QEMU_STATUS=$?
set -e

if [[ ! -f "$TRACE" ]] || ! grep -q '^AGNOSTOS_PROFILE_END$' "$TRACE"; then
    echo "No complete profile was exported. Shut down from the guest with 'shutdown'." >&2
    exit 1
fi

python3 "$SCRIPT_DIR/render-flamegraph.py" \
    "$TRACE" "$OUTPUT"
echo "Created $OUTPUT (QEMU exit status: $QEMU_STATUS)"
