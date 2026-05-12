use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::elf_probe::{ElfProbe, probe_arm_elf_bytes};
use crate::zip_probe::{
    ZipCompression, ZipProbeError, extract_parsed_zip_entry, parse_zip_entries,
};

pub const ARMV6_TARGET_ABI: &str = "armeabi";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApkRunPlan {
    pub path: PathBuf,
    pub native_libraries: Vec<NativeLibraryPlan>,
}

impl ApkRunPlan {
    pub fn abi_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for library in &self.native_libraries {
            *counts.entry(library.abi.clone()).or_insert(0) += 1;
        }
        counts
    }

    pub fn has_target_abi(&self) -> bool {
        self.native_libraries
            .iter()
            .any(|library| library.abi == ARMV6_TARGET_ABI)
    }

    pub fn target_libraries(&self) -> impl Iterator<Item = &NativeLibraryPlan> {
        self.native_libraries
            .iter()
            .filter(|library| library.abi == ARMV6_TARGET_ABI)
    }

    pub fn is_armv6_runnable(&self) -> bool {
        self.has_target_abi()
            && self
                .target_libraries()
                .all(|library| library.armv6_blockers.is_empty())
    }

    pub fn summary_blockers(&self) -> Vec<String> {
        let mut blockers = Vec::new();
        if self.native_libraries.is_empty() {
            blockers.push(
                "no native libraries found; DEX-only APK execution is not implemented".into(),
            );
            return blockers;
        }
        if !self.has_target_abi() {
            blockers.push("missing lib/armeabi/*.so for the ARMv6 interpreter target".into());
        }
        for library in &self.native_libraries {
            if library.armv6_blockers.is_empty() {
                continue;
            }
            let reasons = library
                .armv6_blockers
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            blockers.push(format!("{}: {reasons}", library.entry_name));
        }
        blockers
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLibraryPlan {
    pub entry_name: String,
    pub abi: String,
    pub library_name: String,
    pub compression: ZipCompression,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub probe: Option<ElfProbe>,
    pub probe_error: Option<String>,
    pub armv6_blockers: Vec<Armv6Blocker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Armv6Blocker {
    NotTargetAbi(String),
    ProbeFailed(String),
    NotArmElf(String),
    RequiresArmv7OrNewer,
    RequiresThumb2,
    RequiresVfp3OrNewer(String),
    RequiresNeon,
}

impl fmt::Display for Armv6Blocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTargetAbi(abi) => write!(f, "ABI {abi} is not {ARMV6_TARGET_ABI}"),
            Self::ProbeFailed(err) => write!(f, "ELF probe failed: {err}"),
            Self::NotArmElf(machine) => write!(f, "ELF machine is {machine}, not ARM"),
            Self::RequiresArmv7OrNewer => write!(f, "requires ARMv7 or newer"),
            Self::RequiresThumb2 => write!(f, "requires Thumb-2"),
            Self::RequiresVfp3OrNewer(fp_arch) => write!(f, "requires {fp_arch} beyond VFPv2"),
            Self::RequiresNeon => write!(f, "requires NEON"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApkPlanError {
    Io(String),
    Zip(ZipProbeError),
}

impl fmt::Display for ApkPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Zip(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ApkPlanError {}

pub fn analyze_apk(path: &Path) -> Result<ApkRunPlan, ApkPlanError> {
    let bytes = fs::read(path).map_err(|err| ApkPlanError::Io(err.to_string()))?;
    analyze_apk_bytes(path.to_path_buf(), &bytes)
}

pub fn analyze_apk_bytes(path: PathBuf, bytes: &[u8]) -> Result<ApkRunPlan, ApkPlanError> {
    let entries = parse_zip_entries(bytes).map_err(ApkPlanError::Zip)?;
    let mut native_libraries = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.name.starts_with("lib/") && entry.name.ends_with(".so"))
    {
        let abi = native_library_abi(&entry.name).unwrap_or("").to_string();
        let library_name = native_library_name(&entry.name).unwrap_or("").to_string();
        let extracted = extract_parsed_zip_entry(bytes, entry).map_err(ApkPlanError::Zip)?;
        let probe_result = probe_arm_elf_bytes(
            PathBuf::from(format!("{}!{}", path.display(), entry.name)),
            &extracted,
        );
        let (probe, probe_error) = match probe_result {
            Ok(probe) => (Some(probe), None),
            Err(err) => (None, Some(err.to_string())),
        };
        let armv6_blockers = classify_armv6_library(&abi, probe.as_ref(), probe_error.as_deref());
        native_libraries.push(NativeLibraryPlan {
            entry_name: entry.name.clone(),
            abi,
            library_name,
            compression: entry.compression,
            compressed_size: entry.compressed_size,
            uncompressed_size: entry.uncompressed_size,
            probe,
            probe_error,
            armv6_blockers,
        });
    }

    native_libraries.sort_by(|a, b| a.entry_name.cmp(&b.entry_name));
    Ok(ApkRunPlan {
        path,
        native_libraries,
    })
}

fn classify_armv6_library(
    abi: &str,
    probe: Option<&ElfProbe>,
    probe_error: Option<&str>,
) -> Vec<Armv6Blocker> {
    let mut blockers = Vec::new();
    if abi != ARMV6_TARGET_ABI {
        blockers.push(Armv6Blocker::NotTargetAbi(abi.to_string()));
    }

    let Some(probe) = probe else {
        blockers.push(Armv6Blocker::ProbeFailed(
            probe_error.unwrap_or("unknown error").to_string(),
        ));
        return blockers;
    };

    if probe.machine != "ARM" {
        blockers.push(Armv6Blocker::NotArmElf(probe.machine.clone()));
    }
    if probe.requires_armv7_or_newer() {
        blockers.push(Armv6Blocker::RequiresArmv7OrNewer);
    }
    if probe.requires_thumb2() {
        blockers.push(Armv6Blocker::RequiresThumb2);
    }
    if probe.requires_vfp3_or_newer() {
        blockers.push(Armv6Blocker::RequiresVfp3OrNewer(
            probe.fp_arch().unwrap_or("VFPv3+").to_string(),
        ));
    }
    if probe.requires_neon() {
        blockers.push(Armv6Blocker::RequiresNeon);
    }
    blockers
}

fn native_library_abi(entry_name: &str) -> Option<&str> {
    let rest = entry_name.strip_prefix("lib/")?;
    rest.split_once('/').map(|(abi, _)| abi)
}

fn native_library_name(entry_name: &str) -> Option<&str> {
    let rest = entry_name.strip_prefix("lib/")?;
    rest.split_once('/').map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_armv7_neon_library_as_blocked_for_armv6() {
        let probe = ElfProbe {
            path: PathBuf::from("lib/armeabi-v7a/libminecraftpe.so"),
            machine: "ARM".to_string(),
            eabi: "EABI5".to_string(),
            attributes: vec![
                attr("Tag_CPU_arch", "v7"),
                attr("Tag_THUMB_ISA_use", "Thumb-2"),
                attr("Tag_FP_arch", "VFPv3"),
                attr("Tag_Advanced_SIMD_arch", "NEONv1"),
            ],
        };

        let blockers = classify_armv6_library("armeabi-v7a", Some(&probe), None);
        assert_eq!(
            blockers,
            vec![
                Armv6Blocker::NotTargetAbi("armeabi-v7a".to_string()),
                Armv6Blocker::RequiresArmv7OrNewer,
                Armv6Blocker::RequiresThumb2,
                Armv6Blocker::RequiresVfp3OrNewer("VFPv3".to_string()),
                Armv6Blocker::RequiresNeon,
            ]
        );
    }

    #[test]
    fn accepts_armv6_armeabi_vfpv2_library() {
        let apk = zip_with_one_file(
            "lib/armeabi/libgame.so",
            minimal_arm_elf(arm_attrs("ARM v6", 6, 1, 2, 0)),
        );
        let plan = analyze_apk_bytes(PathBuf::from("game.apk"), &apk).unwrap();

        assert_eq!(plan.native_libraries.len(), 1);
        assert!(plan.has_target_abi());
        assert!(plan.is_armv6_runnable());
        assert!(plan.summary_blockers().is_empty());
    }

    #[test]
    fn rejects_apk_without_armeabi_libraries() {
        let apk = zip_with_one_file(
            "lib/armeabi-v7a/libminecraftpe.so",
            minimal_arm_elf(arm_attrs("ARM v7", 10, 2, 3, 1)),
        );
        let plan = analyze_apk_bytes(PathBuf::from("mcpe.apk"), &apk).unwrap();

        assert!(!plan.has_target_abi());
        assert!(!plan.is_armv6_runnable());
        let blockers = plan.summary_blockers();
        assert!(
            blockers
                .iter()
                .any(|blocker| blocker.contains("missing lib/armeabi/*.so"))
        );
        assert!(
            blockers
                .iter()
                .any(|blocker| blocker.contains("requires Thumb-2"))
        );
    }

    #[test]
    fn plans_local_minecraft_pe_apk_when_present() {
        let apk = Path::new("/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk");
        if !apk.exists() {
            return;
        }
        let plan = analyze_apk(apk).unwrap();
        assert!(!plan.has_target_abi());
        assert!(!plan.is_armv6_runnable());
        assert!(
            plan.summary_blockers()
                .iter()
                .any(|blocker| blocker.contains("missing lib/armeabi/*.so"))
        );
        assert!(
            plan.native_libraries
                .iter()
                .any(
                    |library| library.entry_name == "lib/armeabi-v7a/libminecraftpe.so"
                        && library.armv6_blockers.contains(&Armv6Blocker::RequiresNeon)
                )
        );
    }

    fn attr(name: &str, value: &str) -> crate::elf_probe::ArmAttribute {
        crate::elf_probe::ArmAttribute {
            tag: 0,
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    fn arm_attrs(cpu_name: &str, cpu_arch: u8, thumb: u8, fp_arch: u8, simd: u8) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(5);
        payload.extend_from_slice(cpu_name.as_bytes());
        payload.push(0);
        payload.extend_from_slice(&[6, cpu_arch, 8, 1, 9, thumb, 10, fp_arch, 12, simd]);

        let file_len = 1 + 4 + payload.len();
        let mut subsection_payload = Vec::new();
        subsection_payload.extend_from_slice(b"aeabi\0");
        subsection_payload.push(1);
        push_u32(&mut subsection_payload, file_len as u32);
        subsection_payload.extend_from_slice(&payload);

        let mut attrs = Vec::new();
        attrs.push(b'A');
        push_u32(&mut attrs, (4 + subsection_payload.len()) as u32);
        attrs.extend_from_slice(&subsection_payload);
        attrs
    }

    fn minimal_arm_elf(attrs: Vec<u8>) -> Vec<u8> {
        let mut bytes = vec![0; 52];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 1;
        bytes[5] = 1;
        bytes[6] = 1;
        write_u16(&mut bytes, 16, 3);
        write_u16(&mut bytes, 18, 40);
        write_u32(&mut bytes, 20, 1);
        write_u32(&mut bytes, 36, 0x0500_0000);
        write_u16(&mut bytes, 46, 40);
        write_u16(&mut bytes, 48, 3);
        write_u16(&mut bytes, 50, 2);

        let attr_off = bytes.len();
        bytes.extend_from_slice(&attrs);
        let shstr_off = bytes.len();
        bytes.extend_from_slice(b"\0.ARM.attributes\0.shstrtab\0");
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }

        let shoff = bytes.len();
        bytes.resize(shoff + 3 * 40, 0);
        write_u32(&mut bytes, 32, shoff as u32);

        write_u32(&mut bytes, shoff + 40, 1);
        write_u32(&mut bytes, shoff + 40 + 16, attr_off as u32);
        write_u32(&mut bytes, shoff + 40 + 20, attrs.len() as u32);

        write_u32(&mut bytes, shoff + 80, 17);
        write_u32(&mut bytes, shoff + 80 + 16, shstr_off as u32);
        write_u32(&mut bytes, shoff + 80 + 20, 27);

        bytes
    }

    fn zip_with_one_file(name: &str, data: Vec<u8>) -> Vec<u8> {
        let name = name.as_bytes();
        let mut bytes = Vec::new();
        let local_offset = bytes.len() as u32;
        push_u32(&mut bytes, 0x0403_4b50);
        push_u16(&mut bytes, 20);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, data.len() as u32);
        push_u32(&mut bytes, data.len() as u32);
        push_u16(&mut bytes, name.len() as u16);
        push_u16(&mut bytes, 0);
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&data);

        let central_offset = bytes.len() as u32;
        push_u32(&mut bytes, 0x0201_4b50);
        push_u16(&mut bytes, 20);
        push_u16(&mut bytes, 20);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, data.len() as u32);
        push_u32(&mut bytes, data.len() as u32);
        push_u16(&mut bytes, name.len() as u16);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, local_offset);
        bytes.extend_from_slice(name);

        let central_size = bytes.len() as u32 - central_offset;
        push_u32(&mut bytes, 0x0605_4b50);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 1);
        push_u32(&mut bytes, central_size);
        push_u32(&mut bytes, central_offset);
        push_u16(&mut bytes, 0);
        bytes
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
