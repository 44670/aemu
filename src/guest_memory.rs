use std::collections::BTreeSet;
use std::fmt;

use crate::armv6::{Memory, Result, Trap};

#[derive(Debug, Clone)]
pub struct MappedMemory {
    regions: Vec<MemoryRegion>,
    dirty_pages: BTreeSet<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedMemoryRegionSnapshot {
    pub base: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct MemoryRegion {
    base: u32,
    data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappedMemoryError {
    EmptyRegion,
    RegionOverflow { base: u32, size: usize },
    Overlap { base: u32, size: usize },
}

impl fmt::Display for MappedMemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRegion => write!(f, "memory region size must be nonzero"),
            Self::RegionOverflow { base, size } => {
                write!(f, "memory region {base:#010x}+{size:#x} overflows")
            }
            Self::Overlap { base, size } => {
                write!(
                    f,
                    "memory region {base:#010x}+{size:#x} overlaps an existing region"
                )
            }
        }
    }
}

impl std::error::Error for MappedMemoryError {}

impl Default for MappedMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl MappedMemory {
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
            dirty_pages: BTreeSet::new(),
        }
    }

    pub fn map_zeroed(
        &mut self,
        base: u32,
        size: usize,
    ) -> std::result::Result<(), MappedMemoryError> {
        if size == 0 {
            return Err(MappedMemoryError::EmptyRegion);
        }
        let end = region_end(base, size)?;
        if self
            .regions
            .iter()
            .any(|region| ranges_overlap(base, end, region.base, region.end()))
        {
            return Err(MappedMemoryError::Overlap { base, size });
        }
        self.regions.push(MemoryRegion {
            base,
            data: vec![0; size],
        });
        self.regions.sort_by_key(|region| region.base);
        self.mark_dirty_range(base, size);
        Ok(())
    }

    pub fn load_bytes(&mut self, addr: u32, bytes: &[u8]) -> Result<()> {
        for (idx, byte) in bytes.iter().copied().enumerate() {
            self.store8(addr.wrapping_add(idx as u32), byte)?;
        }
        Ok(())
    }

    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    pub fn snapshot_regions(&self) -> Vec<MappedMemoryRegionSnapshot> {
        self.regions
            .iter()
            .map(|region| MappedMemoryRegionSnapshot {
                base: region.base,
                bytes: region.data.clone(),
            })
            .collect()
    }

    pub fn clear_dirty_pages(&mut self) {
        self.dirty_pages.clear();
    }

    pub fn take_dirty_region_snapshots(&mut self) -> Vec<MappedMemoryRegionSnapshot> {
        let pages = std::mem::take(&mut self.dirty_pages);
        self.snapshots_for_pages(pages)
    }

    pub fn write_clean_bytes(&mut self, addr: u32, bytes: &[u8]) -> Result<()> {
        for (idx, byte) in bytes.iter().copied().enumerate() {
            let (region, off) = self.offset(addr.wrapping_add(idx as u32))?;
            self.regions[region].data[off] = byte;
        }
        Ok(())
    }

    pub fn mapped_span_for_page(&self, page_base: u32, page_size: u32) -> Option<(u32, usize)> {
        let page_end = page_base.checked_add(page_size)?;
        for region in &self.regions {
            let start = page_base.max(region.base);
            let end = page_end.min(region.end());
            if start < end {
                return Some((start, (end - start) as usize));
            }
        }
        None
    }

    fn offset(&self, addr: u32) -> Result<(usize, usize)> {
        for (idx, region) in self.regions.iter().enumerate() {
            if addr >= region.base && addr < region.end() {
                return Ok((idx, (addr - region.base) as usize));
            }
        }
        Err(Trap::Memory(format!(
            "address {addr:#010x} is not mapped in guest memory"
        )))
    }

    fn mark_dirty_range(&mut self, base: u32, size: usize) {
        if size == 0 {
            return;
        }
        let start = base & !0xfff;
        let end = u64::from(base).saturating_add(size as u64);
        let mut page = start;
        while u64::from(page) < end {
            self.dirty_pages.insert(page);
            match page.checked_add(0x1000) {
                Some(next) => page = next,
                None => break,
            }
        }
    }

    fn snapshots_for_pages(&self, pages: BTreeSet<u32>) -> Vec<MappedMemoryRegionSnapshot> {
        let mut snapshots = Vec::new();
        for page in pages {
            if let Some((base, len)) = self.mapped_span_for_page(page, 0x1000) {
                if let Ok((region_idx, off)) = self.offset(base) {
                    snapshots.push(MappedMemoryRegionSnapshot {
                        base,
                        bytes: self.regions[region_idx].data[off..off + len].to_vec(),
                    });
                }
            }
        }
        snapshots
    }
}

impl Memory for MappedMemory {
    fn load8(&mut self, addr: u32) -> Result<u8> {
        let (region, off) = self.offset(addr)?;
        Ok(self.regions[region].data[off])
    }

    fn store8(&mut self, addr: u32, value: u8) -> Result<()> {
        let (region, off) = self.offset(addr)?;
        self.regions[region].data[off] = value;
        self.dirty_pages.insert(addr & !0xfff);
        Ok(())
    }
}

impl MemoryRegion {
    fn end(&self) -> u32 {
        self.base + self.data.len() as u32
    }
}

fn region_end(base: u32, size: usize) -> std::result::Result<u32, MappedMemoryError> {
    let size_u32 =
        u32::try_from(size).map_err(|_| MappedMemoryError::RegionOverflow { base, size })?;
    base.checked_add(size_u32)
        .ok_or(MappedMemoryError::RegionOverflow { base, size })
}

fn ranges_overlap(a_start: u32, a_end: u32, b_start: u32, b_end: u32) -> bool {
    a_start < b_end && b_start < a_end
}

#[cfg(test)]
mod tests {
    use crate::armv6::Memory;

    use super::*;

    #[test]
    fn maps_multiple_regions_and_accesses_bytes() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x10).unwrap();
        memory.map_zeroed(0x3000, 0x10).unwrap();

        memory.store32(0x100c, 0x1122_3344).unwrap();
        memory.store16(0x3002, 0x5566).unwrap();

        assert_eq!(memory.region_count(), 2);
        assert_eq!(memory.load32(0x100c).unwrap(), 0x1122_3344);
        assert_eq!(memory.load16(0x3002).unwrap(), 0x5566);

        let snapshots = memory.snapshot_regions();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].base, 0x1000);
        assert_eq!(
            &snapshots[0].bytes[0x0c..0x10],
            &0x1122_3344u32.to_le_bytes()
        );
        assert_eq!(snapshots[1].base, 0x3000);
        assert_eq!(&snapshots[1].bytes[0x02..0x04], &0x5566u16.to_le_bytes());
    }

    #[test]
    fn rejects_overlapping_regions() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x100).unwrap();

        assert_eq!(
            memory.map_zeroed(0x1080, 0x100),
            Err(MappedMemoryError::Overlap {
                base: 0x1080,
                size: 0x100,
            })
        );
    }

    #[test]
    fn reports_unmapped_access() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x10).unwrap();

        let err = memory.load8(0x2000).unwrap_err();
        assert!(matches!(err, Trap::Memory(_)));
    }
}
