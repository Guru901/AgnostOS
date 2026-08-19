# Kernel readiness checklist

## Short answer

AgnostOS can already be called a small kernel prototype: it is a `no_std`
x86_64 UEFI program that takes control after UEFI boot services, installs a
heap, configures an IDT/PIC path, handles keyboard input, renders to a
framebuffer, and runs a shell.

It is not yet a complete operating-system kernel. The biggest missing
foundations are an owned physical-memory model, page tables, complete CPU fault
handling, an idle/timer design, and a task/process model. Add those before
building substantial storage or user-program support.

## What is already present

- UEFI entry point and GOP framebuffer discovery.
- Explicit boot-services hand-off.
- Global heap allocator, with linked-list and experimental custom backends.
- IDT setup and legacy 8259 PIC support.
- PS/2 keyboard interrupts and optional mouse support.
- Text console, shell command parsing, history, and QEMU shutdown.
- Host-side unit tests for several data structures and rendering/input paths.

## Required kernel foundations

### 1. Preserve the memory map

- [ ] Copy or transform the memory map returned by `exit_boot_services` into
  kernel-owned data before discarding it.
- [ ] Classify usable, reserved, firmware, kernel, framebuffer, and ACPI
  ranges.
- [ ] Reserve the kernel image, boot data, page tables, stacks, and devices.
- [ ] Expose memory-map diagnostics from a safe kernel API.

The current allocator selects the largest conventional region and turns it
into a heap. That is useful for the prototype, but it loses ownership
information needed by paging, DMA, drivers, and future processes.

### 2. Add physical and virtual memory management

- [ ] Implement a 4 KiB physical-frame allocator with allocation and release.
- [ ] Define the x86_64 virtual-address layout and its ownership rules.
- [ ] Create page-table mapping/unmapping primitives with checked alignment and
  permission flags.
- [ ] Deliberately map the kernel, heap, framebuffer, device memory, stacks,
  and boot data.
- [ ] Add guard pages and a controlled out-of-memory path.
- [ ] Put the heap on top of the page/frame allocator instead of permanently
  owning one arbitrary conventional-memory range.

This is the most important missing boundary: without paging, the kernel has no
strong separation between valid memory, device memory, and accidental pointer
accesses.

### 3. Complete CPU exception handling

- [ ] Add handlers for page fault, general protection, invalid opcode, divide
  error, stack-segment, alignment-check, and machine-check exceptions.
- [ ] Print the vector, error code, instruction pointer, stack pointer, and
  page-fault address when those values are available.
- [ ] Define which faults halt the machine and which can be recovered.
- [ ] Keep exception handlers allocation-free and safe when the console itself
  is damaged.

Do not unmask additional device IRQs until their IDT entry, acknowledgement,
shared-state rules, and failure behaviour are defined.

### 4. Make interrupts and time dependable

- [ ] Add a timer source and calibrate its tick-to-time conversion.
- [ ] Make `uptime` report real units; currently the timer API treats ticks as
  milliseconds.
- [ ] Implement an idle path using `hlt` without a check-then-sleep race.
- [ ] Track interrupt nesting and document which locks/operations are legal in
  interrupt context.
- [ ] Replace or formally verify the mutex-backed interrupt queues and expose
  overflow diagnostics.
- [ ] Introduce an interrupt-controller abstraction before adding APIC/IOAPIC
  support.

### 5. Add execution units

- [ ] Define kernel task/thread structures, states, stacks, and lifetimes.
- [ ] Implement cooperative tasks first and test context ownership.
- [ ] Add a scheduler and timer-driven preemption only after context switching
  and cleanup are correct.
- [ ] Decide whether the first scheduler is single-core only and document the
  later SMP requirements.

### 6. Add a driver and I/O boundary

- [ ] Define narrow interfaces for console output, input events, timers,
  block devices, and platform shutdown.
- [ ] Keep UEFI, PS/2, QEMU, and raw I/O-port details behind those interfaces.
- [ ] Add device discovery and timeouts instead of assuming hardware exists.
- [ ] Add a physical/DMA-safe buffer abstraction before storage drivers.
- [ ] Add a block layer, then a read-only filesystem, before filesystem writes.

### 7. Add user/kernel separation (for a general-purpose kernel)

- [ ] Define a syscall ABI and argument-validation rules.
- [ ] Create user address spaces with separate page permissions.
- [ ] Implement a loader for a documented executable format.
- [ ] Add process creation, exit, waiting, signals or an equivalent event
  model, and per-process file/handle ownership.
- [ ] Prevent user code from accessing kernel memory or hardware directly.

These are not required for a monitor or single-purpose embedded kernel, but
they are required before calling AgnostOS a general-purpose OS kernel.

## Quality gates before each milestone

- Host-test pure parsing, allocation metadata, path, queue, and scheduler code.
- Add a deterministic QEMU smoke test for every hardware-facing feature.
- Run formatting, host tests, the UEFI release build, and Clippy in CI.
- Test both allocator feature configurations.
- Document every `unsafe` block with its ownership, lifetime, alignment, and
  interrupt/SMP assumptions.
- Ensure all recoverable hardware and allocation failures return errors rather
  than silently hanging.

## Recommended order

1. Memory-map ownership and allocator diagnostics.
2. Physical-frame allocator and page tables.
3. CPU fault handlers and a race-free timer/idle path.
4. Driver interfaces and DMA-safe buffers.
5. Cooperative tasks, then scheduling/preemption.
6. Read-only filesystem and executable loading.
7. User mode, syscalls, processes, and persistent storage writes.

## Project structure

The code is organized around ownership boundaries rather than one large kernel
module:

```text
src/main.rs       UEFI entry point and panic boundary
src/boot.rs       ordered startup and UEFI hand-off
src/allocator/    heap implementation and allocator selection
src/idt/          IDT, PIC, and interrupt handlers
src/platform/     CPU primitives and host-testable data structures
src/graphics/     framebuffer pixels, drawing, and text rasterization
src/console/      synchronized terminal state and output
src/keyboard/     PS/2 keyboard decoding and event queue
src/mouse/        optional PS/2 mouse support
src/commands/     shell parser and command handlers
src/shell/        interactive input loop
src/timer/        tick/time helpers
src/uefi_graphics/ UEFI GOP discovery
```

When adding a subsystem, put hardware access in a focused module, expose a
small checked API, keep policy in the higher-level module, and add host tests
for all logic that does not require actual CPU or device state.
