# Project improvement audit

Audit date: 2026-09-18

This is a working backlog from a repository-wide pass over the Rust modules,
tests, scripts, CI workflow, and project documentation. Items are ordered by
risk and dependency. Completed items remain here so future work has context.

## Completed on this branch

- [x] Make host tests independent of the UEFI `build-std` Cargo configuration.
  The previous `scripts/test.sh` and CI commands still loaded
  `.cargo/config.toml` and failed with a duplicate `core` lang item. Host
  tests now run from a clean temporary project copy.
- [x] Make the QEMU skip message accurate for both smoke tests.
- [x] Remove stale task-module documentation that claimed context preparation
  had not been added.
- [x] Pin the repository and helper scripts to the CI nightly toolchain, while
  allowing `RUST_TOOLCHAIN` overrides for local experimentation. Document the
  formatter and shared host-test entry point for contributors.
- [x] Add a QEMU timer smoke test. The `timer-smoke` feature emits `T\n` on
  the debug console after ten PIT IRQs; the test treats that marker as its
  expected success signal and keeps the feature out of normal builds.

## Backlog

### High priority

- [ ] Add an explicit `cargo fmt --check` and host-test entry point to the
  contributor workflow, and make CI use the pinned toolchain consistently in
  `scripts/test.sh` rather than the moving `nightly` alias.
- [ ] Add a QEMU timer smoke test and document its expected exit signal. The
  timer conversion is unit-tested, but boot-time IRQ delivery and the
  `uptime` command are not verified by the current smoke tests.
- [ ] Complete CPU exception coverage for page faults, general protection,
  divide errors, invalid opcodes, alignment checks, and machine checks. The
  current fault smoke path covers only invalid opcode handling.
- [ ] Audit page-table activation on real hardware: validate every mapped
  range against the memory map, add cache-policy handling for MMIO, and test
  failure paths before enabling CR3.
- [ ] Replace the public raw `FrameAddress` release path with ownership-aware
  handles or allocation metadata. At present, a caller can construct an
  aligned address and attempt to return a frame it never owns.

### Medium priority

- [ ] Expand `meminfo` with allocator usage, largest free range, and frame
  allocation failures; make diagnostics usable before all subsystems exist.
- [ ] Bound and report shell input growth. The shell currently appends to an
  owned `String` without a user-visible maximum or an allocation-failure
  policy.
- [ ] Define a narrow device/console/timer interface so PS/2, PIC, PIT, UEFI,
  and raw port I/O do not leak into higher-level policy.
- [ ] Add an idle abstraction with an interrupt-safe wake-up contract and
  document which locks and operations are legal in IRQ context.
- [ ] Give task stacks an explicit guard/reservation policy and move them from
  fixed static storage to owned physical frames when the scheduler grows.
- [ ] Add negative tests for malformed/overlapping memory descriptors and
  frame-range overflow; preserve invariants after failed reservations.

### Lower priority

- [ ] Add a read-only filesystem or boot archive path before implementing
  writable storage.
- [ ] Add CI coverage for the ISO builder and both allocator feature modes,
  including the required host tools where the runner supports them.
- [ ] Normalize project documentation spelling, capitalization, and naming;
  several older files contain typos and describe completed work inconsistently.

## Verification baseline

For each completed item, run formatting, both host-test feature sets, the UEFI
release build, and the relevant QEMU smoke test when the required tools are
installed. Keep hardware-facing behavior behind deterministic smoke tests and
keep pure parsing, allocator, scheduler, and rendering behavior in host tests.
