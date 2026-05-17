use std::collections::BTreeSet;
use std::fmt;

use crate::armv7a::{Memory, Result, Trap};

const PAGE_SHIFT: usize = 12;
const PAGE_SIZE: usize = 1 << PAGE_SHIFT;
const PAGE_SIZE_U32: u32 = PAGE_SIZE as u32;
const PAGE_MASK: u32 = !(PAGE_SIZE_U32 - 1);
const PAGE_COUNT: usize = 1usize << (32 - PAGE_SHIFT);
const PAGE_BITMAP_WORDS: usize = PAGE_COUNT / 64;

#[derive(Debug)]
pub struct MappedMemory {
    mapped_pages: Vec<u64>,
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    page_table: Vec<Option<usize>>,
    pages: Vec<MappedPage>,
    mappings: Vec<MappedRange>,
    dirty_pages: BTreeSet<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedMemoryRegionSnapshot {
    pub base: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
struct MappedPage {
    base: u32,
    storage: PageStorage,
}

#[derive(Debug)]
enum PageStorage {
    #[cfg(target_os = "linux")]
    Identity,
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    Owned(Box<[u8; PAGE_SIZE]>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MappedRange {
    base: u32,
    size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappedMemoryError {
    EmptyRegion,
    RegionOverflow { base: u32, size: usize },
    Overlap { base: u32, size: usize },
    MapFailed { base: u32, message: String },
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
                    "memory region {base:#010x}+{size:#x} overlaps an existing mapped page"
                )
            }
            Self::MapFailed { base, message } => {
                write!(f, "memory page {base:#010x} mapping failed: {message}")
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
            mapped_pages: vec![0; PAGE_BITMAP_WORDS],
            #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
            page_table: vec![None; PAGE_COUNT],
            pages: Vec::new(),
            mappings: Vec::new(),
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
        let page_start = align_down(base);
        let page_end = align_up(end)?;
        for page in page_range(page_start, page_end) {
            if self.page_mapped(page) {
                return Err(MappedMemoryError::Overlap { base, size });
            }
        }

        let first_new_page = self.pages.len();
        for page in page_range(page_start, page_end) {
            let page_base = page_addr(page);
            match map_page_storage(page_base) {
                Ok(storage) => {
                    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
                    let index = self.pages.len();
                    self.pages.push(MappedPage {
                        base: page_base,
                        storage,
                    });
                    self.set_page_mapped(page, true);
                    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
                    {
                        self.page_table[page] = Some(index);
                    }
                }
                Err(err) => {
                    self.rollback_new_pages(first_new_page);
                    return Err(err);
                }
            }
        }
        self.mappings.push(MappedRange { base, size });
        self.mappings.sort_by_key(|range| range.base);
        self.mark_dirty_range(base, size);
        Ok(())
    }

    pub fn load_bytes(&mut self, addr: u32, bytes: &[u8]) -> Result<()> {
        self.write_bytes(addr, bytes, true)
    }

    pub fn region_count(&self) -> usize {
        self.mappings.len()
    }

    pub fn snapshot_regions(&self) -> Vec<MappedMemoryRegionSnapshot> {
        self.mappings
            .iter()
            .map(|range| MappedMemoryRegionSnapshot {
                base: range.base,
                bytes: self.read_bytes_lossy(range.base, range.size),
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
        self.write_bytes(addr, bytes, false)
    }

    pub fn mapped_span_for_page(&self, page_base: u32, page_size: u32) -> Option<(u32, usize)> {
        let page_end = page_base.checked_add(page_size)?;
        for range in &self.mappings {
            let range_end = range.end();
            let start = page_base.max(range.base);
            let end = page_end.min(range_end);
            if start < end {
                return Some((start, (end - start) as usize));
            }
        }
        None
    }

    fn rollback_new_pages(&mut self, first_new_page: usize) {
        while self.pages.len() > first_new_page {
            if let Some(page) = self.pages.pop() {
                let index = page_index(page.base);
                self.set_page_mapped(index, false);
                #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
                {
                    self.page_table[index] = None;
                }
                unmap_page_storage(page);
            }
        }
    }

    #[inline(always)]
    fn page_mapped(&self, index: usize) -> bool {
        (self.mapped_pages[index / 64] & (1u64 << (index % 64))) != 0
    }

    fn set_page_mapped(&mut self, index: usize, mapped: bool) {
        let mask = 1u64 << (index % 64);
        if mapped {
            self.mapped_pages[index / 64] |= mask;
        } else {
            self.mapped_pages[index / 64] &= !mask;
        }
    }

    #[inline(always)]
    fn page_index_for_addr(&self, addr: u32) -> Result<usize> {
        let index = page_index(addr);
        if !self.page_mapped(index) {
            return Err(Trap::Memory(format!(
                "address {addr:#010x} is not mapped in guest memory"
            )));
        }
        #[cfg(all(target_os = "linux", not(debug_assertions)))]
        {
            Ok(index)
        }
        #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
        self.page_table[index].ok_or_else(|| {
            Trap::Memory(format!(
                "address {addr:#010x} is not mapped in guest memory"
            ))
        })
    }

    #[inline(always)]
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    fn page_and_offset(&self, addr: u32) -> Result<(&MappedPage, usize)> {
        let index = self.page_index_for_addr(addr)?;
        Ok((&self.pages[index], page_offset(addr)))
    }

    #[inline(always)]
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    fn page_and_offset_mut(&mut self, addr: u32) -> Result<(&mut MappedPage, usize)> {
        let index = self.page_index_for_addr(addr)?;
        Ok((&mut self.pages[index], page_offset(addr)))
    }

    #[inline(always)]
    fn read8_checked(&self, addr: u32) -> Result<u8> {
        #[cfg(all(target_os = "linux", not(debug_assertions)))]
        {
            self.page_index_for_addr(addr)?;
            return Ok(unsafe { std::ptr::read(addr as usize as *const u8) });
        }
        #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
        {
            let (page, off) = self.page_and_offset(addr)?;
            Ok(page.read8(off))
        }
    }

    #[inline(always)]
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    fn read16_checked(&self, addr: u32) -> Result<u16> {
        let b0 = self.read8_checked(addr)?;
        let b1 = self.read8_checked(addr.wrapping_add(1))?;
        Ok(u16::from_le_bytes([b0, b1]))
    }

    #[inline(always)]
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    fn read32_checked(&self, addr: u32) -> Result<u32> {
        let b0 = self.read8_checked(addr)?;
        let b1 = self.read8_checked(addr.wrapping_add(1))?;
        let b2 = self.read8_checked(addr.wrapping_add(2))?;
        let b3 = self.read8_checked(addr.wrapping_add(3))?;
        Ok(u32::from_le_bytes([b0, b1, b2, b3]))
    }

    #[inline(always)]
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    fn write8_checked(&mut self, addr: u32, value: u8) -> Result<()> {
        let (page, off) = self.page_and_offset_mut(addr)?;
        page.write8(off, value);
        Ok(())
    }

    #[inline(always)]
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    fn write16_checked(&mut self, addr: u32, value: u16) -> Result<()> {
        for (idx, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.write8_checked(addr.wrapping_add(idx as u32), byte)?;
        }
        Ok(())
    }

    #[inline(always)]
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    fn write32_checked(&mut self, addr: u32, value: u32) -> Result<()> {
        for (idx, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.write8_checked(addr.wrapping_add(idx as u32), byte)?;
        }
        Ok(())
    }

    fn write_bytes(&mut self, addr: u32, bytes: &[u8], dirty: bool) -> Result<()> {
        #[cfg(all(target_os = "linux", not(debug_assertions)))]
        {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    addr as usize as *mut u8,
                    bytes.len(),
                );
            }
            if dirty {
                self.mark_dirty_range(addr, bytes.len());
            }
            return Ok(());
        }
        #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
        {
            let mut copied = 0usize;
            while copied < bytes.len() {
                let current = addr.wrapping_add(copied as u32);
                let (page, off) = self.page_and_offset_mut(current)?;
                let n = (PAGE_SIZE - off).min(bytes.len() - copied);
                page.write_slice(off, &bytes[copied..copied + n]);
                copied += n;
            }
            if dirty {
                self.mark_dirty_range(addr, bytes.len());
            }
            Ok(())
        }
    }

    fn read_bytes_lossy(&self, addr: u32, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        for idx in 0..len {
            match self.read8_checked(addr.wrapping_add(idx as u32)) {
                Ok(byte) => out.push(byte),
                Err(_) => break,
            }
        }
        out
    }

    fn mark_dirty_range(&mut self, base: u32, size: usize) {
        if size == 0 {
            return;
        }
        let start = base & PAGE_MASK;
        let end = u64::from(base).saturating_add(size as u64);
        let mut page = start;
        while u64::from(page) < end {
            self.dirty_pages.insert(page);
            match page.checked_add(PAGE_SIZE_U32) {
                Some(next) => page = next,
                None => break,
            }
        }
    }

    fn snapshots_for_pages(&self, pages: BTreeSet<u32>) -> Vec<MappedMemoryRegionSnapshot> {
        let mut snapshots = Vec::new();
        for page in pages {
            if let Some((base, len)) = self.mapped_span_for_page(page, PAGE_SIZE_U32) {
                snapshots.push(MappedMemoryRegionSnapshot {
                    base,
                    bytes: self.read_bytes_lossy(base, len),
                });
            }
        }
        snapshots
    }
}

impl Drop for MappedMemory {
    fn drop(&mut self) {
        while let Some(page) = self.pages.pop() {
            unmap_page_storage(page);
        }
    }
}

impl Memory for MappedMemory {
    #[inline(always)]
    fn load8(&mut self, addr: u32) -> Result<u8> {
        #[cfg(all(target_os = "linux", not(debug_assertions)))]
        {
            return Ok(unsafe { std::ptr::read(addr as usize as *const u8) });
        }
        #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
        {
            self.read8_checked(addr)
        }
    }

    #[inline(always)]
    fn load16(&mut self, addr: u32) -> Result<u16> {
        #[cfg(all(target_os = "linux", not(debug_assertions)))]
        {
            let value = unsafe { std::ptr::read_unaligned(addr as usize as *const u16) };
            return Ok(u16::from_le(value));
        }
        #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
        {
            self.read16_checked(addr)
        }
    }

    #[inline(always)]
    fn load32(&mut self, addr: u32) -> Result<u32> {
        #[cfg(all(target_os = "linux", not(debug_assertions)))]
        {
            let value = unsafe { std::ptr::read_unaligned(addr as usize as *const u32) };
            return Ok(u32::from_le(value));
        }
        #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
        {
            self.read32_checked(addr)
        }
    }

    #[inline(always)]
    fn store8(&mut self, addr: u32, value: u8) -> Result<()> {
        #[cfg(all(target_os = "linux", not(debug_assertions)))]
        unsafe {
            std::ptr::write(addr as usize as *mut u8, value);
        }
        #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
        self.write8_checked(addr, value)?;
        self.dirty_pages.insert(addr & PAGE_MASK);
        Ok(())
    }

    #[inline(always)]
    fn store16(&mut self, addr: u32, value: u16) -> Result<()> {
        #[cfg(all(target_os = "linux", not(debug_assertions)))]
        unsafe {
            std::ptr::write_unaligned(addr as usize as *mut u16, value.to_le());
        }
        #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
        self.write16_checked(addr, value)?;
        self.mark_dirty_range(addr, 2);
        Ok(())
    }

    #[inline(always)]
    fn store32(&mut self, addr: u32, value: u32) -> Result<()> {
        #[cfg(all(target_os = "linux", not(debug_assertions)))]
        unsafe {
            std::ptr::write_unaligned(addr as usize as *mut u32, value.to_le());
        }
        #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
        self.write32_checked(addr, value)?;
        self.mark_dirty_range(addr, 4);
        Ok(())
    }
}

impl MappedPage {
    #[inline(always)]
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    fn read8(&self, off: usize) -> u8 {
        match &self.storage {
            #[cfg(target_os = "linux")]
            PageStorage::Identity => unsafe {
                std::ptr::read((self.base + off as u32) as usize as *const u8)
            },
            PageStorage::Owned(bytes) => bytes[off],
        }
    }

    #[inline(always)]
    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    fn write8(&mut self, off: usize, value: u8) {
        match &mut self.storage {
            #[cfg(target_os = "linux")]
            PageStorage::Identity => unsafe {
                std::ptr::write((self.base + off as u32) as usize as *mut u8, value);
            },
            PageStorage::Owned(bytes) => bytes[off] = value,
        }
    }

    #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
    fn write_slice(&mut self, off: usize, bytes: &[u8]) {
        match &mut self.storage {
            #[cfg(target_os = "linux")]
            PageStorage::Identity => unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    (self.base + off as u32) as usize as *mut u8,
                    bytes.len(),
                );
            },
            PageStorage::Owned(page) => page[off..off + bytes.len()].copy_from_slice(bytes),
        }
    }
}

impl MappedRange {
    fn end(&self) -> u32 {
        self.base + self.size as u32
    }
}

fn region_end(base: u32, size: usize) -> std::result::Result<u32, MappedMemoryError> {
    let size_u32 =
        u32::try_from(size).map_err(|_| MappedMemoryError::RegionOverflow { base, size })?;
    base.checked_add(size_u32)
        .ok_or(MappedMemoryError::RegionOverflow { base, size })
}

fn align_down(addr: u32) -> u32 {
    addr & PAGE_MASK
}

fn align_up(addr: u32) -> std::result::Result<u32, MappedMemoryError> {
    if addr == 0 {
        return Ok(0);
    }
    addr.checked_add(PAGE_SIZE_U32 - 1)
        .map(|value| value & PAGE_MASK)
        .ok_or(MappedMemoryError::RegionOverflow {
            base: addr,
            size: PAGE_SIZE,
        })
}

fn page_index(addr: u32) -> usize {
    (addr as usize) >> PAGE_SHIFT
}

#[cfg(not(all(target_os = "linux", not(debug_assertions))))]
fn page_offset(addr: u32) -> usize {
    (addr as usize) & (PAGE_SIZE - 1)
}

fn page_addr(index: usize) -> u32 {
    (index as u32) << PAGE_SHIFT
}

fn page_range(start: u32, end: u32) -> std::ops::Range<usize> {
    page_index(start)..page_index(end)
}

#[cfg(target_os = "linux")]
fn map_page_storage(base: u32) -> std::result::Result<PageStorage, MappedMemoryError> {
    match linux_fixed_mapping::map_page(base) {
        Ok(()) => Ok(PageStorage::Identity),
        Err(message) => {
            #[cfg(debug_assertions)]
            {
                let _ = message;
                Ok(PageStorage::Owned(Box::new([0; PAGE_SIZE])))
            }
            #[cfg(not(debug_assertions))]
            {
                Err(MappedMemoryError::MapFailed { base, message })
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn map_page_storage(_base: u32) -> std::result::Result<PageStorage, MappedMemoryError> {
    Ok(PageStorage::Owned(Box::new([0; PAGE_SIZE])))
}

fn unmap_page_storage(page: MappedPage) {
    #[cfg(target_os = "linux")]
    if matches!(page.storage, PageStorage::Identity) {
        let _ = linux_fixed_mapping::unmap_page(page.base);
    }
}

#[cfg(target_os = "linux")]
mod linux_fixed_mapping {
    use std::ffi::c_void;
    use std::os::raw::{c_int, c_long};

    use super::PAGE_SIZE;

    const PROT_READ: c_int = 0x1;
    const PROT_WRITE: c_int = 0x2;
    const MAP_PRIVATE: c_int = 0x02;
    const MAP_ANONYMOUS: c_int = 0x20;
    const MAP_FIXED_NOREPLACE: c_int = 0x100000;

    unsafe extern "C" {
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: c_int,
            flags: c_int,
            fd: c_int,
            offset: c_long,
        ) -> *mut c_void;
        fn munmap(addr: *mut c_void, len: usize) -> c_int;
    }

    pub fn map_page(addr: u32) -> std::result::Result<(), String> {
        let requested = addr as usize as *mut c_void;
        let mapped = unsafe {
            mmap(
                requested,
                PAGE_SIZE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE,
                -1,
                0,
            )
        };
        if mapped == !0usize as *mut c_void {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if mapped != requested {
            let _ = unsafe { munmap(mapped, PAGE_SIZE) };
            return Err(format!("mmap returned {mapped:p}, expected {addr:#010x}"));
        }
        Ok(())
    }

    pub fn unmap_page(addr: u32) -> std::result::Result<(), String> {
        let rc = unsafe { munmap(addr as usize as *mut c_void, PAGE_SIZE) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::armv7a::Memory;

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
    fn maps_pages_not_subregions() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 2).unwrap();

        assert_eq!(
            memory.map_zeroed(0x1002, 2),
            Err(MappedMemoryError::Overlap {
                base: 0x1002,
                size: 2,
            })
        );
    }

    #[test]
    fn rejects_overlapping_pages() {
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
