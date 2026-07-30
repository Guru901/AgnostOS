use core::alloc::GlobalAlloc;
use core::alloc::Layout;
use core::sync::atomic::Ordering;
use uefi::boot;
use uefi::boot::MemoryType;
use uefi::mem::memory_map::MemoryMap;

use crate::BOOT_SERVICES_EXITED;
use crate::{HEAP_SIZE, HEAP_START};

/// A free memory chunk header stored at the start of each free region.
/// The chunk uses the free memory itself to store bookkeeping data —
/// no separate allocation is needed.
struct FreeChunk {
    /// Size of this free chunk in bytes (including the header itself).
    size: usize,
    /// Pointer to the next free chunk in the linked list, or null if this is the last.
    next: *mut FreeChunk,
}

/// A linked-list based heap allocator for the AgnostOs kernel.
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
pub struct AgnostosAllocator {
    head: spin::Mutex<*mut FreeChunk>,
}

// SAFETY: We are single-core — there is no actual concurrent access.
// These impls exist only to satisfy Rust's type system requirements for
// a static global allocator.
unsafe impl Send for AgnostosAllocator {}
unsafe impl Sync for AgnostosAllocator {}

impl Default for AgnostosAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl AgnostosAllocator {
    /// Creates a new, uninitialized allocator.
    ///
    /// Must call [`init`] before any allocations are made.
    pub const fn new() -> Self {
        Self {
            head: spin::Mutex::new(core::ptr::null_mut()),
        }
    }

    /// Exits UEFI boot services, finds the largest conventional memory region,
    /// and initializes the allocator's free list with it.
    ///
    /// # Panics
    /// Will effectively hang/crash if no conventional memory is found,
    /// since the allocator head remains null and any subsequent allocation
    /// returns null.
    ///
    /// # Safety
    /// Must be called exactly once, after all UEFI services are done being used
    /// (GOP framebuffer pointer must already be saved before calling this).
    /// After this call, all UEFI boot services are unavailable.
    pub fn init(&self) {
        if HEAP_SIZE.load(Ordering::Relaxed) > 0 {
            panic!("Allocator already initialised")
        }

        // Exit boot services — after this point, no UEFI boot service calls are valid.
        let memory_map = unsafe { boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) };
        BOOT_SERVICES_EXITED.store(true, Ordering::Release);

        // Find the largest contiguous conventional (free) memory region.
        let mut heap_start = 0usize;
        let mut heap_size = 0usize;
        for descriptor in memory_map.entries() {
            if descriptor.ty == MemoryType::CONVENTIONAL {
                let size = descriptor.page_count as usize * 4096;
                if size > heap_size {
                    heap_start = descriptor.phys_start as usize;
                    heap_size = size;
                }
            }
        }

        if heap_size == 0 {
            panic!("Failed to initialise heap");
        }

        if heap_size < core::mem::size_of::<FreeChunk>() {
            panic!("Heap is too small");
        }

        if heap_start % core::mem::size_of::<FreeChunk>() != 0 {
            panic!("Heap is misaligned");
        }

        // Store heap info globally so commands like `meminfo` can read them.
        HEAP_START.store(heap_start, Ordering::Relaxed);
        HEAP_SIZE.store(heap_size, Ordering::Relaxed);

        // Write the initial FreeChunk header at the start of the heap region,
        // covering the entire heap as one large free chunk.
        unsafe {
            let chunk = heap_start as *mut FreeChunk;
            (*chunk).size = heap_size;
            (*chunk).next = core::ptr::null_mut();
            *self.head.lock() = chunk;
        }
    }
}

unsafe impl GlobalAlloc for AgnostosAllocator {
    /// Allocates a memory region satisfying `layout`.
    ///
    /// Walks the free list for a chunk large enough to hold the aligned
    /// allocation. If found, splits the chunk if the remainder is large
    /// enough to hold another [`FreeChunk`] header; otherwise removes the
    /// chunk entirely. Returns null if no suitable chunk is found (OOM).
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            // Ensure size and alignment are at least as large as FreeChunk
            // itself, so dealloc can always write a valid header back.
            let size = layout.size().max(core::mem::size_of::<FreeChunk>());
            let align = layout.align().max(core::mem::align_of::<FreeChunk>());

            let mut head = self.head.lock();
            let mut current: *mut *mut FreeChunk = &mut *head;
            let mut prev: *mut *mut FreeChunk = &mut *head;

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
                prev = &mut (*chunk).next;
                current = &mut (*chunk).next;
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
        unsafe {
            let size = layout.size().max(core::mem::size_of::<FreeChunk>());
            let mut head = self.head.lock();

            // Write a FreeChunk header directly into the freed memory.
            let chunk = ptr as *mut FreeChunk;
            (*chunk).size = size;

            // Insert at head of free list.
            (*chunk).next = *head;
            *head = chunk;
        }
    }
}
