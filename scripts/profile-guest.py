#!/usr/bin/env python3
"""Boot the guest headlessly, type commands over QMP, then shut it down."""

import argparse
import json
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path


def fail(message):
    raise RuntimeError(message)


class Qmp:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(30)
        deadline = time.monotonic() + 15
        while True:
            try:
                self.sock.connect(str(path))
                break
            except (FileNotFoundError, ConnectionRefusedError):
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

    def key_press(self, name):
        key = lambda down: {
            "type": "key",
            "data": {"down": down, "key": {"type": "qcode", "data": name}},
        }
        self.command("input-send-event", {"events": [key(True), key(False)]})

    def close(self):
        self.file.close()
        self.sock.close()


def wait_for_trace(trace, process, marker, timeout, description):
    deadline = time.monotonic() + timeout
    content = ""
    while time.monotonic() < deadline:
        if trace.exists():
            content = trace.read_text(errors="replace")
            if marker in content:
                return content
        if process.poll() is not None:
            fail(f"QEMU exited before {description}; debug trace was:\n{content[-4000:]}")
        time.sleep(0.05)
    fail(f"timed out waiting for {description}; debug trace was:\n{content[-4000:]}")


def qcode_for(character):
    if "a" <= character <= "z" or "0" <= character <= "9":
        return character
    special = {" ": "spc", "-": "minus", ".": "dot", "/": "slash"}
    if character in special:
        return special[character]
    fail(f"cannot type {character!r} through the profile script; use lowercase ASCII commands")


def type_command(qmp, command, key_delay):
    for character in command:
        qmp.key_press(qcode_for(character))
        if key_delay:
            time.sleep(key_delay)
    qmp.key_press("ret")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--esp", type=Path, required=True)
    parser.add_argument("--trace", type=Path, required=True)
    parser.add_argument("--command", action="append", dest="commands")
    parser.add_argument("--startup-timeout", type=float, default=60)
    parser.add_argument("--command-delay", type=float, default=0.3)
    parser.add_argument("--key-delay", type=float, default=0.015)
    args = parser.parse_args()

    qemu = shutil.which("qemu-system-x86_64")
    if not qemu:
        fail("qemu-system-x86_64 is required")
    firmware = Path(__file__).resolve().parent.parent / "bios" / "OVMF.4m.fd"
    if not firmware.is_file():
        fail(f"missing UEFI firmware: {firmware}")
    if not args.commands:
        args.commands = ["taskinfo", "uptime", "history", "font 24"]

    qmp_path = args.trace.with_suffix(".qmp")
    process = subprocess.Popen(
        [
            qemu,
            "-bios", str(firmware),
            "-drive", f"format=raw,file=fat:rw:{args.esp}",
            "-display", "none",
            "-serial", "none",
            "-monitor", "none",
            "-no-reboot",
            "-qmp", f"unix:{qmp_path},server=on,wait=off",
            "-chardev", f"file,id=profiletrace,path={args.trace}",
            "-device", "isa-debugcon,iobase=0xe9,chardev=profiletrace",
            "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
        ]
    )
    qmp = None
    try:
        qmp = Qmp(qmp_path)
        wait_for_trace(
            args.trace,
            process,
            "AGNOSTOS_PROFILE_READY",
            args.startup_timeout,
            "the guest shell to become ready",
        )
        for command in args.commands:
            print(f"Guest command: {command}", flush=True)
            type_command(qmp, command, args.key_delay)
            time.sleep(args.command_delay)
        print("Guest command: shutdown", flush=True)
        type_command(qmp, "shutdown", args.key_delay)
        wait_for_trace(
            args.trace,
            process,
            "AGNOSTOS_PROFILE_END",
            30,
            "the guest profile to be exported",
        )
        process.wait(timeout=10)
    finally:
        if qmp:
            qmp.close()
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, subprocess.SubprocessError) as error:
        print(f"Automated guest profiling failed: {error}", file=sys.stderr)
        sys.exit(1)
