use std::fmt;
use std::path::PathBuf;

const EM_ARM: u16 = 40;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const SHT_STRTAB: u32 = 3;
const SHT_DYNSYM: u32 = 11;
const SHN_UNDEF: u16 = 0;

const DT_NULL: u32 = 0;
const DT_NEEDED: u32 = 1;
const DT_PLTRELSZ: u32 = 2;
const DT_STRTAB: u32 = 5;
const DT_SYMTAB: u32 = 6;
const DT_STRSZ: u32 = 10;
const DT_SYMENT: u32 = 11;
const DT_INIT: u32 = 12;
const DT_REL: u32 = 17;
const DT_RELSZ: u32 = 18;
const DT_RELENT: u32 = 19;
const DT_PLTREL: u32 = 20;
const DT_JMPREL: u32 = 23;
const DT_INIT_ARRAY: u32 = 25;
const DT_INIT_ARRAYSZ: u32 = 27;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfDynamicInfo {
    pub path: PathBuf,
    pub needed: Vec<String>,
    pub imports: Vec<ElfImport>,
    pub dynsym: Option<ElfSymbolTableInfo>,
    pub relocations: ElfRelocationInfo,
    pub init: Option<u32>,
    pub init_array: Option<ElfTableRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfImport {
    pub name: String,
    pub binding: SymbolBinding,
    pub kind: SymbolKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElfSymbolTableInfo {
    pub addr: u32,
    pub entry_size: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolBinding {
    Local,
    Global,
    Weak,
    Other(u8),
}

impl fmt::Display for SymbolBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::Global => write!(f, "global"),
            Self::Weak => write!(f, "weak"),
            Self::Other(value) => write!(f, "bind({value})"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    NoType,
    Object,
    Func,
    Section,
    File,
    Common,
    Tls,
    Other(u8),
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoType => write!(f, "notype"),
            Self::Object => write!(f, "object"),
            Self::Func => write!(f, "func"),
            Self::Section => write!(f, "section"),
            Self::File => write!(f, "file"),
            Self::Common => write!(f, "common"),
            Self::Tls => write!(f, "tls"),
            Self::Other(value) => write!(f, "type({value})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ElfRelocationInfo {
    pub rel: Option<ElfTableRange>,
    pub rel_entry_size: Option<u32>,
    pub plt_rel: Option<ElfTableRange>,
    pub plt_rel_kind: Option<PltRelKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElfTableRange {
    pub addr: u32,
    pub size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PltRelKind {
    Rel,
    Rela,
    Other(u32),
}

impl fmt::Display for PltRelKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rel => write!(f, "REL"),
            Self::Rela => write!(f, "RELA"),
            Self::Other(value) => write!(f, "{value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfDynamicError {
    NotElf,
    UnsupportedClass(u8),
    UnsupportedEndian(u8),
    UnsupportedMachine(u16),
    Truncated(&'static str),
    BadProgramHeader,
    BadSectionHeader,
    BadDynamicTable,
    BadStringTable,
}

impl fmt::Display for ElfDynamicError {
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
            Self::BadSectionHeader => write!(f, "bad ELF section header table"),
            Self::BadDynamicTable => write!(f, "bad ELF dynamic table"),
            Self::BadStringTable => write!(f, "bad ELF string table"),
        }
    }
}

impl std::error::Error for ElfDynamicError {}

pub fn parse_elf_dynamic_bytes(
    path: PathBuf,
    bytes: &[u8],
) -> Result<ElfDynamicInfo, ElfDynamicError> {
    let header = parse_header(bytes)?;
    let programs = parse_program_headers(bytes, &header)?;
    let sections = parse_section_headers(bytes, &header)?;
    let dynamics = parse_dynamic_entries(bytes, &programs)?;
    let needed = parse_needed(bytes, &programs, &dynamics)?;
    let imports = parse_imports(bytes, &sections)?;

    Ok(ElfDynamicInfo {
        path,
        needed,
        imports,
        dynsym: dynamic_value(&dynamics, DT_SYMTAB).map(|addr| ElfSymbolTableInfo {
            addr,
            entry_size: dynamic_value(&dynamics, DT_SYMENT),
        }),
        relocations: parse_relocation_info(&dynamics),
        init: dynamic_value(&dynamics, DT_INIT),
        init_array: match (
            dynamic_value(&dynamics, DT_INIT_ARRAY),
            dynamic_value(&dynamics, DT_INIT_ARRAYSZ),
        ) {
            (Some(addr), Some(size)) if size != 0 => Some(ElfTableRange { addr, size }),
            _ => None,
        },
    })
}

#[derive(Debug, Clone, Copy)]
struct ElfHeader {
    phoff: usize,
    phentsize: usize,
    phnum: usize,
    shoff: usize,
    shentsize: usize,
    shnum: usize,
}

#[derive(Debug, Clone, Copy)]
struct ProgramHeader {
    p_type: u32,
    offset: u32,
    vaddr: u32,
    filesz: u32,
    memsz: u32,
}

#[derive(Debug, Clone, Copy)]
struct SectionHeader {
    sh_type: u32,
    offset: u32,
    size: u32,
    link: u32,
    entsize: u32,
}

#[derive(Debug, Clone, Copy)]
struct DynamicEntry {
    tag: u32,
    value: u32,
}

fn parse_header(bytes: &[u8]) -> Result<ElfHeader, ElfDynamicError> {
    if bytes.len() < 52 {
        return Err(ElfDynamicError::Truncated("header"));
    }
    if &bytes[0..4] != b"\x7fELF" {
        return Err(ElfDynamicError::NotElf);
    }
    if bytes[4] != 1 {
        return Err(ElfDynamicError::UnsupportedClass(bytes[4]));
    }
    if bytes[5] != 1 {
        return Err(ElfDynamicError::UnsupportedEndian(bytes[5]));
    }
    let machine = le_u16(bytes, 18)?;
    if machine != EM_ARM {
        return Err(ElfDynamicError::UnsupportedMachine(machine));
    }
    Ok(ElfHeader {
        phoff: le_u32(bytes, 28)? as usize,
        shoff: le_u32(bytes, 32)? as usize,
        phentsize: le_u16(bytes, 42)? as usize,
        phnum: le_u16(bytes, 44)? as usize,
        shentsize: le_u16(bytes, 46)? as usize,
        shnum: le_u16(bytes, 48)? as usize,
    })
}

fn parse_program_headers(
    bytes: &[u8],
    header: &ElfHeader,
) -> Result<Vec<ProgramHeader>, ElfDynamicError> {
    if header.phnum == 0 {
        return Ok(Vec::new());
    }
    if header.phentsize < 32 {
        return Err(ElfDynamicError::BadProgramHeader);
    }
    let ph_end = header
        .phoff
        .checked_add(
            header
                .phentsize
                .checked_mul(header.phnum)
                .ok_or(ElfDynamicError::BadProgramHeader)?,
        )
        .ok_or(ElfDynamicError::BadProgramHeader)?;
    if ph_end > bytes.len() {
        return Err(ElfDynamicError::Truncated("program header table"));
    }

    let mut out = Vec::with_capacity(header.phnum);
    for idx in 0..header.phnum {
        let off = header.phoff + idx * header.phentsize;
        out.push(ProgramHeader {
            p_type: le_u32(bytes, off)?,
            offset: le_u32(bytes, off + 4)?,
            vaddr: le_u32(bytes, off + 8)?,
            filesz: le_u32(bytes, off + 16)?,
            memsz: le_u32(bytes, off + 20)?,
        });
    }
    Ok(out)
}

fn parse_section_headers(
    bytes: &[u8],
    header: &ElfHeader,
) -> Result<Vec<SectionHeader>, ElfDynamicError> {
    if header.shnum == 0 {
        return Ok(Vec::new());
    }
    if header.shentsize < 40 {
        return Err(ElfDynamicError::BadSectionHeader);
    }
    let sh_end = header
        .shoff
        .checked_add(
            header
                .shentsize
                .checked_mul(header.shnum)
                .ok_or(ElfDynamicError::BadSectionHeader)?,
        )
        .ok_or(ElfDynamicError::BadSectionHeader)?;
    if sh_end > bytes.len() {
        return Err(ElfDynamicError::Truncated("section header table"));
    }

    let mut out = Vec::with_capacity(header.shnum);
    for idx in 0..header.shnum {
        let off = header.shoff + idx * header.shentsize;
        out.push(SectionHeader {
            sh_type: le_u32(bytes, off + 4)?,
            offset: le_u32(bytes, off + 16)?,
            size: le_u32(bytes, off + 20)?,
            link: le_u32(bytes, off + 24)?,
            entsize: le_u32(bytes, off + 36)?,
        });
    }
    Ok(out)
}

fn parse_dynamic_entries(
    bytes: &[u8],
    programs: &[ProgramHeader],
) -> Result<Vec<DynamicEntry>, ElfDynamicError> {
    let Some(dynamic) = programs.iter().find(|program| program.p_type == PT_DYNAMIC) else {
        return Ok(Vec::new());
    };
    let start = dynamic.offset as usize;
    let end = start
        .checked_add(dynamic.filesz as usize)
        .ok_or(ElfDynamicError::BadDynamicTable)?;
    if end > bytes.len() {
        return Err(ElfDynamicError::Truncated("dynamic table"));
    }

    let mut out = Vec::new();
    let mut pos = start;
    while pos + 8 <= end {
        let tag = le_u32(bytes, pos)?;
        let value = le_u32(bytes, pos + 4)?;
        if tag == DT_NULL {
            break;
        }
        out.push(DynamicEntry { tag, value });
        pos += 8;
    }
    Ok(out)
}

fn parse_needed(
    bytes: &[u8],
    programs: &[ProgramHeader],
    dynamics: &[DynamicEntry],
) -> Result<Vec<String>, ElfDynamicError> {
    let Some(strtab_addr) = dynamic_value(dynamics, DT_STRTAB) else {
        return Ok(Vec::new());
    };
    let strsz = dynamic_value(dynamics, DT_STRSZ).unwrap_or(u32::MAX);
    let Some(strtab_file_off) = virtual_to_file_offset(programs, strtab_addr) else {
        return Err(ElfDynamicError::BadStringTable);
    };
    let strtab = bounded_bytes(bytes, strtab_file_off, strsz)?;

    let mut needed = Vec::new();
    for entry in dynamics.iter().filter(|entry| entry.tag == DT_NEEDED) {
        needed.push(read_string(strtab, entry.value as usize)?);
    }
    Ok(needed)
}

fn parse_imports(
    bytes: &[u8],
    sections: &[SectionHeader],
) -> Result<Vec<ElfImport>, ElfDynamicError> {
    let mut imports = Vec::new();
    for section in sections
        .iter()
        .filter(|section| section.sh_type == SHT_DYNSYM)
    {
        let entsize = if section.entsize == 0 {
            16
        } else {
            section.entsize
        };
        if entsize < 16 {
            return Err(ElfDynamicError::BadSectionHeader);
        }
        let Some(strtab) = sections.get(section.link as usize) else {
            return Err(ElfDynamicError::BadSectionHeader);
        };
        if strtab.sh_type != SHT_STRTAB {
            return Err(ElfDynamicError::BadSectionHeader);
        }
        let symbols = bounded_bytes(bytes, section.offset, section.size)?;
        let strings = bounded_bytes(bytes, strtab.offset, strtab.size)?;
        let count = symbols.len() / entsize as usize;
        for idx in 0..count {
            let off = idx * entsize as usize;
            let name_off = le_u32(symbols, off)? as usize;
            let info = symbols
                .get(off + 12)
                .copied()
                .ok_or(ElfDynamicError::Truncated("dynamic symbol"))?;
            let shndx = le_u16(symbols, off + 14)?;
            if shndx != SHN_UNDEF || name_off == 0 {
                continue;
            }
            let name = read_string(strings, name_off)?;
            if name.is_empty() {
                continue;
            }
            imports.push(ElfImport {
                name,
                binding: SymbolBinding::from(info >> 4),
                kind: SymbolKind::from(info & 0xf),
            });
        }
    }
    Ok(imports)
}

fn parse_relocation_info(dynamics: &[DynamicEntry]) -> ElfRelocationInfo {
    let rel = match (
        dynamic_value(dynamics, DT_REL),
        dynamic_value(dynamics, DT_RELSZ),
    ) {
        (Some(addr), Some(size)) if size != 0 => Some(ElfTableRange { addr, size }),
        _ => None,
    };
    let plt_rel = match (
        dynamic_value(dynamics, DT_JMPREL),
        dynamic_value(dynamics, DT_PLTRELSZ),
    ) {
        (Some(addr), Some(size)) if size != 0 => Some(ElfTableRange { addr, size }),
        _ => None,
    };
    ElfRelocationInfo {
        rel,
        rel_entry_size: dynamic_value(dynamics, DT_RELENT),
        plt_rel,
        plt_rel_kind: dynamic_value(dynamics, DT_PLTREL).map(PltRelKind::from),
    }
}

fn dynamic_value(dynamics: &[DynamicEntry], tag: u32) -> Option<u32> {
    dynamics
        .iter()
        .find(|entry| entry.tag == tag)
        .map(|entry| entry.value)
}

fn virtual_to_file_offset(programs: &[ProgramHeader], addr: u32) -> Option<u32> {
    for program in programs.iter().filter(|program| program.p_type == PT_LOAD) {
        let mem_end = program.vaddr.checked_add(program.memsz)?;
        let file_end = program.vaddr.checked_add(program.filesz)?;
        if addr >= program.vaddr && addr < mem_end && addr < file_end {
            return program.offset.checked_add(addr - program.vaddr);
        }
    }
    None
}

fn bounded_bytes(bytes: &[u8], offset: u32, requested_size: u32) -> Result<&[u8], ElfDynamicError> {
    let start = offset as usize;
    let available = bytes
        .len()
        .checked_sub(start)
        .ok_or(ElfDynamicError::Truncated("table"))?;
    let size = if requested_size == u32::MAX {
        available
    } else {
        (requested_size as usize).min(available)
    };
    bytes
        .get(start..start + size)
        .ok_or(ElfDynamicError::Truncated("table"))
}

fn read_string(bytes: &[u8], off: usize) -> Result<String, ElfDynamicError> {
    let rest = bytes.get(off..).ok_or(ElfDynamicError::BadStringTable)?;
    let end = rest
        .iter()
        .position(|&byte| byte == 0)
        .ok_or(ElfDynamicError::BadStringTable)?;
    std::str::from_utf8(&rest[..end])
        .map(|value| value.to_string())
        .map_err(|_| ElfDynamicError::BadStringTable)
}

impl From<u8> for SymbolBinding {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Local,
            1 => Self::Global,
            2 => Self::Weak,
            other => Self::Other(other),
        }
    }
}

impl From<u8> for SymbolKind {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::NoType,
            1 => Self::Object,
            2 => Self::Func,
            3 => Self::Section,
            4 => Self::File,
            5 => Self::Common,
            6 => Self::Tls,
            other => Self::Other(other),
        }
    }
}

impl From<u32> for PltRelKind {
    fn from(value: u32) -> Self {
        match value {
            DT_REL => Self::Rel,
            7 => Self::Rela,
            other => Self::Other(other),
        }
    }
}

fn le_u16(bytes: &[u8], off: usize) -> Result<u16, ElfDynamicError> {
    let bytes = bytes
        .get(off..off + 2)
        .ok_or(ElfDynamicError::Truncated("u16"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn le_u32(bytes: &[u8], off: usize) -> Result<u32, ElfDynamicError> {
    let bytes = bytes
        .get(off..off + 4)
        .ok_or(ElfDynamicError::Truncated("u32"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn parses_needed_imports_and_relocation_metadata() {
        let elf = dynamic_test_elf();
        let info = parse_elf_dynamic_bytes(PathBuf::from("libgame.so"), &elf).unwrap();

        assert_eq!(info.needed, vec!["libc.so", "libGLESv2.so"]);
        assert_eq!(
            info.dynsym,
            Some(ElfSymbolTableInfo {
                addr: 0x11c0,
                entry_size: Some(16)
            })
        );
        assert_eq!(
            info.imports,
            vec![
                ElfImport {
                    name: "puts".to_string(),
                    binding: SymbolBinding::Global,
                    kind: SymbolKind::Func,
                },
                ElfImport {
                    name: "glDrawArrays".to_string(),
                    binding: SymbolBinding::Weak,
                    kind: SymbolKind::Func,
                },
            ]
        );
        assert_eq!(
            info.relocations.rel,
            Some(ElfTableRange {
                addr: 0x1500,
                size: 24
            })
        );
        assert_eq!(info.relocations.rel_entry_size, Some(8));
        assert_eq!(
            info.relocations.plt_rel,
            Some(ElfTableRange {
                addr: 0x1600,
                size: 16
            })
        );
        assert_eq!(info.relocations.plt_rel_kind, Some(PltRelKind::Rel));
        assert_eq!(info.init, Some(0x1234));
        assert_eq!(
            info.init_array,
            Some(ElfTableRange {
                addr: 0x1700,
                size: 8
            })
        );
    }

    #[test]
    fn accepts_elf_without_dynamic_segment_or_dynsym() {
        let mut elf = vec![0; 52];
        write_elf_header(&mut elf, 0, 0, 0, 0);
        let info = parse_elf_dynamic_bytes(PathBuf::from("plain.so"), &elf).unwrap();
        assert!(info.needed.is_empty());
        assert!(info.imports.is_empty());
        assert_eq!(info.relocations, ElfRelocationInfo::default());
    }

    #[test]
    fn rejects_non_arm_elf() {
        let mut elf = vec![0; 52];
        write_elf_header(&mut elf, 0, 0, 0, 0);
        write_u16(&mut elf, 18, 3);
        let err = parse_elf_dynamic_bytes(PathBuf::from("x86.so"), &elf).unwrap_err();
        assert_eq!(err, ElfDynamicError::UnsupportedMachine(3));
    }

    #[test]
    fn parses_local_minecraft_dynamic_imports_when_present() {
        let apk = std::path::Path::new("/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk");
        if !apk.exists() {
            return;
        }
        let bytes =
            crate::zip_probe::read_zip_entry(apk, "lib/armeabi-v7a/libminecraftpe.so").unwrap();
        let info = parse_elf_dynamic_bytes(
            PathBuf::from("MineCraftPE-a0.15.0.1.apk!/lib/armeabi-v7a/libminecraftpe.so"),
            &bytes,
        )
        .unwrap();

        assert!(info.needed.iter().any(|needed| needed == "libGLESv2.so"));
        assert!(
            info.imports
                .iter()
                .any(|import| import.name == "glCreateShader")
        );
        assert!(
            info.imports
                .iter()
                .any(|import| import.name == "eglGetProcAddress")
        );
    }

    fn dynamic_test_elf() -> Vec<u8> {
        let dyn_off = 0x100usize;
        let dyn_vaddr = 0x1100u32;
        let dynstr_off = 0x180usize;
        let dynstr_vaddr = 0x1180u32;
        let dynsym_off = 0x1c0usize;
        let dynsym_vaddr = 0x11c0u32;
        let shstr_off = 0x220usize;
        let shoff = 0x260usize;

        let mut dynstr = Vec::new();
        dynstr.push(0);
        let libc = push_str(&mut dynstr, "libc.so");
        let gles = push_str(&mut dynstr, "libGLESv2.so");
        let puts = push_str(&mut dynstr, "puts");
        let draw = push_str(&mut dynstr, "glDrawArrays");

        let mut dynsym = Vec::new();
        dynsym.resize(16, 0);
        push_dynsym(&mut dynsym, puts, 0x12, SHN_UNDEF);
        push_dynsym(&mut dynsym, draw, 0x22, SHN_UNDEF);

        let mut shstr = Vec::new();
        shstr.push(0);
        let dynsym_name = push_str(&mut shstr, ".dynsym");
        let dynstr_name = push_str(&mut shstr, ".dynstr");
        let shstr_name = push_str(&mut shstr, ".shstrtab");

        let mut bytes = vec![0; shoff + 4 * 40];
        write_elf_header(&mut bytes, 52, 2, shoff as u32, 4);

        write_phdr(
            &mut bytes,
            52,
            PT_LOAD,
            0,
            0x1000,
            shoff as u32,
            shoff as u32,
        );
        write_phdr(
            &mut bytes,
            84,
            PT_DYNAMIC,
            dyn_off as u32,
            dyn_vaddr,
            16 * 8,
            16 * 8,
        );

        let mut dyn_entries = Vec::new();
        push_dynamic(&mut dyn_entries, DT_NEEDED, libc);
        push_dynamic(&mut dyn_entries, DT_NEEDED, gles);
        push_dynamic(&mut dyn_entries, DT_STRTAB, dynstr_vaddr);
        push_dynamic(&mut dyn_entries, DT_STRSZ, dynstr.len() as u32);
        push_dynamic(&mut dyn_entries, DT_SYMTAB, dynsym_vaddr);
        push_dynamic(&mut dyn_entries, DT_SYMENT, 16);
        push_dynamic(&mut dyn_entries, DT_REL, 0x1500);
        push_dynamic(&mut dyn_entries, DT_RELSZ, 24);
        push_dynamic(&mut dyn_entries, DT_RELENT, 8);
        push_dynamic(&mut dyn_entries, DT_PLTREL, DT_REL);
        push_dynamic(&mut dyn_entries, DT_JMPREL, 0x1600);
        push_dynamic(&mut dyn_entries, DT_PLTRELSZ, 16);
        push_dynamic(&mut dyn_entries, DT_INIT, 0x1234);
        push_dynamic(&mut dyn_entries, DT_INIT_ARRAY, 0x1700);
        push_dynamic(&mut dyn_entries, DT_INIT_ARRAYSZ, 8);
        push_dynamic(&mut dyn_entries, DT_NULL, 0);

        bytes[dyn_off..dyn_off + dyn_entries.len()].copy_from_slice(&dyn_entries);
        bytes[dynstr_off..dynstr_off + dynstr.len()].copy_from_slice(&dynstr);
        bytes[dynsym_off..dynsym_off + dynsym.len()].copy_from_slice(&dynsym);
        bytes[shstr_off..shstr_off + shstr.len()].copy_from_slice(&shstr);

        write_shdr(
            &mut bytes,
            shoff + 40,
            dynsym_name,
            SHT_DYNSYM,
            dynsym_off as u32,
            dynsym.len() as u32,
            2,
            16,
        );
        write_shdr(
            &mut bytes,
            shoff + 80,
            dynstr_name,
            SHT_STRTAB,
            dynstr_off as u32,
            dynstr.len() as u32,
            0,
            0,
        );
        write_shdr(
            &mut bytes,
            shoff + 120,
            shstr_name,
            SHT_STRTAB,
            shstr_off as u32,
            shstr.len() as u32,
            0,
            0,
        );
        bytes
    }

    fn write_elf_header(bytes: &mut [u8], phoff: u32, phnum: u16, shoff: u32, shnum: u16) {
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 1;
        bytes[5] = 1;
        bytes[6] = 1;
        write_u16(bytes, 16, 3);
        write_u16(bytes, 18, EM_ARM);
        write_u32(bytes, 20, 1);
        write_u32(bytes, 28, phoff);
        write_u32(bytes, 32, shoff);
        write_u32(bytes, 36, 0x0500_0000);
        write_u16(bytes, 42, 32);
        write_u16(bytes, 44, phnum);
        write_u16(bytes, 46, 40);
        write_u16(bytes, 48, shnum);
    }

    fn write_phdr(
        bytes: &mut [u8],
        off: usize,
        p_type: u32,
        file_off: u32,
        vaddr: u32,
        filesz: u32,
        memsz: u32,
    ) {
        write_u32(bytes, off, p_type);
        write_u32(bytes, off + 4, file_off);
        write_u32(bytes, off + 8, vaddr);
        write_u32(bytes, off + 12, vaddr);
        write_u32(bytes, off + 16, filesz);
        write_u32(bytes, off + 20, memsz);
        write_u32(bytes, off + 24, 4);
        write_u32(bytes, off + 28, 4);
    }

    fn write_shdr(
        bytes: &mut [u8],
        off: usize,
        name: u32,
        sh_type: u32,
        file_off: u32,
        size: u32,
        link: u32,
        entsize: u32,
    ) {
        write_u32(bytes, off, name);
        write_u32(bytes, off + 4, sh_type);
        write_u32(bytes, off + 16, file_off);
        write_u32(bytes, off + 20, size);
        write_u32(bytes, off + 24, link);
        write_u32(bytes, off + 36, entsize);
    }

    fn push_dynsym(out: &mut Vec<u8>, name: u32, info: u8, shndx: u16) {
        push_u32(out, name);
        push_u32(out, 0);
        push_u32(out, 0);
        out.push(info);
        out.push(0);
        push_u16(out, shndx);
    }

    fn push_dynamic(out: &mut Vec<u8>, tag: u32, value: u32) {
        push_u32(out, tag);
        push_u32(out, value);
    }

    fn push_str(out: &mut Vec<u8>, value: &str) -> u32 {
        let offset = out.len() as u32;
        out.extend_from_slice(value.as_bytes());
        out.push(0);
        offset
    }

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u16(out: &mut [u8], off: usize, value: u16) {
        out[off..off + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(out: &mut [u8], off: usize, value: u32) {
        out[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
}
