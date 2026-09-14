//! Kernel-owned physical memory map.
//!
//! UEFI owns the buffer returned by `exit_boot_services`. It is copied into
//! this fixed static table before the UEFI buffer is discarded and before the
//! global allocator is initialized.

use core::fmt;

use spin::Once;
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

    fn mark_range(&mut self, start: u64, length: u64, kind: MemoryKind) {
        let Some(end) = start.checked_add(length) else {
            return;
        };
        for range in &mut self.ranges[..self.count] {
            if start < range.end() && range.start < end {
                range.kind = kind;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryMapError {
    AlreadyInitialized,
    Empty,
    InvalidRange,
    TooManyRanges,
}

static MEMORY_MAP: Once<MemoryMapSnapshot> = Once::new();

/// Copies the firmware map into storage owned by the kernel.
pub fn initialize<M: MemoryMap>(
    map: &M,
    heap_start: usize,
    heap_size: usize,
    framebuffer: Option<(usize, usize)>,
) -> Result<(), MemoryMapError> {
    if MEMORY_MAP.get().is_some() {
        return Err(MemoryMapError::AlreadyInitialized);
    }
    let mut snapshot = MemoryMapSnapshot::copy_from(map)?;
    snapshot.mark_range(heap_start as u64, heap_size as u64, MemoryKind::Kernel);
    if let Some((start, length)) = framebuffer {
        snapshot.mark_range(start as u64, length as u64, MemoryKind::Device);
    }
    MEMORY_MAP.call_once(|| snapshot);
    Ok(())
}

#[must_use]
pub fn snapshot() -> Option<&'static MemoryMapSnapshot> {
    MEMORY_MAP.get()
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
}
