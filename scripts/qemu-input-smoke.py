#!/usr/bin/env python3
"""Exercise the real QEMU PS/2 keyboard and mouse paths.

The guest is built with the `input-smoke` feature, which writes one compact
record per input byte at the IRQ queue boundary to QEMU's debug console.  The
script drives the QMP input API; it does not call any kernel test hook.
"""

import json
import shutil
import socket
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


class Qmp:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        deadline = time.monotonic() + 10
        while True:
            try:
                self.sock.connect(str(path))
                break
            except FileNotFoundError:
                if time.monotonic() >= deadline:
                    fail("QEMU did not open its QMP socket")
                time.sleep(0.05)
        self.file = self.sock.makefile("rwb")
        self._read()
        self.command("qmp_capabilities")

    def _read(self):
        line = self.file.readline()
        if not line:
            fail("QMP connection closed unexpectedly")
        return json.loads(line)

    def command(self, name, arguments=None):
        request = {"execute": name}
        if arguments is not None:
            request["arguments"] = arguments
        self.file.write(json.dumps(request).encode() + b"\n")
        self.file.flush()
        while True:
            response = self._read()
            if "return" in response:
                return response["return"]
            if "error" in response:
                fail(f"QMP {name} failed: {response['error']}")

    def events(self, events):
        self.command("input-send-event", {"events": events})

    def close(self):
        self.file.close()
        self.sock.close()


def key(name, down):
    return {"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": name}}}


def key_press(qmp, name):
    qmp.events([key(name, True), key(name, False)])


def wait_for(log, predicate, description, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        data = log.read_text(errors="replace") if log.exists() else ""
        if predicate(data):
            return data
        time.sleep(0.02)
    fail(f"timed out waiting for {description}; debug trace was:\n{data}")


def assert_subsequence(records, expected, name):
    position = 0
    for record in records:
        if position < len(expected) and record == expected[position]:
            position += 1
    if position != len(expected):
        fail(f"{name}: expected ordered bytes {expected}, got {records}")


def main():
    if not QEMU:
        fail("qemu-system-x86_64 is required for the QEMU input smoke test")
    if not OVMF.is_file():
        fail(f"missing UEFI firmware: {OVMF}")

    with tempfile.TemporaryDirectory(prefix="agnostos-input-smoke-") as temp:
        temp = Path(temp)
        esp = temp / "esp"
        boot = esp / "EFI" / "BOOT"
        boot.mkdir(parents=True)

        subprocess.run(
            [
                "cargo", "build", "--release", "--target", "x86_64-unknown-uefi",
                "--features", "uefi-bin,input-smoke",
            ],
            cwd=ROOT,
            check=True,
        )
        shutil.copy2(
            ROOT / "target" / "x86_64-unknown-uefi" / "release" / "agnostos.efi",
            boot / "BOOTX64.EFI",
        )

        qmp_path = temp / "qmp.sock"
        trace = temp / "input.trace"
        process = subprocess.Popen(
            [
                QEMU, "-bios", str(OVMF), "-drive", f"format=raw,file=fat:rw:{esp}",
                "-display", "none", "-serial", "none", "-monitor", "none",
                "-qmp", f"unix:{qmp_path},server=on,wait=off",
                "-chardev", f"file,id=inputtrace,path={trace}",
                "-device", "isa-debugcon,iobase=0xe9,chardev=inputtrace",
            ],
            cwd=ROOT,
        )
        qmp = None
        try:
            qmp = Qmp(qmp_path)
            wait_for(trace, lambda text: "R\n" in text, "the guest input driver")

            # A normal key, Shift modifier, Ctrl modifier, and extended arrow
            # keys. Exact set-1 bytes verify their order at the IRQ boundary.
            key_press(qmp, "a")
            qmp.events([key("shift", True), key("a", True), key("a", False), key("shift", False)])
            qmp.events([key("ctrl", True), key("c", True), key("c", False), key("ctrl", False)])
            for arrow in ("up", "down", "left", "right"):
                key_press(qmp, arrow)
            trace_text = wait_for(trace, lambda text: text.count("K") >= 26, "keyboard input")
            records = [line for line in trace_text.splitlines() if line.startswith("K")]
            assert_subsequence(
                records,
                [
                    "K1e", "K9e", "K2a", "K1e", "K9e", "Kaa",
                    "K1d", "K2e", "Kae", "K9d",
                    "Ke0", "K48", "Ke0", "Kc8", "Ke0", "K50", "Ke0", "Kd0",
                    "Ke0", "K4b", "Ke0", "Kcb",
                    "Ke0", "K4d", "Ke0", "Kcd",
                ],
                "keyboard keys, modifiers, arrows, and ordering",
            )

            # QEMU serializes PS/2 events through its one-byte controller
            # buffer, so a host-side burst cannot outpace this guest's shell.
            # A real B make code activates the smoke-build-only deterministic
            # ring-buffer fill; D proves the live IRQ queue's overflow branch.
            key_press(qmp, "b")
            wait_for(trace, lambda text: "D\n" in text, "keyboard queue overflow")

            # Relative motion plus a button travels through QEMU's PS/2 mouse
            # device. Validate a complete, non-overflow packet reached IRQ12.
            qmp.events([
                {"type": "rel", "data": {"axis": "x", "value": 4}},
                {"type": "rel", "data": {"axis": "y", "value": -2}},
                {"type": "btn", "data": {"down": True, "button": "left"}},
            ])
            trace_text = wait_for(trace, lambda text: text.count("M") >= 3, "mouse input")
            mouse_bytes = [int(line[1:], 16) for line in trace_text.splitlines() if line.startswith("M")]
            if not any(
                packet[0] & 0x08 and not packet[0] & 0xC0 and (packet[1] or packet[2])
                for packet in zip(mouse_bytes, mouse_bytes[1:], mouse_bytes[2:])
            ):
                fail(f"mouse packet was not valid PS/2 motion: {mouse_bytes}")
        finally:
            if qmp:
                try:
                    qmp.command("quit")
                except (OSError, RuntimeError):
                    pass
                qmp.close()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()

    print("QEMU input smoke test passed")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError) as error:
        print(f"QEMU input smoke test failed: {error}", file=sys.stderr)
        sys.exit(1)
