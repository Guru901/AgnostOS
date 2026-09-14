#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

EFI_BINARY="$PROJECT_ROOT/target/x86_64-unknown-uefi/release/agnostos.efi"
OUTPUT_ISO="${1:-$PROJECT_ROOT/target/agnostos.iso}"
WORK_DIR="$(mktemp -d)"
ESP_IMAGE="$WORK_DIR/esp.img"
ISO_ROOT="$WORK_DIR/iso-root"

cleanup() {
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Error: $1 is required but was not found in PATH." >&2
        exit 1
    fi
}

require_command cargo
require_command mkfs.fat
require_command mmd
require_command mcopy
require_command mkisofs

cd "$PROJECT_ROOT"

echo "Building the UEFI executable..."
cargo +nightly build \
    --release \
    --target x86_64-unknown-uefi \
    --features uefi-bin

if [[ ! -f "$EFI_BINARY" ]]; then
    echo "Error: UEFI executable was not produced at $EFI_BINARY" >&2
    exit 1
fi

mkdir -p "$(dirname "$OUTPUT_ISO")"
mkdir -p "$ISO_ROOT"

echo "Creating the FAT EFI System Partition..."
dd if=/dev/zero of="$ESP_IMAGE" bs=1M count=64 status=none
mkfs.fat -F 32 -n AGNOSTOS "$ESP_IMAGE" >/dev/null
mmd -i "$ESP_IMAGE" ::/EFI ::/EFI/BOOT
mcopy -i "$ESP_IMAGE" "$EFI_BINARY" ::/EFI/BOOT/BOOTX64.EFI
cp "$ESP_IMAGE" "$ISO_ROOT/esp.img"

echo "Creating UEFI ISO at $OUTPUT_ISO..."
mkisofs \
    -quiet \
    -o "$OUTPUT_ISO" \
    -V AGNOSTOS \
    -eltorito-platform 0xEF \
    -eltorito-boot esp.img \
    -no-emul-boot \
    "$ISO_ROOT"

echo "Created $OUTPUT_ISO"
