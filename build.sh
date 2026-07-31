#!/bin/bash
set -euo pipefail

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

if [[ ! -f ./bios/OVMF.4m.fd ]]; then
    echo "Error: ./bios/OVMF.4m.fd not found."
    exit 1
fi


cargo build --release
mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-uefi/release/agnostos.efi esp/EFI/BOOT/BOOTX64.EFI
qemu-system-x86_64 -bios ./bios/OVMF.4m.fd -drive format=raw,file=fat:rw:esp
