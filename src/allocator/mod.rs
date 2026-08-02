#[cfg(feature = "custom-allocator")]
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::Ordering;
use uefi::boot;
use uefi::boot::MemoryType;
use uefi::mem::memory_map::MemoryMap;

use crate::{BOOT_SERVICES_EXITED, HEAP_SIZE, HEAP_START};

/// Exits UEFI boot services and returns the largest conventional-memory range
/// for use as the kernel heap.
///
/// # Safety
/// This may only be called once, after all UEFI boot services have been used.
/// The returned range is available for exclusive use by the allocator.
///
/// # Panics
///
/// 1. Will panic if the heap has already been initialised.
/// 2. Will panic if the heap initialised with zero size.
pub fn initialize_heap() -> (*mut u8, usize) {
    assert!(
        HEAP_SIZE.load(Ordering::Relaxed) == 0,
        "Allocator already initialised"
    );

    // Exit boot services — after this point, no UEFI boot service calls are valid.
    // SAFETY: initialization is one-shot and the caller has finished using
    // UEFI services, satisfying `exit_boot_services`' transition requirements.
    let memory_map = unsafe { boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) };
    BOOT_SERVICES_EXITED.store(true, Ordering::Release);

    // Find the largest contiguous conventional (free) memory region.
    let mut heap_start = 0usize;
    let mut heap_size = 0usize;
    for descriptor in memory_map.entries() {
        if descriptor.ty == MemoryType::CONVENTIONAL {
            let Ok(page_count) = usize::try_from(descriptor.page_count) else {
                continue;
            };
            let Some(size) = page_count.checked_mul(4096) else {
                continue;
            };
            let Ok(start) = usize::try_from(descriptor.phys_start) else {
                continue;
            };
            if size > heap_size {
                heap_start = start;
                heap_size = size;
            }
        }
    }

    assert!(heap_size != 0, "Failed to initialise heap");

    // Store heap info globally so commands like `meminfo` can read them.
    HEAP_START.store(heap_start, Ordering::Relaxed);
    HEAP_SIZE.store(heap_size, Ordering::Relaxed);

    (heap_start as *mut u8, heap_size)
}

#[cfg(feature = "custom-allocator")]
/// A free memory chunk header stored at the start of each free region.
/// The chunk uses the free memory itself to store bookkeeping data —
/// no separate allocation is needed.
struct FreeChunk {
    /// Size of this free chunk in bytes (including the header itself).
    size: usize,
    /// Pointer to the next free chunk in the linked list, or null if this is the last.
    next: *mut FreeChunk,
}

#[cfg(feature = "custom-allocator")]
/// A linked-list based heap allocator for the `AgnostOS` kernel.
///
/// Free memory regions are tracked as a singly-linked list of [`FreeChunk`]s.
/// Each chunk stores its metadata (size + next pointer) directly inside the
/// free memory it represents — no separate bookkeeping allocation is needed.
///
/// On allocation: walks the free list for a chunk that fits, splits it if
/// there's enough remainder, and returns the aligned pointer.
///
/// On deallocation: inserts the freed region back at the head of the free list.
/// Note: no coalescing is performed — adjacent free chunks are not merged.
pub struct AgnostOSAllocator {
    head: spin::Mutex<*mut FreeChunk>,
}

// SAFETY: We are single-core — there is no actual concurrent access.
// These impls exist only to satisfy Rust's type system requirements for
// a static global allocator.
#[cfg(feature = "custom-allocator")]
unsafe impl Send for AgnostOSAllocator {}
#[cfg(feature = "custom-allocator")]
unsafe impl Sync for AgnostOSAllocator {}

#[cfg(feature = "custom-allocator")]
impl Default for AgnostOSAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "custom-allocator")]
impl AgnostOSAllocator {
    /// Creates a new, uninitialized allocator.
    ///
    /// Must call [`init`] before any allocations are made.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            head: spin::Mutex::new(core::ptr::null_mut()),
        }
    }

    /// Initializes the allocator's free list with the supplied heap range.
    ///
    /// # Panics
    /// Will effectively hang/crash if no conventional memory is found,
    /// since the allocator head remains null and any subsequent allocation
    /// returns null.
    ///
    /// # Safety
    /// The heap must be exclusively owned by this allocator.
    pub fn init(&self, heap_start: usize, heap_size: usize) {
        assert!(
            heap_size >= core::mem::size_of::<FreeChunk>(),
            "Heap is too small"
        );
        assert!(
            heap_start.is_multiple_of(core::mem::size_of::<FreeChunk>()),
            "Heap is misaligned"
        );

        // Write the initial FreeChunk header at the start of the heap region,
        // covering the entire heap as one large free chunk.
        // SAFETY: the selected conventional-memory range is nonempty, large enough
        // for `FreeChunk`, and aligned as checked above; it is now owned by the heap.
        unsafe {
            let chunk = heap_start as *mut FreeChunk;
            (*chunk).size = heap_size;
            (*chunk).next = core::ptr::null_mut();
            *self.head.lock() = chunk;
        }
    }
}

#[cfg(feature = "custom-allocator")]
unsafe impl GlobalAlloc for AgnostOSAllocator {
    /// Allocates a memory region satisfying `layout`.
    ///
    /// Walks the free list for a chunk large enough to hold the aligned
    /// allocation. If found, splits the chunk if the remainder is large
    /// enough to hold another [`FreeChunk`] header; otherwise removes the
    /// chunk entirely. Returns null if no suitable chunk is found (OOM).
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the free list is initialized from the owned heap region, and the
        // mutex gives this allocator exclusive access while links and headers change.
        unsafe {
            // Ensure size and alignment are at least as large as FreeChunk
            // itself, so dealloc can always write a valid header back.
            let size = layout.size().max(core::mem::size_of::<FreeChunk>());
            let align = layout.align().max(core::mem::align_of::<FreeChunk>());

            let mut head = self.head.lock();
            let mut current: *mut *mut FreeChunk = &raw mut *head;
            let mut prev: *mut *mut FreeChunk = &raw mut *head;

            while !(*current).is_null() {
                let chunk = *current;
                let start = chunk as usize;

                // Align the start of the allocation within this chunk.
                let aligned = (start + align - 1) & !(align - 1);
                let end = aligned + size;
                let chunk_end = start + (*chunk).size;

                if end <= chunk_end {
                    let remainder_start = end;
                    let remainder_size = chunk_end - end;

                    if remainder_size >= core::mem::size_of::<FreeChunk>() {
                        // Chunk is large enough to split — put remainder back.
                        let remainder = remainder_start as *mut FreeChunk;
                        (*remainder).size = remainder_size;
                        (*remainder).next = (*chunk).next;
                        *prev = remainder;
                    } else {
                        // Remainder too small for a FreeChunk header — give it
                        // all to the allocation and remove chunk from list.
                        *prev = (*chunk).next;
                    }

                    return aligned as *mut u8;
                }

                // Chunk didn't fit — advance both pointers.
                prev = &raw mut (*chunk).next;
                current = &raw mut (*chunk).next;
            }

            // No suitable chunk found — out of memory.
            core::ptr::null_mut()
        }
    }

    /// Returns a previously allocated region back to the free list.
    ///
    /// Inserts the freed chunk at the head of the free list.
    /// Note: adjacent free chunks are **not** coalesced — fragmentation
    /// will accumulate over time with many small allocations/deallocations.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `GlobalAlloc` requires `ptr` to be a live allocation from this
        // allocator with `layout`; the mutex gives exclusive access to the free list.
        unsafe {
            let size = layout.size().max(core::mem::size_of::<FreeChunk>());
            let mut head = self.head.lock();

            // Write a FreeChunk header directly into the freed memory.
            // `alloc` guarantees this pointer uses at least `FreeChunk` alignment;
            // `ptr` is erased to `u8` by the `GlobalAlloc` trait signature.
            #[allow(clippy::cast_ptr_alignment)]
            let chunk = ptr.cast::<FreeChunk>();
            debug_assert!(chunk.is_aligned());
            (*chunk).size = size;

            // Insert at head of free list.
            (*chunk).next = *head;
            *head = chunk;
        }
    }
}
