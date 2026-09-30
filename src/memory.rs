//! Kernel-owned physical memory map.
//!
//! UEFI owns the buffer returned by `exit_boot_services`. It is copied into
//! this fixed static table before the UEFI buffer is discarded and before the
//! global allocator is initialized.

use core::fmt;

use spin::{Mutex, Once};
use uefi::mem::memory_map::{MemoryMap, MemoryType};

pub const MAX_MEMORY_RANGES: usize = 256;
const PAGE_SIZE: u64 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind {
    Usable,
    Kernel,
    Firmware,
    Acpi,
    Device,
    Runtime,
    Unusable,
    Reserved,
}

impl MemoryKind {
    const fn from_uefi_type(ty: MemoryType) -> Self {
        match ty {
            MemoryType::CONVENTIONAL => Self::Usable,
            MemoryType::LOADER_CODE | MemoryType::LOADER_DATA => Self::Kernel,
            MemoryType::BOOT_SERVICES_CODE | MemoryType::BOOT_SERVICES_DATA => Self::Firmware,
            MemoryType::ACPI_RECLAIM | MemoryType::ACPI_NON_VOLATILE => Self::Acpi,
            MemoryType::MMIO | MemoryType::MMIO_PORT_SPACE => Self::Device,
            MemoryType::RUNTIME_SERVICES_CODE | MemoryType::RUNTIME_SERVICES_DATA => Self::Runtime,
            MemoryType::UNUSABLE => Self::Unusable,
            _ => Self::Reserved,
        }
    }
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Usable => "usable",
            Self::Kernel => "kernel",
            Self::Firmware => "firmware",
            Self::Acpi => "acpi",
            Self::Device => "device",
            Self::Runtime => "runtime",
            Self::Unusable => "unusable",
            Self::Reserved => "reserved",
        };
        formatter.write_str(name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryRange {
    start: u64,
    length: u64,
    uefi_type: MemoryType,
    kind: MemoryKind,
}

impl MemoryRange {
    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    #[must_use]
    pub const fn length(self) -> u64 {
        self.length
    }

    #[must_use]
    pub const fn end(self) -> u64 {
        self.start.saturating_add(self.length)
    }

    #[must_use]
    pub const fn uefi_type(self) -> MemoryType {
        self.uefi_type
    }

    #[must_use]
    pub const fn kind(self) -> MemoryKind {
        self.kind
    }

    #[must_use]
    const fn contains(self, address: u64) -> bool {
        address >= self.start && address < self.end()
    }

    #[cfg(test)]
    pub(crate) const fn for_test(start: u64, length: u64, uefi_type: MemoryType) -> Self {
        Self {
            start,
            length,
            uefi_type,
            kind: MemoryKind::from_uefi_type(uefi_type),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryMapSnapshot {
    ranges: [MemoryRange; MAX_MEMORY_RANGES],
    count: usize,
}

impl MemoryMapSnapshot {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn ranges(&self) -> &[MemoryRange] {
        &self.ranges[..self.count]
    }

    /// Returns the ownership classification for a physical address in the
    /// snapshot.
    #[must_use]
    pub fn kind_at(&self, address: u64) -> Option<MemoryKind> {
        self.ranges()
            .iter()
            .find(|range| range.contains(address))
            .map(|range| range.kind())
    }

    fn copy_from<M: MemoryMap>(map: &M) -> Result<Self, MemoryMapError> {
        let empty = MemoryRange {
            start: 0,
            length: 0,
            uefi_type: MemoryType::RESERVED,
            kind: MemoryKind::Reserved,
        };
        let mut snapshot = Self {
            ranges: [empty; MAX_MEMORY_RANGES],
            count: 0,
        };

        for descriptor in map.entries() {
            if snapshot.count == MAX_MEMORY_RANGES {
                return Err(MemoryMapError::TooManyRanges);
            }
            let Some(length) = descriptor.page_count.checked_mul(PAGE_SIZE) else {
                return Err(MemoryMapError::InvalidRange);
            };
            let Some(end) = descriptor.phys_start.checked_add(length) else {
                return Err(MemoryMapError::InvalidRange);
            };
            if length == 0 || end <= descriptor.phys_start {
                return Err(MemoryMapError::InvalidRange);
            }

            snapshot.ranges[snapshot.count] = MemoryRange {
                start: descriptor.phys_start,
                length,
                uefi_type: descriptor.ty,
                kind: MemoryKind::from_uefi_type(descriptor.ty),
            };
            snapshot.count += 1;
        }
        if snapshot.is_empty() {
            return Err(MemoryMapError::Empty);
        }
        Ok(snapshot)
    }

    fn reserve_range(
        &mut self,
        start: u64,
        length: u64,
        kind: MemoryKind,
    ) -> Result<(), MemoryMapError> {
        let Some(end) = start.checked_add(length) else {
            return Err(MemoryMapError::InvalidRange);
        };
        if length == 0 || end <= start {
            return Err(MemoryMapError::InvalidRange);
        }

        let empty = MemoryRange {
            start: 0,
            length: 0,
            uefi_type: MemoryType::RESERVED,
            kind: MemoryKind::Reserved,
        };
        let old_ranges = self.ranges;
        let old_count = self.count;
        let mut replacement = [empty; MAX_MEMORY_RANGES];
        let mut replacement_count = 0;
        let mut overlapped = false;

        for range in old_ranges.into_iter().take(old_count) {
            let overlap_start = start.max(range.start);
            let overlap_end = end.min(range.end());
            if overlap_start >= overlap_end {
                if replacement_count == MAX_MEMORY_RANGES {
                    return Err(MemoryMapError::TooManyRanges);
                }
                replacement[replacement_count] = range;
                replacement_count += 1;
                continue;
            }
            overlapped = true;

            let push = |replacement: &mut [MemoryRange; MAX_MEMORY_RANGES],
                        count: &mut usize,
                        value: MemoryRange|
             -> Result<(), MemoryMapError> {
                if value.length == 0 || *count == MAX_MEMORY_RANGES {
                    return if value.length == 0 {
                        Ok(())
                    } else {
                        Err(MemoryMapError::TooManyRanges)
                    };
                }
                replacement[*count] = value;
                *count += 1;
                Ok(())
            };

            push(
                &mut replacement,
                &mut replacement_count,
                MemoryRange {
                    start: range.start,
                    length: overlap_start - range.start,
                    uefi_type: range.uefi_type,
                    kind: range.kind,
                },
            )?;
            push(
                &mut replacement,
                &mut replacement_count,
                MemoryRange {
                    start: overlap_start,
                    length: overlap_end - overlap_start,
                    uefi_type: range.uefi_type,
                    kind,
                },
            )?;
            push(
                &mut replacement,
                &mut replacement_count,
                MemoryRange {
                    start: overlap_end,
                    length: range.end() - overlap_end,
                    uefi_type: range.uefi_type,
                    kind: range.kind,
                },
            )?;
        }

        if !overlapped {
            return Err(MemoryMapError::RangeNotFound);
        }
        self.ranges = replacement;
        self.count = replacement_count;
        Ok(())
    }

    fn add_external_range(
        &mut self,
        start: u64,
        length: u64,
        kind: MemoryKind,
    ) -> Result<(), MemoryMapError> {
        let Some(end) = start.checked_add(length) else {
            return Err(MemoryMapError::InvalidRange);
        };
        if length == 0 || end <= start || self.count == MAX_MEMORY_RANGES {
            return Err(MemoryMapError::InvalidRange);
        }
        let range = MemoryRange {
            start,
            length,
            uefi_type: MemoryType::MMIO,
            kind,
        };
        self.ranges[self.count] = range;
        self.count += 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryMapError {
    AlreadyInitialized,
    Empty,
    InvalidRange,
    RangeNotFound,
    TooManyRanges,
    NotInitialized,
}

static MEMORY_MAP: Once<Mutex<MemoryMapSnapshot>> = Once::new();

/// Copies the firmware map into storage owned by the kernel.
pub fn initialize<M: MemoryMap>(
    map: &M,
    framebuffer: Option<(usize, usize)>,
) -> Result<(), MemoryMapError> {
    if MEMORY_MAP.get().is_some() {
        return Err(MemoryMapError::AlreadyInitialized);
    }
    let mut snapshot = MemoryMapSnapshot::copy_from(map)?;
    if let Some((start, length)) = framebuffer {
        match snapshot.reserve_range(start as u64, length as u64, MemoryKind::Device) {
            Ok(()) => {}
            Err(MemoryMapError::RangeNotFound) => {
                snapshot.add_external_range(start as u64, length as u64, MemoryKind::Device)?
            }
            Err(error) => return Err(error),
        }
    }
    MEMORY_MAP.call_once(|| Mutex::new(snapshot));
    Ok(())
}

/// Reserves a physical range in the owned map without losing the free ranges
/// around it. This is used for heap backing and later for stacks/page tables.
pub fn reserve(start: usize, length: usize, kind: MemoryKind) -> Result<(), MemoryMapError> {
    let Some(map) = MEMORY_MAP.get() else {
        return Err(MemoryMapError::NotInitialized);
    };
    map.lock().reserve_range(start as u64, length as u64, kind)
}

#[must_use]
pub fn snapshot() -> Option<MemoryMapSnapshot> {
    MEMORY_MAP.get().map(|map| *map.lock())
}

/// Returns the ownership classification for a physical address.
#[must_use]
pub fn kind_at(address: u64) -> Option<MemoryKind> {
    snapshot()?.kind_at(address)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemorySummary {
    pub ranges: usize,
    pub usable_bytes: u64,
    pub kernel_bytes: u64,
    pub firmware_bytes: u64,
    pub acpi_bytes: u64,
    pub device_bytes: u64,
    pub runtime_bytes: u64,
    pub reserved_bytes: u64,
    pub unusable_bytes: u64,
}

impl MemoryMapSnapshot {
    #[must_use]
    pub fn summary(&self) -> MemorySummary {
        let mut summary = MemorySummary {
            ranges: self.count,
            ..MemorySummary::default()
        };
        for range in self.ranges() {
            let destination = match range.kind {
                MemoryKind::Usable => &mut summary.usable_bytes,
                MemoryKind::Kernel => &mut summary.kernel_bytes,
                MemoryKind::Firmware => &mut summary.firmware_bytes,
                MemoryKind::Acpi => &mut summary.acpi_bytes,
                MemoryKind::Device => &mut summary.device_bytes,
                MemoryKind::Runtime => &mut summary.runtime_bytes,
                MemoryKind::Unusable => &mut summary.unusable_bytes,
                MemoryKind::Reserved => &mut summary.reserved_bytes,
            };
            *destination = destination.saturating_add(range.length);
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_uefi_memory_types() {
        assert_eq!(
            MemoryKind::from_uefi_type(MemoryType::CONVENTIONAL),
            MemoryKind::Usable
        );
        assert_eq!(
            MemoryKind::from_uefi_type(MemoryType::LOADER_DATA),
            MemoryKind::Kernel
        );
        assert_eq!(
            MemoryKind::from_uefi_type(MemoryType::BOOT_SERVICES_DATA),
            MemoryKind::Firmware
        );
        assert_eq!(
            MemoryKind::from_uefi_type(MemoryType::ACPI_RECLAIM),
            MemoryKind::Acpi
        );
        assert_eq!(
            MemoryKind::from_uefi_type(MemoryType::MMIO),
            MemoryKind::Device
        );
    }

    #[test]
    fn range_end_saturates() {
        let range = MemoryRange {
            start: u64::MAX - 1,
            length: 4,
            uefi_type: MemoryType::RESERVED,
            kind: MemoryKind::Reserved,
        };
        assert_eq!(range.end(), u64::MAX);
    }

    #[test]
    fn reservation_splits_a_usable_range_without_losing_neighbors() {
        let empty = MemoryRange {
            start: 0,
            length: 0,
            uefi_type: MemoryType::RESERVED,
            kind: MemoryKind::Reserved,
        };
        let mut snapshot = MemoryMapSnapshot {
            ranges: [empty; MAX_MEMORY_RANGES],
            count: 1,
        };
        snapshot.ranges[0] = MemoryRange::for_test(0x1000, 0x9000, MemoryType::CONVENTIONAL);

        snapshot
            .reserve_range(0x3000, 0x2000, MemoryKind::Kernel)
            .unwrap();

        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot.ranges()[0].length(), 0x2000);
        assert_eq!(snapshot.ranges()[1].start(), 0x3000);
        assert_eq!(snapshot.ranges()[1].length(), 0x2000);
        assert_eq!(snapshot.ranges()[1].kind(), MemoryKind::Kernel);
        assert_eq!(snapshot.ranges()[2].start(), 0x5000);
        assert_eq!(snapshot.ranges()[2].length(), 0x5000);
    }

    #[test]
    fn failed_reservation_preserves_the_original_map() {
        let empty = MemoryRange {
            start: 0,
            length: 0,
            uefi_type: MemoryType::RESERVED,
            kind: MemoryKind::Reserved,
        };
        let original = MemoryRange::for_test(0x1000, 0x2000, MemoryType::CONVENTIONAL);
        let mut snapshot = MemoryMapSnapshot {
            ranges: [empty; MAX_MEMORY_RANGES],
            count: 1,
        };
        snapshot.ranges[0] = original;
        let before = snapshot;

        assert_eq!(
            snapshot.reserve_range(0x8000, 0x1000, MemoryKind::Kernel),
            Err(MemoryMapError::RangeNotFound)
        );
        assert_eq!(snapshot, before);
    }
}
