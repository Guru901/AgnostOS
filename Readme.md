<div align="center">
  <h1>AgnostOS</h1>
  <p>A small UEFI operating system written in Rust, built primarily as a learning project.</p>
</div>


## Prerequisites

You will need:

- A recent Rust toolchain with Cargo and `rustup`.
- The `x86_64-unknown-uefi` Rust target.
- QEMU with the `qemu-system-x86_64` executable available on your `PATH`.
- Bash to run the provided build script.

Install the required Rust target once:

```sh
rustup target add x86_64-unknown-uefi
```

The repository includes the UEFI firmware image required by the run script at
`bios/OVMF.4m.fd`.

## Build and run

From the repository root, run:

```sh
./scripts/build.sh
```

The script builds the release UEFI executable, creates a temporary EFI System
Partition under `esp/`, and launches it in QEMU. Close the QEMU window to stop
the emulator. Generated files in `target/` and `esp/` are ignored by Git.

To build without launching QEMU:

```sh
cargo build --release --target x86_64-unknown-uefi --features uefi-bin
```

The resulting UEFI executable is written to:

```text
target/x86_64-unknown-uefi/release/agnostos.efi
```

## Project status

This is an experimental learning project, not a production operating system.

## Contributing

Contributions are welcome. Please read the [contribution guidelines](Contributing.md)
before opening a change. For a new feature or a large architectural change,
discuss the direction with the maintainers first.

## License

AgnostOS is licensed under the GNU General Public License v3.0 or later (GPL-3.0-or-later).

See the [LICENSE](LICENSE) file for details.
