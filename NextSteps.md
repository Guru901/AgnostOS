# AgnostOS: What to Build Next

The project already has the essentials of a small UEFI kernel: a framebuffer
console, keyboard and mouse input, interrupts, a PIT timer, a heap allocator,
and an interactive shell. The next work should strengthen the kernel's
foundations before adding many user-facing commands.

## 1. Make the timer observable and tested

Add an `uptime` shell command and a QEMU smoke test that verifies timer IRQs
advance during boot. Decide whether `uptime_ms` needs accurate conversion from
PIT ticks or whether a coarse monotonic tick counter is sufficient.

## 2. Preserve and model the UEFI memory map

Keep a validated copy of the memory map after exiting boot services. Use it to
describe which physical ranges are free, reserved, firmware-owned, or occupied
by the kernel and framebuffer.

## 3. Build a physical-frame allocator

Replace the current "largest conventional region is the heap" policy with a
page-frame allocator that can allocate and free 4 KiB physical frames. Keep
the heap allocator on top of it rather than treating all conventional memory as
one permanently owned block.

## 4. Introduce page tables and a virtual-memory layout

Create an explicit kernel address-space layout and a safe mapper for x86_64
page tables. Start with identity mappings needed by the kernel, then map the
heap and framebuffer deliberately. This becomes the base for isolation and
user programs later.

## 5. Improve fault reporting

Add handlers for page faults, general-protection faults, invalid opcodes, and
divide-by-zero errors. Print the fault address/error code and halt cleanly so
memory-management bugs are diagnosable in QEMU.

## 6. Add a real allocation and memory diagnostic path

Expand `meminfo` with heap usage and allocator statistics, then add a small
kernel self-test that allocates, frees, and checks alignment. Test both the
linked-list allocator and the custom allocator configuration.

## 7. Add cooperative tasks before preemptive scheduling

Define a minimal task structure and an executor that can run several
cooperative tasks. Use timer deadlines for sleeping tasks. Do not add context
switching or preemption until task lifetime, stacks, and cleanup are clear.

## 8. Add a read-only filesystem path

Start with the UEFI filesystem protocol while boot services are available, or
load a small init/archive file before exiting them. This enables configuration,
fonts, and eventually executable loading without committing to a disk-driver
stack too early.

## Suggested order

1. Timer smoke test and `uptime`
2. Memory-map model and frame allocator
3. Paging and fault handlers
4. Allocator diagnostics
5. Cooperative tasks
6. File loading

At each milestone, add a host-side unit test where possible and one QEMU smoke
test for the hardware-facing behavior.
