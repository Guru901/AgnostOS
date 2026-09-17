//! x86_64 virtual-address layout and owned bootstrap page tables.

use core::{
    fmt,
    mem::size_of,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};
use spin::{Mutex, Once};
use x86_64::{
    PhysAddr, VirtAddr,
    instructions::tlb,
    registers::control::{Cr3, Cr3Flags},
    structures::paging::{
        FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PageTableIndex,
        PhysFrame, RecursivePageTable, Size4KiB,
    },
};

use crate::{frame, memory};

pub const PAGE_SIZE: u64 = 4096;
pub const KERNEL_BASE: u64 = 0xffff_8000_0000_0000;
pub const HEAP_BASE: u64 = 0xffff_9000_0000_0000;
pub const FRAMEBUFFER_BASE: u64 = 0xffff_a000_0000_0000;
pub const MMIO_BASE: u64 = 0xffff_b000_0000_0000;
pub const STACK_BASE: u64 = 0xffff_c000_0000_0000;

const LOW_CANONICAL_MAX: u64 = 0x0000_7fff_ffff_ffff;
const HIGH_CANONICAL_MIN: u64 = 0xffff_8000_0000_0000;
const MAX_PAGE_TABLE_FRAMES: usize = 512;
const RECURSIVE_INDEX: u16 = 510;

#[repr(align(4096))]
struct PageTableStorage(PageTable);

// UEFI is required to keep the loaded image reachable while this bootstrap
// runs. Keeping the bootstrap tables inside that image avoids assuming that
// arbitrary conventional physical memory is identity-mapped on every machine.
static mut PAGE_TABLE_STORAGE: [PageTableStorage; MAX_PAGE_TABLE_FRAMES + 1] =
    [const { PageTableStorage(PageTable::new()) }; MAX_PAGE_TABLE_FRAMES + 1];

fn storage_address(index: usize) -> u64 {
    // SAFETY: callers keep `index` within PAGE_TABLE_STORAGE's fixed bounds;
    // addr_of_mut avoids creating a Rust reference to mutable static storage.
    unsafe { core::ptr::addr_of_mut!(PAGE_TABLE_STORAGE[index].0) as u64 }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtualRangeError {
    Empty,
    Misaligned,
    NonCanonical,
    Overflow,
}

impl fmt::Display for VirtualRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "virtual range is empty",
            Self::Misaligned => "virtual range is not page-aligned",
            Self::NonCanonical => "virtual range contains a non-canonical address",
            Self::Overflow => "virtual range overflows the address space",
        })
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
    NotInitialized,
    Frame(frame::FrameError),
    MemoryMap(memory::MemoryMapError),
    AddressTooLarge,
    InvalidPageTableSize,
    MappingFailed,
    UnmappingFailed,
    NotDeviceMemory,
    Range(VirtualRangeError),
}

#[derive(Debug)]
struct PageTableArena {
    frames: [Option<frame::OwnedFrame>; MAX_PAGE_TABLE_FRAMES],
    addresses: [u64; MAX_PAGE_TABLE_FRAMES],
    count: usize,
}

impl PageTableArena {
    fn new() -> Self {
        Self {
            frames: [const { None }; MAX_PAGE_TABLE_FRAMES],
            addresses: [0; MAX_PAGE_TABLE_FRAMES],
            count: 0,
        }
    }
    fn reserve_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        if self.count == MAX_PAGE_TABLE_FRAMES {
            return None;
        }
        let address = storage_address(self.count + 1);
        let owned = frame::OwnedFrame::from_reserved(address, frame::FrameOwner::PageTable)?;
        let address_usize = usize::try_from(address).ok()?;
        // SAFETY: this image-resident frame is aligned and reachable through
        // the current UEFI image mapping.
        unsafe { ptr::write(address_usize as *mut PageTable, PageTable::new()) };
        self.addresses[self.count] = address;
        self.frames[self.count] = Some(owned);
        self.count += 1;
        Some(PhysFrame::containing_address(PhysAddr::new(address)))
    }
}

// SAFETY: returned frames are removed from the allocator and retained in the arena.
unsafe impl FrameAllocator<Size4KiB> for PageTableArena {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.reserve_frame()
    }
}

#[derive(Debug)]
pub struct PageTableRoot {
    frame: frame::OwnedFrame,
    arena: PageTableArena,
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
    #[must_use]
    pub fn child_table_count(&self) -> usize {
        self.arena.count
    }
}

static PAGE_TABLE_ROOT: Once<Mutex<PageTableRoot>> = Once::new();
static PAGING_ACTIVE: AtomicBool = AtomicBool::new(false);
#[must_use]
pub fn root() -> Option<&'static Mutex<PageTableRoot>> {
    PAGE_TABLE_ROOT.get()
}

fn with_offset_mapper<R>(
    operation: impl FnOnce(&mut OffsetPageTable<'_>, &mut PageTableArena) -> R,
) -> Result<R, PagingError> {
    let Some(root) = PAGE_TABLE_ROOT.get() else {
        return Err(PagingError::NotInitialized);
    };
    let mut root = root.lock();
    let address =
        usize::try_from(root.frame.address()).map_err(|_| PagingError::AddressTooLarge)?;
    // SAFETY: the owned root frame is identity mapped and exclusively accessed under the mutex.
    let table = unsafe { &mut *(address as *mut PageTable) };
    // SAFETY: the bootstrap address space identity maps all page-table frames.
    let mut mapper = unsafe { OffsetPageTable::new(table, VirtAddr::zero()) };
    Ok(operation(&mut mapper, &mut root.arena))
}

fn recursive_table_address() -> u64 {
    let index = u64::from(RECURSIVE_INDEX);
    ((index << 39) | (index << 30) | (index << 21) | (index << 12)) | 0xffff_0000_0000_0000
}

fn with_recursive_mapper<R>(
    operation: impl FnOnce(&mut RecursivePageTable<'_>, &mut PageTableArena) -> R,
) -> Result<R, PagingError> {
    let Some(root) = PAGE_TABLE_ROOT.get() else {
        return Err(PagingError::NotInitialized);
    };
    let mut root = root.lock();
    // SAFETY: the recursive P4 entry is installed before CR3 activation.
    let table = unsafe { &mut *(recursive_table_address() as *mut PageTable) };
    // SAFETY: the active table contains the recursive entry at index 510.
    let mut mapper =
        unsafe { RecursivePageTable::new_unchecked(table, PageTableIndex::new(RECURSIVE_INDEX)) };
    Ok(operation(&mut mapper, &mut root.arena))
}

fn map_on_mapper<M: Mapper<Size4KiB>>(
    mapper: &mut M,
    arena: &mut PageTableArena,
    virtual_address: u64,
    physical_address: u64,
    flags: PageTableFlags,
) -> Result<(), PagingError> {
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virtual_address));
    let frame = PhysFrame::containing_address(PhysAddr::new(physical_address));
    // SAFETY: the physical frame is aligned and intermediate tables are owned.
    unsafe { mapper.map_to(page, frame, flags, arena) }
        .map(|flush| flush.flush())
        .map_err(|_| PagingError::MappingFailed)
}

/// Maps one checked, page-aligned 4 KiB page.
pub fn map_page(
    virtual_address: u64,
    physical_address: u64,
    flags: PageTableFlags,
) -> Result<(), PagingError> {
    let range = VirtualRange::new(virtual_address, PAGE_SIZE).map_err(PagingError::Range)?;
    if !physical_address.is_multiple_of(PAGE_SIZE) {
        return Err(PagingError::Range(VirtualRangeError::Misaligned));
    }
    if PAGING_ACTIVE.load(Ordering::Acquire) {
        with_recursive_mapper(|mapper, arena| {
            map_on_mapper(mapper, arena, range.start(), physical_address, flags)
        })??;
    } else {
        with_offset_mapper(|mapper, arena| {
            map_on_mapper(mapper, arena, range.start(), physical_address, flags)
        })??;
    }
    Ok(())
}

/// Maps one physical device page using the kernel's safe MMIO policy.
///
/// Device pages must be explicitly classified as device memory and are always
/// mapped writable, non-executable, and uncached. Callers that need a
/// different cache policy must add a platform-specific validation path first.
pub fn map_device_page(virtual_address: u64, physical_address: u64) -> Result<(), PagingError> {
    if !physical_address.is_multiple_of(PAGE_SIZE) {
        return Err(PagingError::Range(VirtualRangeError::Misaligned));
    }
    if memory::kind_at(physical_address) != Some(memory::MemoryKind::Device) {
        return Err(PagingError::NotDeviceMemory);
    }
    map_page(
        virtual_address,
        physical_address,
        PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_CACHE
            | PageTableFlags::NO_EXECUTE,
    )
}

/// Unmaps one checked page and returns its physical frame address.
pub fn unmap_page(virtual_address: u64) -> Result<u64, PagingError> {
    let range = VirtualRange::new(virtual_address, PAGE_SIZE).map_err(PagingError::Range)?;
    let physical = if PAGING_ACTIVE.load(Ordering::Acquire) {
        with_recursive_mapper(|mapper, _| unmap_on_mapper(mapper, range.start()))??
    } else {
        with_offset_mapper(|mapper, _| unmap_on_mapper(mapper, range.start()))??
    };
    Ok(physical)
}

fn unmap_on_mapper<M: Mapper<Size4KiB>>(
    mapper: &mut M,
    virtual_address: u64,
) -> Result<u64, PagingError> {
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virtual_address));
    mapper
        .unmap(page)
        .map(|(frame, flush)| {
            flush.flush();
            frame.start_address().as_u64()
        })
        .map_err(|_| PagingError::UnmappingFailed)
}

fn map_range(start: u64, length: u64, flags: PageTableFlags) -> Result<(), PagingError> {
    let end = start
        .checked_add(length)
        .ok_or(PagingError::AddressTooLarge)?;
    let first = start / PAGE_SIZE * PAGE_SIZE;
    // `end` is exclusive.  Do not round an already aligned end up to the
    // following page, or every exact-page mapping would accidentally include
    // one page beyond the requested range.
    let last = if end.is_multiple_of(PAGE_SIZE) {
        end
    } else {
        end.checked_add(PAGE_SIZE - 1)
            .ok_or(PagingError::AddressTooLarge)?
            / PAGE_SIZE
            * PAGE_SIZE
    };
    let mut address = first;
    while address < last {
        map_page(address, address, flags)?;
        address += PAGE_SIZE;
    }
    Ok(())
}

/// Builds the live identity mappings and activates the owned root in CR3.
pub fn initialize(
    framebuffer: (usize, usize),
    kernel_image: (usize, usize),
) -> Result<(), PagingError> {
    if PAGE_TABLE_ROOT.get().is_some() {
        return Err(PagingError::AlreadyInitialized);
    }
    if size_of::<PageTable>() != PAGE_SIZE as usize {
        return Err(PagingError::InvalidPageTableSize);
    }
    let address = storage_address(0);
    let owned = frame::OwnedFrame::from_reserved(address, frame::FrameOwner::PageTable)
        .ok_or(PagingError::AddressTooLarge)?;
    let address_usize = usize::try_from(address).map_err(|_| PagingError::AddressTooLarge)?;
    // SAFETY: the frame is aligned, part of the loaded image, and writable
    // through the current UEFI image mapping.
    unsafe { ptr::write(address_usize as *mut PageTable, PageTable::new()) };
    PAGE_TABLE_ROOT.call_once(|| {
        Mutex::new(PageTableRoot {
            frame: owned,
            arena: PageTableArena::new(),
        })
    });

    // Map only the ranges that are live during this bootstrap. The firmware
    // map can contain very large device/ACPI/runtime descriptors; those stay
    // physically reserved and are mapped on demand by `map_page` instead of
    // making CR3 activation depend on their size.
    let image_start = kernel_image.0 as u64 & !(PAGE_SIZE - 1);
    let image_end = (kernel_image.0 as u64)
        .checked_add(kernel_image.1 as u64)
        .ok_or(PagingError::AddressTooLarge)?;
    map_range(
        image_start,
        image_end.saturating_sub(image_start),
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
    )?;

    let heap_start = crate::HEAP_START.load(Ordering::Relaxed) as u64;
    let heap_size = crate::HEAP_SIZE.load(Ordering::Relaxed) as u64;
    if heap_size != 0 {
        map_range(
            heap_start,
            heap_size,
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
        )?;
    }

    let framebuffer_start = framebuffer.0 as u64 & !(PAGE_SIZE - 1);
    let framebuffer_end = (framebuffer.0 as u64)
        .checked_add(framebuffer.1 as u64)
        .ok_or(PagingError::AddressTooLarge)?;
    map_range(
        framebuffer_start,
        framebuffer_end.saturating_sub(framebuffer_start),
        PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_CACHE
            | PageTableFlags::NO_EXECUTE,
    )?;

    let stack_marker = 0u8;
    let stack_page = (&stack_marker as *const u8 as u64) & !(PAGE_SIZE - 1);
    // Leave one unmapped guard page on either side of the bootstrap stack
    // window. A stack overflow/underflow should become a diagnosed page fault,
    // not silently corrupt adjacent kernel state.
    map_range(
        stack_page.saturating_sub(31 * PAGE_SIZE),
        62 * PAGE_SIZE,
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
    )?;

    let root_table_address = usize::try_from(address).map_err(|_| PagingError::AddressTooLarge)?;
    // A recursive entry gives the active mapper a stable virtual route to
    // every level of this hierarchy without recursively allocating mappings
    // for the mapper's own child tables.
    // SAFETY: this is the exclusively owned level-4 frame.
    let root_table = unsafe { &mut *(root_table_address as *mut PageTable) };
    root_table[PageTableIndex::new(RECURSIVE_INDEX)].set_addr(
        PhysAddr::new(address),
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
    );
    let root_frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(address));
    // SAFETY: all live kernel addresses are identity mapped and the recursive
    // entry makes page-table metadata reachable after this write.
    unsafe { Cr3::write(root_frame, Cr3Flags::empty()) };
    PAGING_ACTIVE.store(true, Ordering::Release);
    tlb::flush_all();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_page_ranges_do_not_round_into_an_extra_page() {
        let end: u64 = 0x20_000;
        let rounded = if end.is_multiple_of(PAGE_SIZE) {
            end
        } else {
            (end + PAGE_SIZE - 1) / PAGE_SIZE * PAGE_SIZE
        };
        assert_eq!(rounded, end);
    }

    #[test]
    fn layout_regions_are_canonical_and_page_aligned() {
        let layout = VirtualMemoryLayout::new();
        for range in [
            layout.kernel,
            layout.heap,
            layout.framebuffer,
            layout.mmio,
            layout.stacks,
        ] {
            assert_eq!(range.start() % PAGE_SIZE, 0);
            assert_eq!(range.length() % PAGE_SIZE, 0);
            assert!(range.end_exclusive() > range.start());
        }
    }
    #[test]
    fn virtual_ranges_reject_bad_addresses() {
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
