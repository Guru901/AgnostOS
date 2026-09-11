#!/usr/bin/env python3
"""Verify that an invalid opcode reaches AgnostOS's fatal exception path."""

import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
QEMU = shutil.which("qemu-system-x86_64")
OVMF = ROOT / "bios" / "OVMF.4m.fd"


def fail(message):
    raise RuntimeError(message)


def main():
    if not QEMU:
        fail("qemu-system-x86_64 is required for the QEMU fault smoke test")
    if not OVMF.is_file():
        fail(f"missing UEFI firmware: {OVMF}")

    with tempfile.TemporaryDirectory(prefix="agnostos-fault-smoke-") as temporary:
        temporary = Path(temporary)
        boot = temporary / "esp" / "EFI" / "BOOT"
        boot.mkdir(parents=True)
        subprocess.run(
            [
                "cargo", "build", "--release", "--target", "x86_64-unknown-uefi",
                "--features", "uefi-bin,fault-smoke",
            ],
            cwd=ROOT,
            check=True,
        )
        shutil.copy2(
            ROOT / "target" / "x86_64-unknown-uefi" / "release" / "agnostos.efi",
            boot / "BOOTX64.EFI",
        )

        trace = temporary / "fault.trace"
        process = subprocess.Popen(
            [
                QEMU, "-bios", str(OVMF), "-drive", f"format=raw,file=fat:rw:{temporary / 'esp'}",
                "-display", "none", "-serial", "none", "-monitor", "none",
                "-chardev", f"file,id=faulttrace,path={trace}",
                "-device", "isa-debugcon,iobase=0xe9,chardev=faulttrace",
            ],
            cwd=ROOT,
        )
        try:
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if trace.exists() and "F\n" in trace.read_text(errors="replace"):
                    print("QEMU fault smoke test passed")
                    return
                time.sleep(0.02)
            output = trace.read_text(errors="replace") if trace.exists() else ""
            fail(f"invalid opcode did not reach the exception handler: {output}")
        finally:
            process.terminate()
            process.wait()


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError) as error:
        print(f"QEMU fault smoke test failed: {error}", file=sys.stderr)
        sys.exit(1)
