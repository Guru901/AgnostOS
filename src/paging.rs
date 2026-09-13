//! x86_64 virtual-address layout and page-table ownership.
//!
//! This module establishes the address policy used by later mapping code. The
//! boot path still runs under the firmware-provided mappings, so the root
//! table allocated here is initialized but deliberately not loaded into CR3.

use core::{fmt, mem::size_of, ptr};

use spin::Once;
use x86_64::structures::paging::PageTable;

use crate::{frame, memory};

pub const PAGE_SIZE: u64 = 4096;
pub const KERNEL_BASE: u64 = 0xffff_8000_0000_0000;
pub const HEAP_BASE: u64 = 0xffff_9000_0000_0000;
pub const FRAMEBUFFER_BASE: u64 = 0xffff_a000_0000_0000;
pub const MMIO_BASE: u64 = 0xffff_b000_0000_0000;
pub const STACK_BASE: u64 = 0xffff_c000_0000_0000;

const LOW_CANONICAL_MAX: u64 = 0x0000_7fff_ffff_ffff;
const HIGH_CANONICAL_MIN: u64 = 0xffff_8000_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtualRangeError {
    Empty,
    Misaligned,
    NonCanonical,
    Overflow,
}

impl fmt::Display for VirtualRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "virtual range is empty",
            Self::Misaligned => "virtual range is not page-aligned",
            Self::NonCanonical => "virtual range contains a non-canonical address",
            Self::Overflow => "virtual range overflows the address space",
        };
        formatter.write_str(message)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtualRange {
    start: u64,
    length: u64,
}

impl VirtualRange {
    pub const fn new(start: u64, length: u64) -> Result<Self, VirtualRangeError> {
        if length == 0 {
            return Err(VirtualRangeError::Empty);
        }
        if !start.is_multiple_of(PAGE_SIZE) || !length.is_multiple_of(PAGE_SIZE) {
            return Err(VirtualRangeError::Misaligned);
        }
        let Some(end) = start.checked_add(length - 1) else {
            return Err(VirtualRangeError::Overflow);
        };
        if !is_canonical(start) || !is_canonical(end) {
            return Err(VirtualRangeError::NonCanonical);
        }
        Ok(Self { start, length })
    }

    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    #[must_use]
    pub const fn length(self) -> u64 {
        self.length
    }

    #[must_use]
    pub const fn end_exclusive(self) -> u64 {
        self.start + self.length
    }
}

const fn is_canonical(address: u64) -> bool {
    address <= LOW_CANONICAL_MAX || address >= HIGH_CANONICAL_MIN
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtualMemoryLayout {
    pub kernel: VirtualRange,
    pub heap: VirtualRange,
    pub framebuffer: VirtualRange,
    pub mmio: VirtualRange,
    pub stacks: VirtualRange,
}

impl VirtualMemoryLayout {
    /// Fixed 1 TiB regions leave room for growth without moving other areas.
    pub const fn new() -> Self {
        Self {
            kernel: region(KERNEL_BASE),
            heap: region(HEAP_BASE),
            framebuffer: region(FRAMEBUFFER_BASE),
            mmio: region(MMIO_BASE),
            stacks: region(STACK_BASE),
        }
    }
}

impl Default for VirtualMemoryLayout {
    fn default() -> Self {
        Self::new()
    }
}

const fn region(start: u64) -> VirtualRange {
    match VirtualRange::new(start, 1 << 40) {
        Ok(range) => range,
        Err(_) => panic!("invalid fixed virtual memory layout"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PagingError {
    AlreadyInitialized,
    Frame(frame::FrameError),
    MemoryMap(memory::MemoryMapError),
    AddressTooLarge,
    InvalidPageTableSize,
}

#[derive(Debug)]
pub struct PageTableRoot {
    frame: frame::OwnedFrame,
}

impl PageTableRoot {
    #[must_use]
    pub const fn physical_address(&self) -> u64 {
        self.frame.address()
    }

    #[must_use]
    pub const fn owner(&self) -> frame::FrameOwner {
        self.frame.owner()
    }
}

static PAGE_TABLE_ROOT: Once<PageTableRoot> = Once::new();

#[must_use]
pub fn root() -> Option<&'static PageTableRoot> {
    PAGE_TABLE_ROOT.get()
}

/// Allocates and initializes one actual level-4 page-table frame.
///
/// The physical address is temporarily usable as a pointer because this is
/// still before the kernel installs its own CR3. The frame remains owned by
/// `PageTableRoot`, so later mapper code can safely add child-table ownership.
pub fn initialize() -> Result<(), PagingError> {
    if PAGE_TABLE_ROOT.get().is_some() {
        return Err(PagingError::AlreadyInitialized);
    }
    if size_of::<PageTable>() != PAGE_SIZE as usize {
        return Err(PagingError::InvalidPageTableSize);
    }

    let owned = frame::allocate_owned(frame::FrameOwner::PageTable).map_err(PagingError::Frame)?;
    let address = owned.address();
    let address_usize = usize::try_from(address).map_err(|_| PagingError::AddressTooLarge)?;
    if let Err(error) = memory::reserve(
        address_usize,
        PAGE_SIZE as usize,
        memory::MemoryKind::Kernel,
    ) {
        let _ = owned.release();
        return Err(PagingError::MemoryMap(error));
    }

    // SAFETY: the frame is 4 KiB-aligned, exclusively owned by `owned`, and
    // the current UEFI bootstrap mapping makes its physical address writable.
    // The table is not exposed through CR3 until a later paging milestone.
    unsafe {
        ptr::write(address_usize as *mut PageTable, PageTable::new());
    }

    PAGE_TABLE_ROOT.call_once(|| PageTableRoot { frame: owned });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_regions_are_canonical_and_page_aligned() {
        let layout = VirtualMemoryLayout::new();
        let ranges = [
            layout.kernel,
            layout.heap,
            layout.framebuffer,
            layout.mmio,
            layout.stacks,
        ];
        for range in ranges {
            assert_eq!(range.start() % PAGE_SIZE, 0);
            assert_eq!(range.length() % PAGE_SIZE, 0);
            assert!(range.end_exclusive() > range.start());
        }
    }

    #[test]
    fn virtual_ranges_reject_bad_alignment_and_noncanonical_addresses() {
        assert_eq!(
            VirtualRange::new(1, PAGE_SIZE),
            Err(VirtualRangeError::Misaligned)
        );
        assert_eq!(
            VirtualRange::new(0x0000_8000_0000_0000, PAGE_SIZE),
            Err(VirtualRangeError::NonCanonical)
        );
    }
}
