# Code audit

Audit date: 2026-08-10  
Scope: Rust sources under `src/` and `tests/`, checked against `Contributing.md`.

## Executive summary

Framebuffer and heap representations are now opaque, graphics entry points use
domain types, and the riskiest arithmetic and PS/2 wait paths have been
hardened. The remaining work is concentrated in failure handling, keeping
unsafe justification consistently local, and completing the intended module
boundaries.

## Rule compliance

### 1. Every `unsafe` block needs a preceding `// SAFETY:` comment — partly followed

Framebuffer drawing now calls safe owner methods, and its raw pointer accesses
are confined to the framebuffer owner. However, the following call sites still
need an immediately preceding justification that explains Rust's safety
requirements, not merely the device protocol:

- `src/graphics/mod.rs`: raw framebuffer reads, writes, and scroll operations
  are contained, but each raw operation should explicitly tie pointer validity
  and bounds to `is_drawable`/the constructor invariant.
- `src/mouse/mod.rs`: the PS/2 readiness loops and I/O wrappers use unsafe port
  operations. Their surrounding safety contract is documented, but the inner
  `inb`/`outb` blocks do not all carry a local `SAFETY` explanation.
- `src/idt/mod.rs`: interrupt handlers read the PS/2 data port and acknowledge
  the PIC. The handlers rely on their vector/interrupt context and on PIC
  initialization; those invariants need to be stated immediately before the
  I/O blocks.
- `src/commands/shutdown.rs`: the QEMU debug-exit write remains hardware
  specific. Its block needs to state that port `0xf4` is valid only when the
  emulator device is configured.

### 3. No panics, `unwrap`, or `expect` in kernel paths — not followed

- `src/main.rs`: `uefi::helpers::init().unwrap()` can panic during startup.
- `src/uefi_graphics/mod.rs`: GOP discovery and mode setup still use `expect`
  for ordinary firmware failures. These should return a typed error for the
  pre-boot status-reporting path in `main`.
- `src/allocator/mod.rs`: heap initialization uses assertions for repeated
  initialization, missing conventional memory, and invalid heap geometry.
  These are not recoverable after boot-services exit, but should follow a
  deliberate fatal policy rather than panic unwinding/assertion behaviour.
- `src/idt/mod.rs`: the double-fault handler calls `panic!`, which can re-enter
  code that depends on a working stack, console, or allocator. It needs a
  minimal allocation-free halt/report path.

## Safety and correctness risks beyond the explicit rules

- `src/console/mod.rs` and `src/allocator/mod.rs`: the `unsafe impl Send`/
  `Sync` rationale documents the current single-core design and mutex use, but
  SMP is not mechanically prevented. Adding an SMP startup path without
  revisiting framebuffer mapping and allocator ownership would invalidate this
  reasoning. Gate SMP explicitly or introduce subsystem owners that establish
  the required synchronization before secondary cores can run.

## Structure improvements

The current direction is now:

```text
boot/uefi -> platform (ring buffer, interrupts, memory map)
          -> drivers (keyboard, mouse, framebuffer)
          -> display (drawing + text)
          -> console -> shell -> commands
```

- `graphics` now keeps raw framebuffer operations in `Framebuffer` and makes
  drawing use safe pixel methods, but framebuffer ownership/access and pure
  drawing/text routines still live in one large module. Splitting those files
  would make the safe boundary auditable by construction.
- PS/2 controller sequencing is now in `mouse`, with bounded waits and a
  typed error; the IDT only invokes initialization while interrupts are
  disabled.
- Keyboard and mouse now share an internal fixed-capacity `RingBuffer` with a
  documented drop-newest policy and drop counter.
- Command parsing is now separated into `commands::parser` and has a unit
  test. Effects still read console and heap globals directly; introduce a
  narrow command context to make handlers independently testable.
- Global locks and atomics are crate-private, but `KWRITER`, input queues,
  packet state, and heap metadata are still process-lifetime statics. A boot
  context that constructs subsystem owners would make initialization order and
  ownership explicit.
- `main` mostly orchestrates initialization, but firmware/heap/interrupt
  errors are not yet propagated into one pre-boot versus post-boot failure
  policy.

## Verification status

`scripts/test.sh` runs host-safe tests outside the UEFI-only Cargo
`build-std` configuration. It tests both linked-list and custom allocator
configurations with mouse support enabled. At the latest audit, 9 tests passed
in each configuration. `cargo test --all-features` is intentionally not the
host test command: enabling `uefi-bin` builds the no-std UEFI binary for the
host and the repository's global `build-std` setting conflicts with host
`std`'s prebuilt `core`.
