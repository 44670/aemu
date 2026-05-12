use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfProbe {
    pub path: PathBuf,
    pub machine: String,
    pub eabi: String,
    pub attributes: Vec<ArmAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmAttribute {
    pub tag: u64,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfProbeError {
    Io(String),
    NotElf,
    UnsupportedClass(u8),
    UnsupportedEndian(u8),
    Truncated(&'static str),
}

impl fmt::Display for ElfProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::NotElf => write!(f, "not an ELF file"),
            Self::UnsupportedClass(class) => write!(f, "unsupported ELF class {class}"),
            Self::UnsupportedEndian(endian) => write!(f, "unsupported ELF endian {endian}"),
            Self::Truncated(what) => write!(f, "truncated {what}"),
        }
    }
}

impl std::error::Error for ElfProbeError {}

impl ElfProbe {
    pub fn requires_armv7_or_newer(&self) -> bool {
        self.attr_value("Tag_CPU_arch")
            .map(|value| {
                matches!(
                    value,
                    "v7" | "v7E-M" | "v8" | "v8-M.baseline" | "v8-M.mainline" | "v8.1-M.mainline"
                )
            })
            .unwrap_or(false)
    }

    pub fn requires_thumb2(&self) -> bool {
        self.attr_value("Tag_THUMB_ISA_use")
            .map(|value| value.contains("Thumb-2"))
            .unwrap_or(false)
    }

    pub fn requires_vfp(&self) -> bool {
        self.attr_value("Tag_FP_arch")
            .map(|value| value != "None" && value != "Allowed")
            .unwrap_or(false)
    }

    pub fn requires_neon(&self) -> bool {
        self.attr_value("Tag_Advanced_SIMD_arch")
            .map(|value| value != "None")
            .unwrap_or(false)
    }

    pub fn fp_arch(&self) -> Option<&str> {
        self.attr_value("Tag_FP_arch")
    }

    pub fn requires_vfp3_or_newer(&self) -> bool {
        self.fp_arch()
            .map(|value| {
                matches!(
                    value,
                    "VFPv3" | "VFPv3-D16" | "VFPv4" | "VFPv4-D16" | "FPv5-A" | "FPv5-D16"
                )
            })
            .unwrap_or(false)
    }

    pub fn attr_value(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attr| attr.name == name)
            .map(|attr| attr.value.as_str())
    }
}

pub fn probe_arm_elf(path: &Path) -> Result<ElfProbe, ElfProbeError> {
    let bytes = fs::read(path).map_err(|err| ElfProbeError::Io(err.to_string()))?;
    probe_arm_elf_bytes(path.to_path_buf(), &bytes)
}

pub fn probe_arm_elf_bytes(path: PathBuf, bytes: &[u8]) -> Result<ElfProbe, ElfProbeError> {
    if bytes.len() < 52 {
        return Err(ElfProbeError::Truncated("ELF header"));
    }
    if &bytes[0..4] != b"\x7fELF" {
        return Err(ElfProbeError::NotElf);
    }
    if bytes[4] != 1 {
        return Err(ElfProbeError::UnsupportedClass(bytes[4]));
    }
    if bytes[5] != 1 {
        return Err(ElfProbeError::UnsupportedEndian(bytes[5]));
    }

    let machine_raw = le_u16(&bytes, 18)?;
    let flags = le_u32(&bytes, 36)?;
    let shoff = le_u32(&bytes, 32)? as usize;
    let shentsize = le_u16(&bytes, 46)? as usize;
    let shnum = le_u16(&bytes, 48)? as usize;
    let shstrndx = le_u16(&bytes, 50)? as usize;

    let mut sections = Vec::with_capacity(shnum);
    for idx in 0..shnum {
        let off = shoff + idx * shentsize;
        if off + 40 > bytes.len() {
            return Err(ElfProbeError::Truncated("section header"));
        }
        sections.push(Section {
            name_off: le_u32(&bytes, off)? as usize,
            offset: le_u32(&bytes, off + 16)? as usize,
            size: le_u32(&bytes, off + 20)? as usize,
        });
    }

    let shstr = sections
        .get(shstrndx)
        .and_then(|section| section_bytes(&bytes, section))
        .ok_or(ElfProbeError::Truncated("section string table"))?;

    let attributes = sections
        .iter()
        .find(|section| section_name(shstr, section.name_off) == Some(".ARM.attributes"))
        .and_then(|section| section_bytes(&bytes, section))
        .map(parse_arm_attributes)
        .unwrap_or_default();

    Ok(ElfProbe {
        path,
        machine: match machine_raw {
            40 => "ARM".to_string(),
            other => format!("unknown({other})"),
        },
        eabi: format!("EABI{}", (flags >> 24) & 0xff),
        attributes,
    })
}

#[derive(Debug, Clone, Copy)]
struct Section {
    name_off: usize,
    offset: usize,
    size: usize,
}

fn section_bytes<'a>(bytes: &'a [u8], section: &Section) -> Option<&'a [u8]> {
    bytes.get(section.offset..section.offset.checked_add(section.size)?)
}

fn section_name(strings: &[u8], off: usize) -> Option<&str> {
    let rest = strings.get(off..)?;
    let end = rest.iter().position(|&byte| byte == 0)?;
    std::str::from_utf8(&rest[..end]).ok()
}

fn parse_arm_attributes(bytes: &[u8]) -> Vec<ArmAttribute> {
    if bytes.first() != Some(&b'A') {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut pos = 1usize;
    while pos + 4 <= bytes.len() {
        let subsection_len = le_u32_lossy(bytes, pos) as usize;
        if subsection_len < 4 || pos + subsection_len > bytes.len() {
            break;
        }
        let vendor_start = pos + 4;
        let Some(vendor_end_rel) = bytes[vendor_start..pos + subsection_len]
            .iter()
            .position(|&byte| byte == 0)
        else {
            break;
        };
        let vendor_end = vendor_start + vendor_end_rel;
        let vendor = std::str::from_utf8(&bytes[vendor_start..vendor_end]).unwrap_or("");
        let mut subpos = vendor_end + 1;
        let subsection_end = pos + subsection_len;
        if vendor == "aeabi" {
            while subpos < subsection_end {
                let tag_start = subpos;
                let (tag, tag_len) = read_uleb128(&bytes[subpos..subsection_end]);
                subpos += tag_len;
                if tag_len == 0 || subpos + 4 > subsection_end {
                    break;
                }
                let len = le_u32_lossy(bytes, subpos) as usize;
                let payload_start = subpos + 4;
                let payload_end = tag_start + len;
                if len < tag_len + 4 || payload_end > subsection_end {
                    break;
                }
                if tag == 1 {
                    parse_file_attribute_payload(&bytes[payload_start..payload_end], &mut out);
                }
                subpos = payload_end;
            }
        }
        pos += subsection_len;
    }
    out
}

fn parse_file_attribute_payload(mut payload: &[u8], out: &mut Vec<ArmAttribute>) {
    while !payload.is_empty() {
        let (tag, tag_len) = read_uleb128(payload);
        if tag_len == 0 {
            break;
        }
        payload = &payload[tag_len..];
        let Some(spec) = attr_spec(tag) else {
            let (value, value_len) = read_uleb128(payload);
            if value_len == 0 {
                break;
            }
            out.push(ArmAttribute {
                tag,
                name: format!("Tag_{tag}"),
                value: value.to_string(),
            });
            payload = &payload[value_len..];
            continue;
        };

        match spec.kind {
            AttrKind::String => {
                let Some(end) = payload.iter().position(|&byte| byte == 0) else {
                    break;
                };
                let value = std::str::from_utf8(&payload[..end])
                    .unwrap_or("")
                    .to_string();
                out.push(ArmAttribute {
                    tag,
                    name: spec.name.to_string(),
                    value,
                });
                payload = &payload[end + 1..];
            }
            AttrKind::Uleb(mapping) => {
                let (raw, raw_len) = read_uleb128(payload);
                if raw_len == 0 {
                    break;
                }
                let value = mapping
                    .iter()
                    .find(|(candidate, _)| *candidate == raw)
                    .map(|(_, name)| (*name).to_string())
                    .unwrap_or_else(|| raw.to_string());
                out.push(ArmAttribute {
                    tag,
                    name: spec.name.to_string(),
                    value,
                });
                payload = &payload[raw_len..];
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AttrSpec {
    name: &'static str,
    kind: AttrKind,
}

#[derive(Debug, Clone, Copy)]
enum AttrKind {
    String,
    Uleb(&'static [(u64, &'static str)]),
}

fn attr_spec(tag: u64) -> Option<AttrSpec> {
    Some(match tag {
        4 => AttrSpec {
            name: "Tag_CPU_raw_name",
            kind: AttrKind::String,
        },
        5 => AttrSpec {
            name: "Tag_CPU_name",
            kind: AttrKind::String,
        },
        6 => AttrSpec {
            name: "Tag_CPU_arch",
            kind: AttrKind::Uleb(&[
                (0, "Pre-v4"),
                (1, "v4"),
                (2, "v4T"),
                (3, "v5T"),
                (4, "v5TE"),
                (5, "v5TEJ"),
                (6, "v6"),
                (7, "v6KZ"),
                (8, "v6T2"),
                (9, "v6K"),
                (10, "v7"),
                (11, "v6-M"),
                (12, "v6S-M"),
                (13, "v7E-M"),
                (14, "v8"),
                (15, "v8-R"),
                (16, "v8-M.baseline"),
                (17, "v8-M.mainline"),
                (21, "v8.1-M.mainline"),
            ]),
        },
        7 => AttrSpec {
            name: "Tag_CPU_arch_profile",
            kind: AttrKind::Uleb(&[
                (0, "None"),
                (65, "Application"),
                (82, "Real-time"),
                (77, "Microcontroller"),
                (83, "Application or Real-time"),
            ]),
        },
        8 => AttrSpec {
            name: "Tag_ARM_ISA_use",
            kind: AttrKind::Uleb(&[(0, "No"), (1, "Yes")]),
        },
        9 => AttrSpec {
            name: "Tag_THUMB_ISA_use",
            kind: AttrKind::Uleb(&[(0, "No"), (1, "Thumb-1"), (2, "Thumb-2")]),
        },
        10 => AttrSpec {
            name: "Tag_FP_arch",
            kind: AttrKind::Uleb(&[
                (0, "None"),
                (1, "VFPv1"),
                (2, "VFPv2"),
                (3, "VFPv3"),
                (4, "VFPv3-D16"),
                (5, "VFPv4"),
                (6, "VFPv4-D16"),
                (7, "FPv5-A"),
                (8, "FPv5-D16"),
            ]),
        },
        12 => AttrSpec {
            name: "Tag_Advanced_SIMD_arch",
            kind: AttrKind::Uleb(&[
                (0, "None"),
                (1, "NEONv1"),
                (2, "NEONv1 with Fused-MAC"),
                (3, "ARMv8 NEON"),
            ]),
        },
        24 => AttrSpec {
            name: "Tag_ABI_align_needed",
            kind: AttrKind::Uleb(&[(0, "No"), (1, "8-byte"), (2, "4-byte")]),
        },
        25 => AttrSpec {
            name: "Tag_ABI_align_preserved",
            kind: AttrKind::Uleb(&[(0, "No"), (1, "8-byte except leaf SP"), (2, "8-byte")]),
        },
        28 => AttrSpec {
            name: "Tag_ABI_VFP_args",
            kind: AttrKind::Uleb(&[
                (0, "AAPCS"),
                (1, "VFP registers"),
                (2, "toolchain-specific"),
            ]),
        },
        34 => AttrSpec {
            name: "Tag_CPU_unaligned_access",
            kind: AttrKind::Uleb(&[(0, "No"), (1, "v6")]),
        },
        44 => AttrSpec {
            name: "Tag_DIV_use",
            kind: AttrKind::Uleb(&[
                (0, "Allowed"),
                (1, "Not allowed"),
                (2, "Allowed in v7-R/v7-M"),
            ]),
        },
        _ => return None,
    })
}

fn read_uleb128(bytes: &[u8]) -> (u64, usize) {
    let mut value = 0u64;
    let mut shift = 0u32;
    for (idx, &byte) in bytes.iter().enumerate() {
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return (value, idx + 1);
        }
        shift += 7;
        if shift >= 64 {
            break;
        }
    }
    (0, 0)
}

fn le_u16(bytes: &[u8], off: usize) -> Result<u16, ElfProbeError> {
    let bytes = bytes
        .get(off..off + 2)
        .ok_or(ElfProbeError::Truncated("u16"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn le_u32(bytes: &[u8], off: usize) -> Result<u32, ElfProbeError> {
    let bytes = bytes
        .get(off..off + 4)
        .ok_or(ElfProbeError::Truncated("u32"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn le_u32_lossy(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_file_attributes() {
        let attrs = [
            b'A', 0x3d, 0, 0, 0, b'a', b'e', b'a', b'b', b'i', 0, 1, 0x33, 0, 0, 0, 5, b'A', b'R',
            b'M', b' ', b'v', b'7', 0, 6, 10, 7, 65, 8, 1, 9, 2, 10, 3, 12, 1, 17, 2, 18, 4, 20, 1,
            21, 1, 23, 3, 24, 1, 26, 2, 27, 3, 30, 2, 34, 1, 38, 1, 44, 1, 68, 1,
        ];
        let parsed = parse_arm_attributes(&attrs);
        assert!(
            parsed
                .iter()
                .any(|a| a.name == "Tag_CPU_name" && a.value == "ARM v7")
        );
        assert!(
            parsed
                .iter()
                .any(|a| a.name == "Tag_CPU_arch" && a.value == "v7")
        );
        assert!(
            parsed
                .iter()
                .any(|a| a.name == "Tag_THUMB_ISA_use" && a.value == "Thumb-2")
        );
    }

    #[test]
    fn probes_local_minecraft_pe_so_when_present() {
        let path = Path::new("/tmp/aemu-mcpe-0.15/lib/armeabi-v7a/libminecraftpe.so");
        if !path.exists() {
            return;
        }
        let probe = probe_arm_elf(path).unwrap();
        assert_eq!(probe.machine, "ARM");
        assert_eq!(probe.eabi, "EABI5");
        assert!(probe.requires_armv7_or_newer());
        assert!(probe.requires_thumb2());
        assert!(probe.requires_vfp());
        assert!(probe.requires_neon());
    }

    #[test]
    fn probes_minecraft_pe_so_from_local_apk_when_present() {
        let apk = Path::new("/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk");
        if !apk.exists() {
            return;
        }
        let bytes =
            crate::zip_probe::read_zip_entry(apk, "lib/armeabi-v7a/libminecraftpe.so").unwrap();
        let probe = probe_arm_elf_bytes(
            PathBuf::from("MineCraftPE-a0.15.0.1.apk!/lib/armeabi-v7a/libminecraftpe.so"),
            &bytes,
        )
        .unwrap();
        assert!(probe.requires_armv7_or_newer());
        assert!(probe.requires_thumb2());
        assert!(probe.requires_vfp());
        assert!(probe.requires_neon());
    }
}
