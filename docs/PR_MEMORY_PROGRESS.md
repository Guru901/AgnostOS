# AgnostOS Memory and Boot-Foundation PR History

This document records the changes delivered across the eight PRs shown in the
project history: #25 through #32. The PRs form one dependency chain. Each later
branch contains the earlier work as its base, so the descriptions below focus
on what each PR added or corrected rather than repeating the entire repository
at every step.

The overall progression was:

```text
UEFI memory-map ownership
        ↓
physical frame allocation
        ↓
boot/input reliability fixes
        ↓
heap backed by owned physical frames
        ↓
frame-owner metadata
        ↓
virtual-address layout and page-table root
        ↓
checked mappings and active CR3 bootstrap
```

## PR #25 — Preserve and classify the UEFI memory map

Branch/commit: `codex/memory-map-ownership` / `2f55839`

Purpose: preserve the firmware memory map after `exit_boot_services()` and
make physical-memory ownership visible to the kernel instead of leaving the
heap as the only known allocation.

### Main implementation

- Added `src/memory.rs` as the kernel-owned memory-map subsystem.
- Copied UEFI descriptors into a fixed-size static array with capacity for 256
  ranges. This avoids allocating memory while the heap and memory ownership are
  still being initialized.
- Added `MemoryKind` classifications:

  - `Usable` for conventional memory.
  - `Kernel` for loader code/data.
  - `Firmware` for boot-services code/data.
  - `Acpi` for ACPI reclaimable and non-volatile memory.
  - `Device` for MMIO and MMIO port-space descriptors.
  - `Runtime` for UEFI runtime-service memory.
  - `Unusable` for UEFI unusable memory.
  - `Reserved` for all other protected ranges.

- Added `MemoryRange` accessors for start address, byte length, end address,
  original UEFI type, and kernel classification.
- Added `MemoryMapSnapshot`, which exposes a bounded read-only view of the
  copied descriptor table.
- Added checked descriptor parsing:

  - Rejects empty maps.
  - Rejects zero-length descriptors.
  - Detects page-count multiplication overflow.
  - Detects physical-address range overflow.
  - Rejects more than the fixed descriptor capacity.

- Added `MemorySummary` and `summary()` so the kernel can report totals for
  usable, kernel-owned, firmware, ACPI, device, runtime, reserved, and unusable
  memory.
- Added a `meminfo` command path that prints the heap and the classified memory
  totals.
- Added `Framebuffer::physical_range()` so GOP framebuffer bytes can be
  classified as device memory.
- Updated boot and allocator initialization to pass the framebuffer range into
  memory initialization.
- Exported the new memory module from `src/lib.rs`.
- Updated `NextSteps.md` and `docs/KERNEL_READINESS.md` to record memory-map
  ownership as an implemented foundation.

### Effect on the project

Before this PR, the kernel had no durable view of physical memory after UEFI
boot services ended. After it, the kernel could answer which physical ranges
were usable, firmware-owned, device memory, or reserved. This was the basis for
the frame allocator and later page-table ownership.

### Verification

- Added classification tests for representative UEFI memory types.
- Added range-end saturation coverage.
- Added host-testable memory summary and descriptor logic.

## PR #26 — Add the physical frame allocator

Branch/commit: `codex/physical-frame-allocator` / `e1c8a7b`

This branch also carried the startup hardening commits `341c648` and `dea4ea7`
described below because they were needed to keep the firmware/QEMU boot path
usable while the allocator work was being developed.

Purpose: provide a fixed-storage physical 4 KiB frame allocator that allocates
only from ranges classified as usable.

### Main implementation

- Added `src/frame.rs`.
- Added page-aligned `FrameAddress` values.
- Added an internal fixed array of free physical ranges rather than using heap
  allocation for allocator metadata.
- Built the free list from `MemoryKind::Usable` ranges only.
- Added one-frame allocation.
- Added frame release with validation for:

  - Unaligned addresses.
  - Double frees.
  - Frames overlapping an existing free range.
  - Free-range storage exhaustion.

- Added allocator statistics:

  - Total physical frames discovered.
  - Currently free frames.
  - Number of free physical ranges.

- Added global initialization from the kernel-owned memory snapshot.
- Added explicit errors for uninitialized state, out-of-memory, invalid
  addresses, duplicate initialization, and storage exhaustion.
- Added `meminfo` output for frame statistics.
- Added frame allocator tests for allocation, release, unusable-range skipping,
  alignment validation, and double-free rejection.
- Updated boot ordering so the allocator is initialized after the memory map is
  copied.

### Startup hardening carried by this PR

Commit `341c648` hardened PS/2 and interrupt startup:

- Kept device IRQs masked while the controller and IDT were being initialized.
- Drained stale PS/2 output bytes left by firmware or device setup.
- Ignored keyboard ACK/RESEND protocol bytes instead of treating them as
  keyboard scancodes.
- Added delayed runtime IRQ enabling so keyboard input is enabled only after
  the input path is ready.
- Added separate mouse enabling support.
- Loaded the kernel data selector into DS, ES, FS, GS, and SS after installing
  the kernel GDT, preventing interrupt returns through stale UEFI selectors.

Commit `dea4ea7` temporarily disabled the QEMU smoke commands in the test
script while the allocator branch was being stabilized. This was a temporary
development workaround, not the intended final test policy; PR #28 restored
the smoke checks.

### Effect on the project

The kernel gained its first physical allocation boundary. Future heaps, page
tables, kernel stacks, and DMA buffers can now request physical frames without
scanning raw UEFI descriptors or accidentally consuming firmware/device memory.

## PR #27 — Preserve single-character backspace behavior

Branch/commit: `fix/backspace-input` / `991bbca`

Purpose: fix shell editing corruption that appeared when the visual cursor and
the shell-owned input string became unsynchronized.

### Main implementation

- Clamped the console cursor to the current line length before backspace and
  insertion operations.
- Added `console::reset_input_cursor()` for starting a fresh prompt.
- Updated shell command completion to insert completion characters through the
  normal cursor-aware insertion path instead of appending raw text directly.
- Made character removal safe when given a stale or oversized cursor index.
- Preserved the single-character backspace case instead of allowing an invalid
  cursor position to clear or corrupt the whole line.
- Reset the visual cursor after command execution and Ctrl-C prompt resets.
- Added regression coverage for removal with a stale maximum cursor value.

### GDT safety improvement

This PR also carried the data-segment reload introduced during the preceding
startup hardening: after loading the kernel GDT, all data and stack segment
registers are switched to the kernel data selector before interrupts can return.

### Effect on the project

This was primarily an input correctness PR, but it was important for memory and
boot work because QEMU smoke tests use shell/input activity to prove that the
kernel survived initialization and that interrupt state remains valid.

## PR #28 — Restore QEMU smoke checks and fix the mouse PIC cascade

Branch/commit: `codex/qemu-smoke-fix` / `4007c2d`

Purpose: restore hardware smoke testing and fix why mouse IRQ12 could not reach
the CPU even when the slave PIC mask was open.

### Main implementation

- Re-enabled the QEMU input and fault smoke tests in `scripts/test.sh`.
- Kept the host tests in both allocator configurations:

  - Default/linked-list allocator.
  - Custom allocator without default features.

- Fixed PIC mouse startup by unmasking both:

  - Slave IRQ12 for the PS/2 mouse.
  - Master IRQ2, the cascade line through which all slave PIC interrupts pass.

### Effect on the project

The test script once again verifies real boot behavior instead of stopping at
host-side tests. Keyboard and mouse input are now exercised through QEMU’s
actual PS/2 devices, and the fault smoke verifies the exception path.

This caught issues that ordinary Rust tests cannot detect, such as PIC mask
ordering, stale controller bytes, IRQ acknowledgement, and guest boot hangs.

## PR #29 — Put the heap on top of the physical frame allocator

Branch/commit: `codex/heap-on-frame-allocator` / `032f2f4`

Purpose: stop selecting an arbitrary conventional-memory descriptor directly
for the heap. The heap must be a consumer of the same physical allocator that
will later serve page tables and stacks.

### Main implementation

- Added contiguous physical-range allocation to `src/frame.rs`.
- Added `PhysicalRange` with start and byte-length accessors.
- Added `allocate_contiguous(frames)`.
- Added `largest_free_frames()`.
- Selected the largest free usable range for the heap.
- Removed the old allocator scan that searched the UEFI map independently for
  the largest conventional descriptor.
- Reserved the selected heap range in the owned memory map as `Kernel` memory.
- Removed the separate frame allocator initialization from `boot.rs`; heap
  initialization now establishes the frame allocator before selecting heap
  backing memory.
- Added `HeapError::FrameAllocatorUnavailable` and
  `HeapError::MemoryReservationFailed` failure propagation through the heap
  initialization path.
- Changed the memory map to a mutex-protected snapshot so reservations can
  continue after initialization.
- Replaced broad “mark overlapping descriptor” behavior with exact range
  splitting:

  - The portion before a reservation remains in its old category.
  - The exact reserved portion gets the requested category.
  - The portion after the reservation remains in its old category.

- Added external-range support for a framebuffer that is not contained in a
  firmware descriptor.
- Added `RangeNotFound` and `NotInitialized` memory-map errors.
- Added a regression test proving a reservation splits a usable range without
  losing either neighbor.
- Updated documentation to mark heap-on-frame-allocator work complete.

### Effect on the project

The heap became a real physical-memory consumer instead of a special case. A
frame allocated to the heap is removed from the frame allocator and the same
range is marked kernel-owned in the memory map. This prevents page-table or
stack allocation from reusing heap frames.

## PR #30 — Track owned kernel frame reservations

Branch/commit: `codex/owned-kernel-frames` / `1c7b513`

Purpose: attach an ownership category to every frame/range returned from the
physical allocator.

### Main implementation

- Added `FrameOwner` categories:

  - `Heap`.
  - `PageTable`.
  - `KernelStack`.
  - `Kernel`.

- Added `OwnedFrame`, containing a frame address and owner category.
- Added `OwnedPhysicalRange`, containing a physical range and owner category.
- Added address, length, owner, and release accessors.
- Added `allocate_owned(owner)`.
- Added `allocate_contiguous_owned(frames, owner)`.
- Changed heap allocation to use `FrameOwner::Heap` explicitly.
- Added ownership metadata tests.

### Effect on the project

The allocator now expresses why a physical frame is allocated, not only where
it is located. This is the ownership contract needed for page-table lifetime,
kernel-stack cleanup, and later address-space teardown.

The owner is metadata; it does not yet implement a full reference-counted
mapping lifetime or automatic deallocation of page-table trees.

## PR #31 — Define the virtual-address layout and allocate a page-table root

Branch/commit: `codex/paging-foundation` / `54be9a5`

Purpose: establish the x86_64 virtual-address policy and allocate the first real
page-table frame.

### Virtual-address layout

Added `src/paging.rs` with fixed canonical regions:

| Region | Base address | Intended contents |
|---|---:|---|
| Kernel | `0xffff_8000_0000_0000` | Kernel image and kernel mappings |
| Heap | `0xffff_9000_0000_0000` | Higher-half heap alias |
| Framebuffer | `0xffff_a000_0000_0000` | GOP framebuffer alias |
| MMIO | `0xffff_b000_0000_0000` | Device and MMIO mappings |
| Stacks | `0xffff_c000_0000_0000` | Kernel and future task stacks |

Each region is one TiB wide. The layout leaves substantial growth room while
keeping categories separated.

### Checked virtual ranges

- Added `VirtualRange`.
- Required non-zero length.
- Required page-aligned start and length.
- Checked end-address overflow.
- Rejected non-canonical x86_64 addresses.
- Added typed errors for empty, misaligned, non-canonical, and overflowing
  ranges.
- Added `VirtualMemoryLayout` as the named layout policy.

### Page-table root

- Allocated one physical frame with `FrameOwner::PageTable`.
- Reserved that frame in the owned memory map.
- Checked that `x86_64::structures::paging::PageTable` is exactly 4096 bytes.
- Zero-initialized the frame with `PageTable::new()`.
- Retained it in a `PageTableRoot` handle so the frame cannot be accidentally
  treated as free physical memory.
- Added accessors for its physical address and ownership category.
- Integrated initialization into the boot sequence after heap setup.

### Effect on the project

The kernel acquired a real paging foundation, but this PR intentionally did
not load the new table into CR3. It was a reviewable intermediate milestone:
the address policy and ownership were established before mapping live runtime
regions.

## PR #32 — Complete the memory bootstrap and activate paging

Branch/commit: `codex/complete-memory` / `95ce34c`

Purpose: turn the page-table root into a usable bootstrap address space while
keeping the existing UEFI-loaded execution addresses valid.

### Precise physical reservations

- Queried the UEFI `LoadedImage` protocol before exiting boot services.
- Captured the loaded image base and size.
- Reserved the loaded kernel image as `MemoryKind::Kernel` before frame
  allocation.
- Reserved a 64-page window around the current runtime stack before frame
  allocator initialization.
- Continued exact framebuffer reservation from the memory-map initialization.
- Continued exact heap reservation after contiguous heap frame allocation.
- Reserved the root page-table frame and every intermediate page-table frame
  allocated by the mapper.
- Kept UEFI loader data/code and other protected categories out of the free
  frame list.

### Page-table ownership and allocation

- Added a retained `PageTableArena` with capacity for 512 page-table frames.
- Implemented `x86_64::FrameAllocator<Size4KiB>` for that arena.
- Every intermediate table is:

  1. Allocated from reserved page-aligned bootstrap storage inside the loaded
     kernel image. This is necessary before CR3 activation because UEFI is not
     required to identity-map arbitrary conventional physical frames.
  2. Marked with `FrameOwner::PageTable`.
  3. Reserved in the owned physical memory map.
  4. Initialized to an empty `PageTable`.
  5. Retained for the lifetime of the root table.

### Mapping primitives

- Added checked `map_page()` for one 4 KiB mapping.
- Added checked `unmap_page()` that returns the unmapped physical frame.
- Validated virtual alignment and canonicality through `VirtualRange`.
- Validated physical-frame alignment.
- Added mapping and unmapping error variants.
- Flushed the affected TLB entry after mapping/unmapping.
- Used `OffsetPageTable` while building the table under the firmware’s
  identity-style bootstrap mapping.
- Added a recursive level-4 entry at page-table index 510.
- Used `RecursivePageTable` after CR3 activation so future mapping operations
  can reach all page-table levels without depending on a permanent physical
  offset mapping.

### Bootstrap mappings

The activated table maps the exact live ranges required to continue execution:

- Loaded kernel image, identity-mapped and writable during bootstrap.
- Heap backing range, identity-mapped, writable, and non-executable.
- Framebuffer range, identity-mapped, writable, non-cacheable, and
  non-executable.
- A 64-page identity-mapped runtime stack window, writable and non-executable.

The implementation deliberately does not walk and map every firmware device,
ACPI, or runtime descriptor at boot. Those ranges remain physically reserved
and can be mapped later through the checked mapping API. This keeps CR3
activation bounded and avoids making startup depend on potentially huge UEFI
descriptor ranges.

### Boot integration

- Changed `allocator::initialize_heap()` to receive the loaded image range.
- Changed `paging::initialize()` to receive the framebuffer and loaded image
  ranges.
- Installed paging after the global heap allocator is ready and before the IDT,
  PIC, and input runtime path are enabled.
- Loaded the owned root frame into CR3.
- Flushed the TLB after activation.
- Preserved identity mappings so existing code, global data, console state,
  framebuffer pointers, and interrupt stacks continue to work.

### Documentation updates

- Marked exact image/page-table/stack/device ownership work complete where
  implemented.
- Marked checked mapping/unmapping as implemented.
- Marked deliberate bootstrap mapping as implemented.
- Explicitly left higher-half relocation, guard pages, demand-mapped MMIO,
  and dynamic out-of-memory recovery as future work.

### Important limitation

This PR activates a safe bootstrap address space, not the final relocated
higher-half kernel. The kernel still executes at its existing identity-mapped
addresses. The fixed higher-half layout and mapping API are ready, but moving
the kernel, heap, and framebuffer to those virtual bases is a separate step.

## Combined result after PR #32

The project moved from a UEFI shell prototype with an ad-hoc heap to a kernel
with these memory foundations:

- Firmware memory descriptors are copied into kernel-owned storage.
- Physical ranges are classified and reported.
- Usable frames are allocated from a fixed-storage allocator.
- Heap memory is removed from the free-frame pool and marked kernel-owned.
- Page-table and stack ownership is explicit.
- Kernel image, heap, framebuffer, and stack ranges are reserved precisely.
- A canonical x86_64 virtual layout exists.
- Real page-table frames are allocated and initialized.
- Checked 4 KiB mapping and unmapping APIs exist.
- CR3 is switched to the owned root with a recursive page-table entry.
- QEMU verifies that keyboard, mouse, and exception paths still work after the
  memory bootstrap.

## Real-hardware firmware compatibility correction

After the memory work was tested on a physical machine, page-table
initialization failed even though QEMU succeeded. The cause was an invalid
bootstrap assumption: QEMU identity-mapped arbitrary physical frames, while the
machine's UEFI firmware did not guarantee that mapping for a newly allocated
frame.

The correction keeps the bootstrap root and intermediate page tables in a
page-aligned static storage arena inside the already-loaded kernel image. The
image is already reachable under the firmware's current mappings and is
reserved as kernel-owned before the frame allocator starts. Once CR3 is active,
the checked mapping API can use the recursive page-table mapping; allocating
additional page-table frames directly from general physical memory remains a
later step after a permanent physical-memory access strategy is installed.

This preserves the physical ownership model while removing the firmware-
specific identity-map dependency that caused the real-machine failure.

## Deliberately not included

These PRs did not implement scheduling or task management:

- Kernel threads and task structures.
- Context switching.
- Scheduler policy.
- User processes.
- Syscalls.

They also did not complete the remaining memory follow-up work:

- Higher-half relocation of the running kernel.
- Demand-mapped MMIO with complete cache-policy validation.
- Guard pages around stacks and allocations.
- Automatic cleanup of empty page-table branches.
- A controlled dynamic out-of-memory recovery path.
- Full page-fault classification and recoverability policy.

## Verification across the PR chain

The final memory branch was validated with:

- Host unit tests using the default allocator configuration.
- Host unit tests using the custom allocator configuration.
- Graphics integration tests.
- UEFI-targeted `cargo check`/release compilation.
- Clippy with warnings denied.
- QEMU keyboard/mouse input smoke testing.
- QEMU invalid-opcode fault smoke testing.
- Formatting and `git diff --check`.

The QEMU tests are still valuable even after real boot-media support is added:
they provide deterministic regression coverage for early boot, PIC/PS/2
behavior, page-table activation, and exception handling.
