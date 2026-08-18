#[cfg(feature = "custom-allocator")]
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::Ordering;
use uefi::boot;
use uefi::boot::MemoryType;
use uefi::mem::memory_map::MemoryMap;

use crate::{BOOT_SERVICES_EXITED, HEAP_SIZE, HEAP_START};

#[cfg(all(feature = "uefi-bin", feature = "custom-allocator"))]
#[global_allocator]
pub static ALLOCATOR: AgnostOSAllocator = AgnostOSAllocator::new();

#[cfg(all(feature = "uefi-bin", not(feature = "custom-allocator")))]
#[global_allocator]
pub static ALLOCATOR: linked_list_allocator::LockedHeap =
    linked_list_allocator::LockedHeap::empty();

/// An exclusively owned conventional-memory range selected for the kernel heap.
///
/// Its address and length are deliberately kept private: only allocator setup
/// code can turn this validated region into a raw allocation range.
#[derive(Debug)]
pub struct HeapRegion {
    start: usize,
    size: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum HeapError {
    AlreadyInitialized,
    NoConventionalMemory,
    TooSmall,
    Misaligned,
}

/// Exits UEFI boot services and returns the largest conventional-memory region
/// for use as the kernel heap.
///
/// # Safety
/// This may only be called once, after all UEFI boot services have been used.
/// The returned region is available for exclusive use by the allocator.
///
pub fn initialize_heap() -> Result<HeapRegion, HeapError> {
    if HEAP_SIZE.load(Ordering::Relaxed) != 0 {
        return Err(HeapError::AlreadyInitialized);
    }

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

    if heap_size == 0 {
        return Err(HeapError::NoConventionalMemory);
    }

    // Store heap info globally so commands like `meminfo` can read them.
    HEAP_START.store(heap_start, Ordering::Relaxed);
    HEAP_SIZE.store(heap_size, Ordering::Relaxed);

    Ok(HeapRegion {
        start: heap_start,
        size: heap_size,
    })
}

#[cfg(feature = "linked-list-allocator")]
/// Initializes the linked-list allocator from an owned heap region.
pub fn initialize_linked_list_allocator(
    allocator: &linked_list_allocator::LockedHeap,
    region: HeapRegion,
) {
    // SAFETY: `region` is consumed here and was selected as exclusively owned
    // conventional memory by `initialize_heap`.
    unsafe {
        allocator.lock().init(region.start as *mut u8, region.size);
    }
}

/// Installs the selected global allocator over an owned heap region.
pub fn initialize_global(region: HeapRegion) -> Result<(), HeapError> {
    #[cfg(all(feature = "uefi-bin", feature = "custom-allocator"))]
    {
        ALLOCATOR.init(region)
    }

    #[cfg(all(feature = "uefi-bin", not(feature = "custom-allocator")))]
    {
        initialize_linked_list_allocator(&ALLOCATOR, region);
        Ok(())
    }

    #[cfg(not(feature = "uefi-bin"))]
    {
        let _ = region;
        Ok(())
    }
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
/// there's enough remainder, and returns the aligned pointer. On deallocation,
/// adjacent free chunks are coalesced to limit fragmentation.
///
/// On deallocation: inserts the freed region back at the head of the free list.
pub struct AgnostOSAllocator {
    head: spin::Mutex<*mut FreeChunk>,
}

// SAFETY: allocator metadata is protected by `head`; initialization consumes
// the one owned `HeapRegion` before interrupts are enabled. The kernel has no
// SMP startup path. Any future SMP support must re-audit these impls and the
// backing-memory ownership model.
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
    pub fn init(&self, region: HeapRegion) -> Result<(), HeapError> {
        let heap_start = region.start;
        let heap_size = region.size;
        if heap_size < core::mem::size_of::<FreeChunk>() {
            return Err(HeapError::TooSmall);
        }
        if !heap_start.is_multiple_of(core::mem::align_of::<FreeChunk>()) {
            return Err(HeapError::Misaligned);
        }

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
        Ok(())
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
                let Some(alignment_offset) = align.checked_sub(1) else {
                    return core::ptr::null_mut();
                };
                let Some(aligned_unmasked) = start.checked_add(alignment_offset) else {
                    prev = &raw mut (*chunk).next;
                    current = &raw mut (*chunk).next;
                    continue;
                };
                let aligned = aligned_unmasked & !alignment_offset;
                let Some(end) = aligned.checked_add(size) else {
                    prev = &raw mut (*chunk).next;
                    current = &raw mut (*chunk).next;
                    continue;
                };
                let Some(chunk_end) = start.checked_add((*chunk).size) else {
                    prev = &raw mut (*chunk).next;
                    current = &raw mut (*chunk).next;
                    continue;
                };

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
    /// Inserts the freed chunk in address order and coalesces adjacent ranges.
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

            // Insert in address order so adjacent free ranges can be merged.
            let mut link: *mut *mut FreeChunk = &raw mut *head;
            while !(*link).is_null() && (*link as usize) < (chunk as usize) {
                link = &raw mut (**link).next;
            }
            (*chunk).next = *link;
            *link = chunk;

            // Merge with the following range first.
            let next = (*chunk).next;
            if !next.is_null()
                && (chunk as usize)
                    .checked_add((*chunk).size)
                    .is_some_and(|end| end == next as usize)
                && let Some(merged_size) = (*chunk).size.checked_add((*next).size)
            {
                (*chunk).size = merged_size;
                (*chunk).next = (*next).next;
            }

            // Find and merge the preceding range, if it is contiguous.
            let mut previous: *mut FreeChunk = core::ptr::null_mut();
            let mut current = *head;
            while !current.is_null() && current != chunk {
                previous = current;
                current = (*current).next;
            }
            if !previous.is_null()
                && (previous as usize)
                    .checked_add((*previous).size)
                    .is_some_and(|end| end == chunk as usize)
                && let Some(merged_size) = (*previous).size.checked_add((*chunk).size)
            {
                (*previous).size = merged_size;
                (*previous).next = (*chunk).next;
            }
        }
    }
}
