//! Physical 4 KiB frame allocation.
//!
//! Only ranges classified as `Usable` by the owned UEFI memory map enter this
//! fixed-storage free list. The heap, kernel, firmware, ACPI, and device
//! ranges are therefore reserved automatically.

use spin::{Mutex, Once};

use crate::memory::{self, MemoryKind, MemoryRange};

const PAGE_SIZE: u64 = 4096;
const MAX_FREE_RANGES: usize = memory::MAX_MEMORY_RANGES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameAddress(u64);

impl FrameAddress {
    #[must_use]
    pub const fn new(address: u64) -> Option<Self> {
        if address.is_multiple_of(PAGE_SIZE) {
            Some(Self(address))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn address(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FreeRange {
    start: u64,
    frames: u64,
}

impl FreeRange {
    #[must_use]
    const fn end(self) -> u64 {
        self.start
            .saturating_add(self.frames.saturating_mul(PAGE_SIZE))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameStats {
    pub total_frames: u64,
    pub free_frames: u64,
    pub free_ranges: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    AlreadyInitialized,
    MemoryMapUnavailable,
    OutOfMemory,
    InvalidAddress,
    FreeRangeStorageFull,
}

struct FrameAllocator {
    ranges: [FreeRange; MAX_FREE_RANGES],
    range_count: usize,
    total_frames: u64,
    free_frames: u64,
}

impl FrameAllocator {
    fn new(ranges: &[MemoryRange]) -> Self {
        let empty = FreeRange {
            start: 0,
            frames: 0,
        };
        let mut allocator = Self {
            ranges: [empty; MAX_FREE_RANGES],
            range_count: 0,
            total_frames: 0,
            free_frames: 0,
        };
        for range in ranges {
            if range.kind() != MemoryKind::Usable || allocator.range_count == MAX_FREE_RANGES {
                continue;
            }
            let frames = range.length() / PAGE_SIZE;
            if frames == 0 {
                continue;
            }
            allocator.ranges[allocator.range_count] = FreeRange {
                start: range.start(),
                frames,
            };
            allocator.range_count += 1;
            allocator.total_frames = allocator.total_frames.saturating_add(frames);
            allocator.free_frames = allocator.free_frames.saturating_add(frames);
        }
        allocator
    }

    fn allocate(&mut self) -> Result<FrameAddress, FrameError> {
        if self.range_count == 0 {
            return Err(FrameError::OutOfMemory);
        }
        let range = &mut self.ranges[0];
        let address = range.start;
        range.start = range.start.saturating_add(PAGE_SIZE);
        range.frames -= 1;
        if range.frames == 0 {
            self.remove_range(0);
        }
        self.free_frames -= 1;
        Ok(FrameAddress(address))
    }

    fn release(&mut self, frame: FrameAddress) -> Result<(), FrameError> {
        let address = frame.address();
        if !address.is_multiple_of(PAGE_SIZE) {
            return Err(FrameError::InvalidAddress);
        }
        let mut index = 0;
        while index < self.range_count && self.ranges[index].start < address {
            index += 1;
        }
        if index > 0 && self.ranges[index - 1].end() > address {
            return Err(FrameError::InvalidAddress);
        }
        if index < self.range_count && address.saturating_add(PAGE_SIZE) > self.ranges[index].start
        {
            return Err(FrameError::InvalidAddress);
        }

        if index > 0 && self.ranges[index - 1].end() == address {
            self.ranges[index - 1].frames += 1;
            if index < self.range_count && self.ranges[index - 1].end() == self.ranges[index].start
            {
                self.ranges[index - 1].frames += self.ranges[index].frames;
                self.remove_range(index);
            }
        } else if index < self.range_count
            && address.saturating_add(PAGE_SIZE) == self.ranges[index].start
        {
            self.ranges[index].start = address;
            self.ranges[index].frames += 1;
        } else {
            if self.range_count == MAX_FREE_RANGES {
                return Err(FrameError::FreeRangeStorageFull);
            }
            let mut move_index = self.range_count;
            while move_index > index {
                self.ranges[move_index] = self.ranges[move_index - 1];
                move_index -= 1;
            }
            self.ranges[index] = FreeRange {
                start: address,
                frames: 1,
            };
            self.range_count += 1;
        }
        self.free_frames = self.free_frames.saturating_add(1);
        Ok(())
    }

    fn remove_range(&mut self, index: usize) {
        for current in index..self.range_count.saturating_sub(1) {
            self.ranges[current] = self.ranges[current + 1];
        }
        self.range_count -= 1;
    }

    fn stats(&self) -> FrameStats {
        FrameStats {
            total_frames: self.total_frames,
            free_frames: self.free_frames,
            free_ranges: self.range_count,
        }
    }
}

static FRAME_ALLOCATOR: Once<Mutex<FrameAllocator>> = Once::new();

/// Initializes the frame allocator from the kernel-owned memory map.
pub fn initialize() -> Result<(), FrameError> {
    if FRAME_ALLOCATOR.get().is_some() {
        return Err(FrameError::AlreadyInitialized);
    }
    let Some(snapshot) = memory::snapshot() else {
        return Err(FrameError::MemoryMapUnavailable);
    };
    FRAME_ALLOCATOR.call_once(|| Mutex::new(FrameAllocator::new(snapshot.ranges())));
    Ok(())
}

/// Allocates one 4 KiB physical frame.
pub fn allocate() -> Result<FrameAddress, FrameError> {
    let Some(allocator) = FRAME_ALLOCATOR.get() else {
        return Err(FrameError::MemoryMapUnavailable);
    };
    allocator.lock().allocate()
}

/// Returns a frame to the allocator.
pub fn release(frame: FrameAddress) -> Result<(), FrameError> {
    let Some(allocator) = FRAME_ALLOCATOR.get() else {
        return Err(FrameError::MemoryMapUnavailable);
    };
    allocator.lock().release(frame)
}

#[must_use]
pub fn stats() -> Option<FrameStats> {
    FRAME_ALLOCATOR
        .get()
        .map(|allocator| allocator.lock().stats())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uefi::mem::memory_map::MemoryType;

    fn usable(start: u64, frames: u64) -> MemoryRange {
        MemoryRange::for_test(start, frames * PAGE_SIZE, MemoryType::CONVENTIONAL)
    }

    #[test]
    fn allocates_and_releases_frames() {
        let mut allocator = FrameAllocator::new(&[usable(0x1000, 3)]);
        assert_eq!(allocator.allocate().unwrap().address(), 0x1000);
        assert_eq!(allocator.allocate().unwrap().address(), 0x2000);
        assert_eq!(allocator.stats().free_frames, 1);
        allocator.release(FrameAddress(0x1000)).unwrap();
        assert_eq!(allocator.stats().free_frames, 2);
        assert_eq!(allocator.allocate().unwrap().address(), 0x1000);
    }

    #[test]
    fn rejects_double_free_and_unaligned_addresses() {
        let mut allocator = FrameAllocator::new(&[usable(0x1000, 1)]);
        let frame = allocator.allocate().unwrap();
        allocator.release(frame).unwrap();
        assert_eq!(allocator.release(frame), Err(FrameError::InvalidAddress));
        assert_eq!(
            allocator.release(FrameAddress(0x1234)),
            Err(FrameError::InvalidAddress)
        );
    }

    #[test]
    fn skips_non_usable_ranges() {
        let reserved = MemoryRange::for_test(0, PAGE_SIZE, MemoryType::RESERVED);
        let mut allocator = FrameAllocator::new(&[reserved, usable(0x2000, 1)]);
        assert_eq!(allocator.stats().total_frames, 1);
        assert_eq!(allocator.allocate().unwrap().address(), 0x2000);
    }
}
