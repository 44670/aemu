use std::fmt;
use std::path::PathBuf;

use crate::armv7a::{Memory, VecMemory};

const PT_LOAD: u32 = 1;
const PT_ARM_EXIDX: u32 = 0x7000_0001;
const PAGE_SIZE: u32 = 0x1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfLoadPlan {
    pub path: PathBuf,
    pub load_bias: u32,
    pub entry: u32,
    pub memory_base: u32,
    pub memory_size: u32,
    pub arm_exidx: Option<ElfLoadRange>,
    pub segments: Vec<ElfLoadSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElfLoadRange {
    pub addr: u32,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfLoadSegment {
    pub file_offset: u32,
    pub file_size: u32,
    pub memory_addr: u32,
    pub memory_size: u32,
    pub flags: u32,
    pub align: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfLoadError {
    NotElf,
    UnsupportedClass(u8),
    UnsupportedEndian(u8),
    UnsupportedMachine(u16),
    Truncated(&'static str),
    BadProgramHeader,
    BadLoadSegment(&'static str),
    NoLoadSegments,
    Memory(String),
}

impl fmt::Display for ElfLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotElf => write!(f, "not an ELF file"),
            Self::UnsupportedClass(class) => write!(f, "unsupported ELF class {class}"),
            Self::UnsupportedEndian(endian) => write!(f, "unsupported ELF endian {endian}"),
            Self::UnsupportedMachine(machine) => {
                write!(f, "unsupported ELF machine {machine}, expected ARM")
            }
            Self::Truncated(what) => write!(f, "truncated ELF {what}"),
            Self::BadProgramHeader => write!(f, "bad ELF program header table"),
            Self::BadLoadSegment(what) => write!(f, "bad ELF load segment: {what}"),
            Self::NoLoadSegments => write!(f, "ELF has no PT_LOAD segments"),
            Self::Memory(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ElfLoadError {}

pub fn plan_elf_load(
    path: PathBuf,
    bytes: &[u8],
    load_bias: u32,
) -> Result<ElfLoadPlan, ElfLoadError> {
    let header = parse_header(bytes)?;
    let ph_end = header
        .phoff
        .checked_add(
            header
                .phentsize
                .checked_mul(header.phnum)
                .ok_or(ElfLoadError::BadProgramHeader)?,
        )
        .ok_or(ElfLoadError::BadProgramHeader)?;
    if ph_end > bytes.len() {
        return Err(ElfLoadError::Truncated("program header table"));
    }

    let mut segments = Vec::new();
    let mut memory_base = u32::MAX;
    let mut memory_end = 0u32;
    let mut arm_exidx = None;

    for idx in 0..header.phnum {
        let off = header.phoff + idx * header.phentsize;
        let ph = parse_program_header(bytes, off, header.phentsize)?;
        if ph.p_type == PT_ARM_EXIDX {
            let addr = load_bias
                .checked_add(ph.vaddr)
                .ok_or(ElfLoadError::BadProgramHeader)?;
            arm_exidx = Some(ElfLoadRange {
                addr,
                size: ph.memsz.max(ph.filesz),
            });
            continue;
        }
        if ph.p_type != PT_LOAD {
            continue;
        }
        if ph.filesz > ph.memsz {
            return Err(ElfLoadError::BadLoadSegment(
                "file size exceeds memory size",
            ));
        }
        let file_end = ph
            .offset
            .checked_add(ph.filesz)
            .ok_or(ElfLoadError::BadLoadSegment("file range overflow"))?;
        if file_end as usize > bytes.len() {
            return Err(ElfLoadError::Truncated("load segment data"));
        }
        let memory_addr = load_bias
            .checked_add(ph.vaddr)
            .ok_or(ElfLoadError::BadLoadSegment("virtual address overflow"))?;
        let segment_end = memory_addr
            .checked_add(ph.memsz)
            .ok_or(ElfLoadError::BadLoadSegment("memory range overflow"))?;
        let page_start = align_down(memory_addr, PAGE_SIZE);
        let page_end = align_up(segment_end, PAGE_SIZE)
            .ok_or(ElfLoadError::BadLoadSegment("page range overflow"))?;

        memory_base = memory_base.min(page_start);
        memory_end = memory_end.max(page_end);
        segments.push(ElfLoadSegment {
            file_offset: ph.offset,
            file_size: ph.filesz,
            memory_addr,
            memory_size: ph.memsz,
            flags: ph.flags,
            align: ph.align,
        });
    }

    if segments.is_empty() {
        return Err(ElfLoadError::NoLoadSegments);
    }

    let memory_size = memory_end
        .checked_sub(memory_base)
        .ok_or(ElfLoadError::BadLoadSegment("empty memory range"))?;
    let entry = load_bias
        .checked_add(header.entry)
        .ok_or(ElfLoadError::BadLoadSegment("entry address overflow"))?;

    Ok(ElfLoadPlan {
        path,
        load_bias,
        entry,
        memory_base,
        memory_size,
        arm_exidx,
        segments,
    })
}

pub fn load_elf_into_vec_memory(
    bytes: &[u8],
    plan: &ElfLoadPlan,
) -> Result<VecMemory, ElfLoadError> {
    let mut memory = VecMemory::new(plan.memory_base, plan.memory_size as usize);
    load_elf_into_memory(bytes, plan, &mut memory)?;
    Ok(memory)
}

pub fn load_elf_into_memory<M: Memory>(
    bytes: &[u8],
    plan: &ElfLoadPlan,
    memory: &mut M,
) -> Result<(), ElfLoadError> {
    for segment in &plan.segments {
        let start = segment.file_offset as usize;
        let end = start
            .checked_add(segment.file_size as usize)
            .ok_or(ElfLoadError::BadLoadSegment("file range overflow"))?;
        let data = bytes
            .get(start..end)
            .ok_or(ElfLoadError::Truncated("load segment data"))?;
        for (idx, byte) in data.iter().copied().enumerate() {
            memory
                .store8(segment.memory_addr.wrapping_add(idx as u32), byte)
                .map_err(|err| ElfLoadError::Memory(err.to_string()))?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ElfHeader {
    entry: u32,
    phoff: usize,
    phentsize: usize,
    phnum: usize,
}

#[derive(Debug, Clone, Copy)]
struct ProgramHeader {
    p_type: u32,
    offset: u32,
    vaddr: u32,
    filesz: u32,
    memsz: u32,
    flags: u32,
    align: u32,
}

fn parse_header(bytes: &[u8]) -> Result<ElfHeader, ElfLoadError> {
    if bytes.len() < 52 {
        return Err(ElfLoadError::Truncated("header"));
    }
    if &bytes[0..4] != b"\x7fELF" {
        return Err(ElfLoadError::NotElf);
    }
    if bytes[4] != 1 {
        return Err(ElfLoadError::UnsupportedClass(bytes[4]));
    }
    if bytes[5] != 1 {
        return Err(ElfLoadError::UnsupportedEndian(bytes[5]));
    }
    let machine = le_u16(bytes, 18)?;
    if machine != 40 {
        return Err(ElfLoadError::UnsupportedMachine(machine));
    }
    Ok(ElfHeader {
        entry: le_u32(bytes, 24)?,
        phoff: le_u32(bytes, 28)? as usize,
        phentsize: le_u16(bytes, 42)? as usize,
        phnum: le_u16(bytes, 44)? as usize,
    })
}

fn parse_program_header(
    bytes: &[u8],
    off: usize,
    phentsize: usize,
) -> Result<ProgramHeader, ElfLoadError> {
    if phentsize < 32 {
        return Err(ElfLoadError::BadProgramHeader);
    }
    if off.checked_add(32).ok_or(ElfLoadError::BadProgramHeader)? > bytes.len() {
        return Err(ElfLoadError::Truncated("program header"));
    }
    Ok(ProgramHeader {
        p_type: le_u32(bytes, off)?,
        offset: le_u32(bytes, off + 4)?,
        vaddr: le_u32(bytes, off + 8)?,
        filesz: le_u32(bytes, off + 16)?,
        memsz: le_u32(bytes, off + 20)?,
        flags: le_u32(bytes, off + 24)?,
        align: le_u32(bytes, off + 28)?,
    })
}

fn align_down(value: u32, align: u32) -> u32 {
    value & !(align - 1)
}

fn align_up(value: u32, align: u32) -> Option<u32> {
    value
        .checked_add(align - 1)
        .map(|value| align_down(value, align))
}

fn le_u16(bytes: &[u8], off: usize) -> Result<u16, ElfLoadError> {
    let bytes = bytes
        .get(off..off + 2)
        .ok_or(ElfLoadError::Truncated("u16"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn le_u32(bytes: &[u8], off: usize) -> Result<u32, ElfLoadError> {
    let bytes = bytes
        .get(off..off + 4)
        .ok_or(ElfLoadError::Truncated("u32"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::armv7a::Memory;
    use crate::guest_memory::MappedMemory;

    use super::*;

    #[test]
    fn plans_and_loads_pt_load_segment_with_bss() {
        let mut elf = minimal_elf_with_one_load_segment(0x1000, 0x1050, &[1, 2, 3, 4], 8);
        let plan = plan_elf_load(PathBuf::from("libgame.so"), &elf, 0x7000_0000).unwrap();

        assert_eq!(plan.entry, 0x7000_1050);
        assert_eq!(plan.memory_base, 0x7000_1000);
        assert_eq!(plan.memory_size, 0x1000);
        assert_eq!(plan.segments.len(), 1);
        assert_eq!(plan.segments[0].memory_addr, 0x7000_1000);
        assert_eq!(plan.segments[0].file_size, 4);
        assert_eq!(plan.segments[0].memory_size, 8);

        let mut memory = load_elf_into_vec_memory(&elf, &plan).unwrap();
        assert_eq!(memory.base(), 0x7000_1000);
        assert_eq!(memory.len(), 0x1000);
        assert_eq!(memory.load8(0x7000_1000).unwrap(), 1);
        assert_eq!(memory.load8(0x7000_1003).unwrap(), 4);
        assert_eq!(memory.load8(0x7000_1004).unwrap(), 0);
        assert_eq!(memory.load8(0x7000_1007).unwrap(), 0);

        elf[0x100] = 9;
        assert_eq!(memory.load8(0x7000_1000).unwrap(), 1);
    }

    #[test]
    fn rejects_load_segment_with_file_size_beyond_memory_size() {
        let elf = minimal_elf_with_one_load_segment(0x1000, 0x1000, &[1, 2, 3, 4], 2);
        let err = plan_elf_load(PathBuf::from("bad.so"), &elf, 0).unwrap_err();
        assert_eq!(
            err,
            ElfLoadError::BadLoadSegment("file size exceeds memory size")
        );
    }

    #[test]
    fn rejects_elf_without_load_segments() {
        let mut elf = minimal_elf_with_one_load_segment(0x1000, 0x1000, &[1], 1);
        write_u32(&mut elf, 52, 2);
        let err = plan_elf_load(PathBuf::from("no-load.so"), &elf, 0).unwrap_err();
        assert_eq!(err, ElfLoadError::NoLoadSegments);
    }

    #[test]
    fn loads_segments_into_mapped_memory() {
        let elf = minimal_elf_with_one_load_segment(0x2000, 0x2000, &[0xaa, 0xbb], 4);
        let plan = plan_elf_load(PathBuf::from("mapped.so"), &elf, 0x7100_0000).unwrap();
        let mut memory = MappedMemory::new();
        memory
            .map_zeroed(plan.memory_base, plan.memory_size as usize)
            .unwrap();

        load_elf_into_memory(&elf, &plan, &mut memory).unwrap();

        assert_eq!(memory.load8(0x7100_2000).unwrap(), 0xaa);
        assert_eq!(memory.load8(0x7100_2001).unwrap(), 0xbb);
        assert_eq!(memory.load8(0x7100_2002).unwrap(), 0);
    }

    fn minimal_elf_with_one_load_segment(
        vaddr: u32,
        entry: u32,
        data: &[u8],
        memsz: u32,
    ) -> Vec<u8> {
        let mut bytes = vec![0; 0x100 + data.len()];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 1;
        bytes[5] = 1;
        bytes[6] = 1;
        write_u16(&mut bytes, 16, 3);
        write_u16(&mut bytes, 18, 40);
        write_u32(&mut bytes, 20, 1);
        write_u32(&mut bytes, 24, entry);
        write_u32(&mut bytes, 28, 52);
        write_u32(&mut bytes, 36, 0x0500_0000);
        write_u16(&mut bytes, 42, 32);
        write_u16(&mut bytes, 44, 1);

        write_u32(&mut bytes, 52, PT_LOAD);
        write_u32(&mut bytes, 56, 0x100);
        write_u32(&mut bytes, 60, vaddr);
        write_u32(&mut bytes, 64, vaddr);
        write_u32(&mut bytes, 68, data.len() as u32);
        write_u32(&mut bytes, 72, memsz);
        write_u32(&mut bytes, 76, 5);
        write_u32(&mut bytes, 80, PAGE_SIZE);
        bytes[0x100..0x100 + data.len()].copy_from_slice(data);
        bytes
    }

    fn write_u16(out: &mut [u8], off: usize, value: u16) {
        out[off..off + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(out: &mut [u8], off: usize, value: u32) {
        out[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
}
