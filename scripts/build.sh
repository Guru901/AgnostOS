#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if ! command -v qemu-system-x86_64 >/dev/null 2>&1; then
    echo "Error: qemu-system-x86_64 is not installed or is not in your PATH."
    echo
    echo "Install QEMU and try again."
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo is not installed or is not in your PATH."
    echo
    echo "Install CARGO and try again."
    exit 1
fi

if [[ ! -f "$PROJECT_ROOT/bios/OVMF.4m.fd" ]]; then
    echo "Error: $PROJECT_ROOT/bios/OVMF.4m.fd not found."
    exit 1
fi

"$SCRIPT_DIR/test.sh"

cd "$PROJECT_ROOT"
cargo build --release --target x86_64-unknown-uefi --features uefi-bin
mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-uefi/release/agnostos.efi esp/EFI/BOOT/BOOTX64.EFI
exec qemu-system-x86_64 -bios ./bios/OVMF.4m.fd -drive format=raw,file=fat:rw:esp -device isa-debug-exit,iobase=0xf4,iosize=0x04
