#!/usr/bin/env python3
"""Verify that PIT IRQs advance the kernel timer during boot."""

import shutil
import subprocess
import sys
import tempfile
import time
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
QEMU = shutil.which("qemu-system-x86_64")
OVMF = ROOT / "bios" / "OVMF.4m.fd"
RUST_TOOLCHAIN = os.environ.get("RUST_TOOLCHAIN", "nightly-2026-08-01")


def fail(message):
    raise RuntimeError(message)


def main():
    if not QEMU:
        fail("qemu-system-x86_64 is required for the QEMU timer smoke test")
    if not OVMF.is_file():
        fail(f"missing UEFI firmware: {OVMF}")

    with tempfile.TemporaryDirectory(prefix="agnostos-timer-smoke-") as temporary:
        temporary = Path(temporary)
        boot = temporary / "esp" / "EFI" / "BOOT"
        boot.mkdir(parents=True)
        subprocess.run(
            [
                "cargo", f"+{RUST_TOOLCHAIN}", "build", "--release", "--target", "x86_64-unknown-uefi",
                "--features", "uefi-bin,timer-smoke",
            ], cwd=ROOT, check=True,
        )
        shutil.copy2(
            ROOT / "target" / "x86_64-unknown-uefi" / "release" / "agnostos.efi",
            boot / "BOOTX64.EFI",
        )
        trace = temporary / "timer.trace"
        process = subprocess.Popen(
            [
                QEMU, "-bios", str(OVMF), "-drive", f"format=raw,file=fat:rw:{temporary / 'esp'}",
                "-display", "none", "-serial", "none", "-monitor", "none",
                "-chardev", f"file,id=timertrace,path={trace}",
                "-device", "isa-debugcon,iobase=0xe9,chardev=timertrace",
            ], cwd=ROOT,
        )
        try:
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if trace.exists() and "T\n" in trace.read_text(errors="replace"):
                    print("QEMU timer smoke test passed")
                    return
                time.sleep(0.02)
            output = trace.read_text(errors="replace") if trace.exists() else ""
            fail(f"PIT timer IRQ marker was not observed: {output}")
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError) as error:
        print(f"QEMU timer smoke test failed: {error}", file=sys.stderr)
        sys.exit(1)
