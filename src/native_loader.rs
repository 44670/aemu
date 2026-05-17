use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::apk_plan::ARMV7A_TARGET_ABI;
use crate::elf_dynamic::{
    ElfDynamicError, ElfDynamicInfo, ElfDynamicSymbol, ElfImport, ElfRelocation, SymbolBinding,
    parse_elf_dynamic_bytes,
};
use crate::elf_linker::apply_arm_rel_relocations;
use crate::elf_loader::{ElfLoadError, ElfLoadPlan, load_elf_into_memory, plan_elf_load};
use crate::guest_memory::{MappedMemory, MappedMemoryError};
use crate::hle_imports::{
    HleCallBehavior, HleImportDescriptor, HleSymbolKind, HleSymbolShape, describe_hle_import,
    initialize_hle_symbol, should_link_hle_symbol,
};
use crate::zip_probe::{ZipEntry, ZipProbeError, extract_parsed_zip_entry, parse_zip_entries};

pub const DEFAULT_SHARED_OBJECT_BASE: u32 = 0x7000_0000;
pub const DEFAULT_SHARED_OBJECT_ALIGN: u32 = 0x0010_0000;
pub const DEFAULT_HLE_BASE: u32 = 0x6f00_0000;
const DEFAULT_HLE_PAGE_SIZE: usize = 0x10000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLoadConfig {
    pub abi: String,
    pub first_load_bias: u32,
    pub load_align: u32,
    pub hle_base: u32,
    pub hle_page_size: usize,
}

impl Default for NativeLoadConfig {
    fn default() -> Self {
        Self {
            abi: ARMV7A_TARGET_ABI.to_string(),
            first_load_bias: DEFAULT_SHARED_OBJECT_BASE,
            load_align: DEFAULT_SHARED_OBJECT_ALIGN,
            hle_base: DEFAULT_HLE_BASE,
            hle_page_size: DEFAULT_HLE_PAGE_SIZE,
        }
    }
}

#[derive(Debug)]
pub struct NativeLinkReport {
    pub apk_path: PathBuf,
    pub abi: String,
    pub memory: MappedMemory,
    pub objects: Vec<LoadedNativeObject>,
    pub global_symbols: Vec<NativeSymbol>,
    pub hle_symbols: Vec<HleSymbol>,
    pub resolved_imports: Vec<ResolvedNativeImport>,
    pub unresolved_imports: Vec<UnresolvedNativeImport>,
    pub relocation_errors: Vec<NativeRelocationError>,
}

impl NativeLinkReport {
    pub fn is_linked(&self) -> bool {
        self.unresolved_imports.is_empty() && self.relocation_errors.is_empty()
    }

    pub fn hle_symbol_by_address(&self, address: u32) -> Option<&HleSymbol> {
        self.hle_symbols.iter().find(|symbol| {
            symbol
                .address
                .checked_add(symbol.shape.size())
                .is_some_and(|end| address >= symbol.address && address < end)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedNativeObject {
    pub entry_name: String,
    pub library_name: String,
    pub load_bias: u32,
    pub memory_base: u32,
    pub memory_size: u32,
    pub entry: u32,
    pub needed: Vec<String>,
    pub imports: Vec<ElfImport>,
    pub defined_symbols: Vec<NativeSymbol>,
    pub relocations: Vec<ElfRelocation>,
    pub relocation_count: usize,
    pub init: Option<u32>,
    pub init_array: Option<GuestTableRange>,
    pub arm_exidx: Option<GuestTableRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSymbol {
    pub name: String,
    pub address: u32,
    pub library_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HleSymbol {
    pub name: String,
    pub address: u32,
    pub kind: HleSymbolKind,
    pub shape: HleSymbolShape,
    pub behavior: HleCallBehavior,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNativeImport {
    pub library_name: String,
    pub name: String,
    pub address: u32,
    pub source: NativeSymbolSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeSymbolSource {
    Native { library_name: String },
    Hle { kind: HleSymbolKind },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedNativeImport {
    pub library_name: String,
    pub name: String,
    pub binding: SymbolBinding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeRelocationError {
    pub library_name: String,
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestTableRange {
    pub addr: u32,
    pub size: u32,
}

#[derive(Debug)]
pub enum NativeLoadError {
    Io(String),
    Zip(ZipProbeError),
    MissingAbi {
        requested: String,
        available: Vec<String>,
    },
    ElfLoad {
        library_name: String,
        source: ElfLoadError,
    },
    Dynamic {
        library_name: String,
        source: ElfDynamicError,
    },
    Memory(MappedMemoryError),
    GuestMemory(String),
    Hle(String),
    AddressOverflow,
}

impl fmt::Display for NativeLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Zip(err) => write!(f, "{err}"),
            Self::MissingAbi {
                requested,
                available,
            } => {
                if available.is_empty() {
                    write!(f, "no native libraries found for ABI {requested}")
                } else {
                    write!(
                        f,
                        "no native libraries found for ABI {requested}; available ABIs: {}",
                        available.join(", ")
                    )
                }
            }
            Self::ElfLoad {
                library_name,
                source,
            } => write!(f, "{library_name}: {source}"),
            Self::Dynamic {
                library_name,
                source,
            } => write!(f, "{library_name}: {source}"),
            Self::Memory(err) => write!(f, "{err}"),
            Self::GuestMemory(err) => write!(f, "{err}"),
            Self::Hle(err) => write!(f, "{err}"),
            Self::AddressOverflow => write!(f, "guest address range overflow"),
        }
    }
}

impl std::error::Error for NativeLoadError {}

pub fn load_apk_native_libraries(path: &Path) -> Result<NativeLinkReport, NativeLoadError> {
    load_apk_native_libraries_with_config(path, &NativeLoadConfig::default())
}

pub fn load_apk_native_libraries_with_config(
    path: &Path,
    config: &NativeLoadConfig,
) -> Result<NativeLinkReport, NativeLoadError> {
    let bytes = std::fs::read(path).map_err(|err| NativeLoadError::Io(err.to_string()))?;
    load_apk_native_libraries_bytes(path.to_path_buf(), &bytes, config)
}

pub fn load_apk_native_libraries_bytes(
    apk_path: PathBuf,
    apk_bytes: &[u8],
    config: &NativeLoadConfig,
) -> Result<NativeLinkReport, NativeLoadError> {
    let zip_entries = parse_zip_entries(apk_bytes).map_err(NativeLoadError::Zip)?;
    let mut images = extract_native_images(&apk_path, apk_bytes, &zip_entries, config)?;
    let order = dependency_order(&images);

    let mut memory = MappedMemory::new();
    memory
        .map_zeroed(config.hle_base, config.hle_page_size)
        .map_err(NativeLoadError::Memory)?;

    let mut next_load_bias = config.first_load_bias;
    let mut loaded = Vec::with_capacity(images.len());
    for idx in order {
        let image = &mut images[idx];
        let plan =
            plan_elf_load(image.path.clone(), &image.bytes, next_load_bias).map_err(|source| {
                NativeLoadError::ElfLoad {
                    library_name: image.library_name.clone(),
                    source,
                }
            })?;
        memory
            .map_zeroed(plan.memory_base, plan.memory_size as usize)
            .map_err(NativeLoadError::Memory)?;
        load_elf_into_memory(&image.bytes, &plan, &mut memory).map_err(|source| {
            NativeLoadError::ElfLoad {
                library_name: image.library_name.clone(),
                source,
            }
        })?;

        next_load_bias = next_aligned_load_bias(&plan, config.load_align)?;
        loaded.push(loaded_object(image, &plan));
    }

    let global_symbols = collect_global_symbols(&loaded);
    let hle_symbols = collect_hle_symbols(&loaded, config)?;
    write_hle_symbols(&mut memory, &hle_symbols)?;
    let (resolved_imports, unresolved_imports) =
        resolve_imports(&loaded, &global_symbols, &hle_symbols);
    let relocation_errors = if unresolved_imports.is_empty() {
        apply_relocations(&mut memory, &loaded, &global_symbols, &hle_symbols)
    } else {
        Vec::new()
    };

    Ok(NativeLinkReport {
        apk_path,
        abi: config.abi.clone(),
        memory,
        objects: loaded,
        global_symbols,
        hle_symbols,
        resolved_imports,
        unresolved_imports,
        relocation_errors,
    })
}

struct NativeImage {
    path: PathBuf,
    entry_name: String,
    library_name: String,
    bytes: Vec<u8>,
    dynamic: ElfDynamicInfo,
}

fn extract_native_images(
    apk_path: &Path,
    apk_bytes: &[u8],
    zip_entries: &[ZipEntry],
    config: &NativeLoadConfig,
) -> Result<Vec<NativeImage>, NativeLoadError> {
    let mut available_abis = BTreeSet::new();
    let mut images = Vec::new();

    for entry in zip_entries
        .iter()
        .filter(|entry| entry.name.starts_with("lib/") && entry.name.ends_with(".so"))
    {
        let Some((abi, library_name)) = native_library_parts(&entry.name) else {
            continue;
        };
        available_abis.insert(abi.to_string());
        if abi != config.abi {
            continue;
        }

        let bytes = extract_parsed_zip_entry(apk_bytes, entry).map_err(NativeLoadError::Zip)?;
        let path = PathBuf::from(format!("{}!{}", apk_path.display(), entry.name));
        let dynamic = parse_elf_dynamic_bytes(path.clone(), &bytes).map_err(|source| {
            NativeLoadError::Dynamic {
                library_name: library_name.to_string(),
                source,
            }
        })?;
        images.push(NativeImage {
            path,
            entry_name: entry.name.clone(),
            library_name: library_name.to_string(),
            bytes,
            dynamic,
        });
    }

    if images.is_empty() {
        return Err(NativeLoadError::MissingAbi {
            requested: config.abi.clone(),
            available: available_abis.into_iter().collect(),
        });
    }

    images.sort_by(|a, b| a.library_name.cmp(&b.library_name));
    Ok(images)
}

fn native_library_parts(entry_name: &str) -> Option<(&str, &str)> {
    let rest = entry_name.strip_prefix("lib/")?;
    let (abi, library_name) = rest.split_once('/')?;
    if library_name.is_empty() {
        return None;
    }
    Some((abi, library_name))
}

fn dependency_order(images: &[NativeImage]) -> Vec<usize> {
    let by_name = images
        .iter()
        .enumerate()
        .map(|(idx, image)| (image.library_name.as_str(), idx))
        .collect::<BTreeMap<_, _>>();
    let mut marks = vec![VisitMark::Unvisited; images.len()];
    let mut out = Vec::with_capacity(images.len());
    for idx in 0..images.len() {
        visit_dependency(idx, images, &by_name, &mut marks, &mut out);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VisitMark {
    Unvisited,
    Visiting,
    Visited,
}

fn visit_dependency(
    idx: usize,
    images: &[NativeImage],
    by_name: &BTreeMap<&str, usize>,
    marks: &mut [VisitMark],
    out: &mut Vec<usize>,
) {
    match marks[idx] {
        VisitMark::Visited => return,
        VisitMark::Visiting => return,
        VisitMark::Unvisited => {}
    }
    marks[idx] = VisitMark::Visiting;
    for needed in &images[idx].dynamic.needed {
        if let Some(dep_idx) = by_name.get(needed.as_str()).copied() {
            visit_dependency(dep_idx, images, by_name, marks, out);
        }
    }
    marks[idx] = VisitMark::Visited;
    out.push(idx);
}

fn next_aligned_load_bias(plan: &ElfLoadPlan, load_align: u32) -> Result<u32, NativeLoadError> {
    let end = plan
        .memory_base
        .checked_add(plan.memory_size)
        .ok_or(NativeLoadError::AddressOverflow)?;
    align_up(
        end.checked_add(load_align)
            .ok_or(NativeLoadError::AddressOverflow)?,
        load_align,
    )
    .ok_or(NativeLoadError::AddressOverflow)
}

fn loaded_object(image: &NativeImage, plan: &ElfLoadPlan) -> LoadedNativeObject {
    LoadedNativeObject {
        entry_name: image.entry_name.clone(),
        library_name: image.library_name.clone(),
        load_bias: plan.load_bias,
        memory_base: plan.memory_base,
        memory_size: plan.memory_size,
        entry: plan.entry,
        needed: image.dynamic.needed.clone(),
        imports: image.dynamic.imports.clone(),
        defined_symbols: defined_symbols(
            &image.library_name,
            plan.load_bias,
            &image.dynamic.symbols,
        ),
        relocations: image.dynamic.relocation_entries.clone(),
        relocation_count: image.dynamic.relocation_entries.len(),
        init: image
            .dynamic
            .init
            .map(|addr| plan.load_bias.wrapping_add(addr)),
        init_array: image.dynamic.init_array.map(|range| GuestTableRange {
            addr: plan.load_bias.wrapping_add(range.addr),
            size: range.size,
        }),
        arm_exidx: plan.arm_exidx.map(|range| GuestTableRange {
            addr: range.addr,
            size: range.size,
        }),
    }
}

fn defined_symbols(
    library_name: &str,
    load_bias: u32,
    symbols: &[ElfDynamicSymbol],
) -> Vec<NativeSymbol> {
    let mut out = Vec::new();
    for symbol in symbols {
        if symbol.shndx == 0 {
            continue;
        }
        if symbol.binding == SymbolBinding::Local {
            continue;
        }
        let Some(name) = &symbol.name else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        out.push(NativeSymbol {
            name: name.clone(),
            address: load_bias.wrapping_add(symbol.value),
            library_name: library_name.to_string(),
        });
    }
    out
}

fn collect_global_symbols(objects: &[LoadedNativeObject]) -> Vec<NativeSymbol> {
    let mut by_name = BTreeMap::<String, NativeSymbol>::new();
    for object in objects {
        for symbol in &object.defined_symbols {
            by_name
                .entry(symbol.name.clone())
                .or_insert_with(|| symbol.clone());
        }
    }
    by_name.into_values().collect()
}

fn collect_hle_symbols(
    objects: &[LoadedNativeObject],
    config: &NativeLoadConfig,
) -> Result<Vec<HleSymbol>, NativeLoadError> {
    let mut by_name = BTreeMap::<String, HleImportDescriptor>::new();
    for object in objects {
        for import in &object.imports {
            if should_link_hle_symbol(&import.name)
                && let Some(descriptor) = describe_hle_import(&import.name)
            {
                by_name.entry(import.name.clone()).or_insert(descriptor);
            }
        }
        for symbol in &object.defined_symbols {
            if should_link_hle_symbol(&symbol.name)
                && let Some(descriptor) = describe_hle_import(&symbol.name)
            {
                if descriptor.kind == HleSymbolKind::Target
                    || is_defined_hle_override_symbol(&symbol.name)
                {
                    by_name.entry(symbol.name.clone()).or_insert(descriptor);
                }
            }
        }
    }

    let mut out = Vec::with_capacity(by_name.len());
    let mut byte_off = 0u32;
    for (name, descriptor) in by_name {
        byte_off = align_up(byte_off, 4).ok_or(NativeLoadError::AddressOverflow)?;
        let size = descriptor.shape.size();
        let end = byte_off
            .checked_add(size)
            .ok_or(NativeLoadError::AddressOverflow)?;
        if end as usize > config.hle_page_size {
            return Err(NativeLoadError::AddressOverflow);
        }
        out.push(HleSymbol {
            name,
            address: config
                .hle_base
                .checked_add(byte_off)
                .ok_or(NativeLoadError::AddressOverflow)?,
            kind: descriptor.kind,
            shape: descriptor.shape,
            behavior: descriptor.behavior,
        });
        byte_off = end;
    }
    Ok(out)
}

fn is_defined_hle_override_symbol(name: &str) -> bool {
    matches!(
        name,
        "__divsi3"
            | "__udivsi3"
            | "__modsi3"
            | "__umodsi3"
            | "__divdi3"
            | "__udivdi3"
            | "__moddi3"
            | "__umoddi3"
    )
}

fn write_hle_symbols(
    memory: &mut MappedMemory,
    hle_symbols: &[HleSymbol],
) -> Result<(), NativeLoadError> {
    for symbol in hle_symbols {
        initialize_hle_symbol(
            memory,
            HleImportDescriptor {
                kind: symbol.kind,
                shape: symbol.shape,
                behavior: symbol.behavior,
            },
            symbol.address,
        )
        .map_err(|err| NativeLoadError::Hle(err.to_string()))?;
    }
    Ok(())
}

fn resolve_imports(
    objects: &[LoadedNativeObject],
    global_symbols: &[NativeSymbol],
    hle_symbols: &[HleSymbol],
) -> (Vec<ResolvedNativeImport>, Vec<UnresolvedNativeImport>) {
    let native_by_name = global_symbols
        .iter()
        .map(|symbol| (symbol.name.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    let hle_by_name = hle_symbols
        .iter()
        .map(|symbol| (symbol.name.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();

    let mut seen = BTreeSet::new();
    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();
    for object in objects {
        for import in &object.imports {
            if !seen.insert((object.library_name.clone(), import.name.clone())) {
                continue;
            }
            let native = native_by_name.get(import.name.as_str());
            let hle = hle_by_name.get(import.name.as_str());
            if let Some(symbol) = native
                .filter(|_| hle.is_some())
                .filter(|_| should_prefer_native_symbol_over_hle(&import.name))
            {
                resolved.push(ResolvedNativeImport {
                    library_name: object.library_name.clone(),
                    name: import.name.clone(),
                    address: symbol.address,
                    source: NativeSymbolSource::Native {
                        library_name: symbol.library_name.clone(),
                    },
                });
            } else if let Some(symbol) = hle {
                resolved.push(ResolvedNativeImport {
                    library_name: object.library_name.clone(),
                    name: import.name.clone(),
                    address: symbol.address,
                    source: NativeSymbolSource::Hle { kind: symbol.kind },
                });
            } else if let Some(symbol) = native {
                resolved.push(ResolvedNativeImport {
                    library_name: object.library_name.clone(),
                    name: import.name.clone(),
                    address: symbol.address,
                    source: NativeSymbolSource::Native {
                        library_name: symbol.library_name.clone(),
                    },
                });
            } else {
                unresolved.push(UnresolvedNativeImport {
                    library_name: object.library_name.clone(),
                    name: import.name.clone(),
                    binding: import.binding,
                });
            }
        }
    }
    (resolved, unresolved)
}

fn apply_relocations(
    memory: &mut MappedMemory,
    objects: &[LoadedNativeObject],
    global_symbols: &[NativeSymbol],
    hle_symbols: &[HleSymbol],
) -> Vec<NativeRelocationError> {
    let mut by_name = BTreeMap::<String, u32>::new();
    for symbol in global_symbols {
        by_name.insert(symbol.name.clone(), symbol.address);
    }
    for symbol in hle_symbols {
        if by_name.contains_key(&symbol.name)
            && should_prefer_native_symbol_over_hle(&symbol.name)
            && !hle_symbol_overrides_native(symbol)
        {
            continue;
        }
        by_name.insert(symbol.name.clone(), symbol.address);
    }

    let mut errors = Vec::new();
    for object in objects {
        if object.relocations.is_empty() {
            continue;
        }
        let result = apply_arm_rel_relocations(
            memory,
            object.load_bias,
            &object.relocations,
            &|name: &str| by_name.get(name).copied(),
        );
        if let Err(err) = result {
            errors.push(NativeRelocationError {
                library_name: object.library_name.clone(),
                error: err.to_string(),
            });
        }
    }
    errors
}

fn hle_symbol_overrides_native(symbol: &HleSymbol) -> bool {
    symbol.kind == HleSymbolKind::Target || is_defined_hle_override_symbol(&symbol.name)
}

fn should_prefer_native_symbol_over_hle(name: &str) -> bool {
    matches!(
        name,
        "_Unwind_DeleteException"
            | "_Unwind_GetLanguageSpecificData"
            | "_Unwind_GetRegionStart"
            | "_Unwind_Resume"
            | "__cxa_allocate_exception"
            | "__cxa_bad_cast"
            | "__cxa_begin_catch"
            | "__cxa_begin_cleanup"
            | "__cxa_call_unexpected"
            | "__cxa_end_catch"
            | "__cxa_free_exception"
            | "__cxa_pure_virtual"
            | "__cxa_rethrow"
            | "__cxa_throw"
            | "__cxa_type_match"
            | "__gxx_personality_v0"
    )
}

fn align_up(value: u32, align: u32) -> Option<u32> {
    if align == 0 || !align.is_power_of_two() {
        return None;
    }
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::armv7a::Memory;

    use super::*;

    #[test]
    fn loads_dependencies_with_one_to_one_guest_addresses_and_applies_relocations() {
        let support = test_so(&[], &[], &[("support_func", 0x1200)], &[]);
        let game = test_so(
            &["libsupport.so"],
            &["support_func", "glCreateShader"],
            &[],
            &[
                TestRelocation {
                    offset: 0x700,
                    symbol: Some("support_func"),
                    kind: 21,
                    addend: 0,
                },
                TestRelocation {
                    offset: 0x704,
                    symbol: Some("glCreateShader"),
                    kind: 22,
                    addend: 0,
                },
                TestRelocation {
                    offset: 0x708,
                    symbol: None,
                    kind: 23,
                    addend: 0x44,
                },
            ],
        );
        let apk = zip_with_files(&[
            ("lib/armeabi-v7a/libgame.so", game),
            ("lib/armeabi-v7a/libsupport.so", support),
        ]);

        let mut report = load_apk_native_libraries_bytes(
            PathBuf::from("game.apk"),
            &apk,
            &NativeLoadConfig::default(),
        )
        .unwrap();

        assert!(report.is_linked());
        assert_eq!(report.objects[0].library_name, "libsupport.so");
        assert_eq!(report.objects[0].load_bias, DEFAULT_SHARED_OBJECT_BASE);
        assert_eq!(report.objects[0].memory_base, DEFAULT_SHARED_OBJECT_BASE);
        assert_eq!(report.objects[1].library_name, "libgame.so");

        let support_addr = report
            .global_symbols
            .iter()
            .find(|symbol| symbol.name == "support_func")
            .unwrap()
            .address;
        assert_eq!(support_addr, report.objects[0].load_bias + 0x1200);

        let gl_addr = report
            .hle_symbols
            .iter()
            .find(|symbol| symbol.name == "glCreateShader")
            .unwrap()
            .address;
        assert_eq!(gl_addr, DEFAULT_HLE_BASE);
        assert_eq!(report.memory.load32(gl_addr).unwrap(), 0xe7f0_00f0);
        assert_eq!(
            report.hle_symbol_by_address(gl_addr).unwrap().name,
            "glCreateShader"
        );
        assert!(report.unresolved_imports.is_empty());

        let game_bias = report.objects[1].load_bias;
        assert_eq!(
            report.memory.load32(game_bias + 0x700).unwrap(),
            support_addr
        );
        assert_eq!(report.memory.load32(game_bias + 0x704).unwrap(), gl_addr);
        assert_eq!(
            report.memory.load32(game_bias + 0x708).unwrap(),
            game_bias + 0x44
        );
    }

    #[test]
    fn hle_symbols_override_native_exports_for_runtime_helpers_but_not_game_logic() {
        let cxx_name = "_ZNSs14_M_replace_auxEjjjc";
        let target_name = "_ZN8WebTokenC2ERKS_";
        let support = test_so(&[], &[], &[(cxx_name, 0x1200)], &[]);
        let game = test_so(
            &["libsupport.so"],
            &[cxx_name],
            &[(target_name, 0x1400)],
            &[
                TestRelocation {
                    offset: 0x700,
                    symbol: Some(cxx_name),
                    kind: 21,
                    addend: 0,
                },
                TestRelocation {
                    offset: 0x704,
                    symbol: Some(target_name),
                    kind: 21,
                    addend: 0,
                },
            ],
        );
        let apk = zip_with_files(&[
            ("lib/armeabi-v7a/libgame.so", game),
            ("lib/armeabi-v7a/libsupport.so", support),
        ]);

        let mut report = load_apk_native_libraries_bytes(
            PathBuf::from("game.apk"),
            &apk,
            &NativeLoadConfig::default(),
        )
        .unwrap();

        let native_addr = report
            .global_symbols
            .iter()
            .find(|symbol| symbol.name == cxx_name)
            .unwrap()
            .address;
        let hle_addr = report
            .hle_symbols
            .iter()
            .find(|symbol| symbol.name == cxx_name)
            .unwrap()
            .address;
        assert_ne!(hle_addr, native_addr);
        let native_target_addr = report
            .global_symbols
            .iter()
            .find(|symbol| symbol.name == target_name)
            .unwrap()
            .address;
        assert!(
            report
                .hle_symbols
                .iter()
                .all(|symbol| symbol.name != target_name)
        );

        let game_bias = report
            .objects
            .iter()
            .find(|object| object.library_name == "libgame.so")
            .unwrap()
            .load_bias;
        assert_eq!(report.memory.load32(game_bias + 0x700).unwrap(), hle_addr);
        assert_eq!(
            report.memory.load32(game_bias + 0x704).unwrap(),
            native_target_addr
        );
        assert!(report.resolved_imports.iter().any(|import| {
            import.name == cxx_name
                && matches!(
                    import.source,
                    NativeSymbolSource::Hle {
                        kind: HleSymbolKind::CxxStd
                    }
                )
        }));
    }

    #[test]
    fn hle_symbols_override_defined_compiler_integer_helpers() {
        for helper_name in ["__umodsi3", "__umoddi3"] {
            let game = test_so(
                &[],
                &[],
                &[(helper_name, 0x1200)],
                &[TestRelocation {
                    offset: 0x700,
                    symbol: Some(helper_name),
                    kind: 21,
                    addend: 0,
                }],
            );
            let apk = zip_with_files(&[("lib/armeabi-v7a/libgame.so", game)]);

            let mut report = load_apk_native_libraries_bytes(
                PathBuf::from("game.apk"),
                &apk,
                &NativeLoadConfig::default(),
            )
            .unwrap();

            let native_addr = report
                .global_symbols
                .iter()
                .find(|symbol| symbol.name == helper_name)
                .unwrap()
                .address;
            let hle_addr = report
                .hle_symbols
                .iter()
                .find(|symbol| symbol.name == helper_name)
                .unwrap()
                .address;
            assert_ne!(hle_addr, native_addr);

            let game_bias = report
                .objects
                .iter()
                .find(|object| object.library_name == "libgame.so")
                .unwrap()
                .load_bias;
            assert_eq!(report.memory.load32(game_bias + 0x700).unwrap(), hle_addr);
        }
    }

    #[test]
    fn minecraft_game_logic_exports_remain_native_by_default() {
        let game_logic_names = [
            "_ZN3mce12TextureGroup14getTexturePairERK16ResourceLocation",
            "_ZN4Font4initEv",
            "_ZN11AppPlatform7loadPNGER11TextureDataRKSs",
            "_ZN18MinecraftTelemetry4tickEv",
        ];
        let exports = game_logic_names
            .iter()
            .enumerate()
            .map(|(idx, name)| (*name, 0x1200 + idx as u32 * 4))
            .collect::<Vec<_>>();
        let relocations = game_logic_names
            .iter()
            .enumerate()
            .map(|(idx, name)| TestRelocation {
                offset: 0x700 + idx as u32 * 4,
                symbol: Some(*name),
                kind: 21,
                addend: 0,
            })
            .collect::<Vec<_>>();
        let game = test_so(&[], &[], &exports, &relocations);
        let apk = zip_with_files(&[("lib/armeabi-v7a/libgame.so", game)]);

        let mut report = load_apk_native_libraries_bytes(
            PathBuf::from("game.apk"),
            &apk,
            &NativeLoadConfig::default(),
        )
        .unwrap();

        let game_bias = report.objects[0].load_bias;
        for (idx, name) in game_logic_names.iter().enumerate() {
            assert!(report.hle_symbols.iter().all(|symbol| symbol.name != *name));
            let native_addr = report
                .global_symbols
                .iter()
                .find(|symbol| symbol.name == *name)
                .unwrap()
                .address;
            assert_eq!(
                report
                    .memory
                    .load32(game_bias + 0x700 + idx as u32 * 4)
                    .unwrap(),
                native_addr
            );
        }
    }

    #[test]
    fn reports_requested_abi_when_apk_has_only_other_native_abi() {
        let apk = zip_with_files(&[(
            "lib/x86/libgame.so",
            test_so(&[], &[], &[("game_func", 0x1000)], &[]),
        )]);
        let err = load_apk_native_libraries_bytes(
            PathBuf::from("game.apk"),
            &apk,
            &NativeLoadConfig::default(),
        )
        .unwrap_err();

        match err {
            NativeLoadError::MissingAbi {
                requested,
                available,
            } => {
                assert_eq!(requested, "armeabi-v7a");
                assert_eq!(available, vec!["x86"]);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn local_minecraft_link_has_no_game_logic_hle_symbols_when_present() {
        let apk = Path::new("/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk");
        if !apk.exists() {
            return;
        }
        let config = NativeLoadConfig {
            abi: "armeabi-v7a".to_string(),
            ..NativeLoadConfig::default()
        };
        let report = load_apk_native_libraries_with_config(apk, &config).unwrap();

        assert!(
            report
                .hle_symbols
                .iter()
                .all(|symbol| symbol.kind != HleSymbolKind::Target)
        );
    }

    struct TestRelocation {
        offset: u32,
        symbol: Option<&'static str>,
        kind: u8,
        addend: u32,
    }

    fn test_so(
        needed: &[&str],
        imports: &[&str],
        exports: &[(&str, u32)],
        relocations: &[TestRelocation],
    ) -> Vec<u8> {
        let file_size = 0x800usize;
        let dyn_off = 0x100usize;
        let dynstr_off = 0x240usize;
        let dynsym_off = 0x300usize;
        let rel_off = 0x400usize;
        let shoff = 0x500usize;

        let mut dynstr = Vec::new();
        dynstr.push(0);
        let mut needed_offsets = Vec::new();
        for name in needed {
            needed_offsets.push(push_str(&mut dynstr, name));
        }

        let mut symbol_offsets = BTreeMap::new();
        for (name, _) in exports {
            symbol_offsets.insert(*name, push_str(&mut dynstr, name));
        }
        for name in imports {
            symbol_offsets
                .entry(*name)
                .or_insert_with(|| push_str(&mut dynstr, name));
        }

        let mut dynsym = Vec::new();
        dynsym.resize(16, 0);
        let mut symbol_indices = BTreeMap::new();
        for (name, value) in exports {
            symbol_indices.insert(*name, (dynsym.len() / 16) as u32);
            push_dynsym(&mut dynsym, symbol_offsets[name], *value, 0x12, 1);
        }
        for name in imports {
            symbol_indices.insert(*name, (dynsym.len() / 16) as u32);
            push_dynsym(&mut dynsym, symbol_offsets[name], 0, 0x12, 0);
        }

        let mut bytes = vec![0; file_size];
        write_elf_header(&mut bytes, 52, 2, shoff as u32, 3);
        write_phdr(&mut bytes, 52, 1, 0, 0, file_size as u32, 0x2000);
        write_phdr(
            &mut bytes,
            84,
            2,
            dyn_off as u32,
            dyn_off as u32,
            16 * 8,
            16 * 8,
        );

        let mut dyn_entries = Vec::new();
        for needed_off in needed_offsets {
            push_dynamic(&mut dyn_entries, 1, needed_off);
        }
        push_dynamic(&mut dyn_entries, 5, dynstr_off as u32);
        push_dynamic(&mut dyn_entries, 10, dynstr.len() as u32);
        push_dynamic(&mut dyn_entries, 6, dynsym_off as u32);
        push_dynamic(&mut dyn_entries, 11, 16);
        if !relocations.is_empty() {
            push_dynamic(&mut dyn_entries, 17, rel_off as u32);
            push_dynamic(&mut dyn_entries, 18, (relocations.len() * 8) as u32);
            push_dynamic(&mut dyn_entries, 19, 8);
        }
        push_dynamic(&mut dyn_entries, 12, 0x128);
        push_dynamic(&mut dyn_entries, 25, 0x710);
        push_dynamic(&mut dyn_entries, 27, 4);
        push_dynamic(&mut dyn_entries, 0, 0);

        bytes[dyn_off..dyn_off + dyn_entries.len()].copy_from_slice(&dyn_entries);
        bytes[dynstr_off..dynstr_off + dynstr.len()].copy_from_slice(&dynstr);
        bytes[dynsym_off..dynsym_off + dynsym.len()].copy_from_slice(&dynsym);
        for (idx, relocation) in relocations.iter().enumerate() {
            let rel_entry_off = rel_off + idx * 8;
            let symbol_index = relocation
                .symbol
                .and_then(|name| symbol_indices.get(name).copied())
                .unwrap_or(0);
            write_u32(&mut bytes, rel_entry_off, relocation.offset);
            write_u32(
                &mut bytes,
                rel_entry_off + 4,
                (symbol_index << 8) | u32::from(relocation.kind),
            );
            write_u32(&mut bytes, relocation.offset as usize, relocation.addend);
        }

        write_shdr(
            &mut bytes,
            shoff + 40,
            11,
            dynsym_off as u32,
            dynsym.len() as u32,
            2,
            16,
        );
        write_shdr(
            &mut bytes,
            shoff + 80,
            3,
            dynstr_off as u32,
            dynstr.len() as u32,
            0,
            0,
        );
        bytes
    }

    fn zip_with_files(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
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
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(data);

            push_u32(&mut central, 0x0201_4b50);
            push_u16(&mut central, 20);
            push_u16(&mut central, 20);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u32(&mut central, 0);
            push_u32(&mut central, data.len() as u32);
            push_u32(&mut central, data.len() as u32);
            push_u16(&mut central, name.len() as u16);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u16(&mut central, 0);
            push_u32(&mut central, 0);
            push_u32(&mut central, local_offset);
            central.extend_from_slice(name.as_bytes());
        }

        let central_offset = bytes.len() as u32;
        let central_size = central.len() as u32;
        bytes.extend_from_slice(&central);
        push_u32(&mut bytes, 0x0605_4b50);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, files.len() as u16);
        push_u16(&mut bytes, files.len() as u16);
        push_u32(&mut bytes, central_size);
        push_u32(&mut bytes, central_offset);
        push_u16(&mut bytes, 0);
        bytes
    }

    fn write_elf_header(bytes: &mut [u8], phoff: u32, phnum: u16, shoff: u32, shnum: u16) {
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 1;
        bytes[5] = 1;
        bytes[6] = 1;
        write_u16(bytes, 16, 3);
        write_u16(bytes, 18, 40);
        write_u32(bytes, 20, 1);
        write_u32(bytes, 24, 0);
        write_u32(bytes, 28, phoff);
        write_u32(bytes, 32, shoff);
        write_u32(bytes, 36, 0x0500_0000);
        write_u16(bytes, 40, 52);
        write_u16(bytes, 42, 32);
        write_u16(bytes, 44, phnum);
        write_u16(bytes, 46, 40);
        write_u16(bytes, 48, shnum);
    }

    fn write_phdr(
        bytes: &mut [u8],
        off: usize,
        p_type: u32,
        p_offset: u32,
        p_vaddr: u32,
        p_filesz: u32,
        p_memsz: u32,
    ) {
        write_u32(bytes, off, p_type);
        write_u32(bytes, off + 4, p_offset);
        write_u32(bytes, off + 8, p_vaddr);
        write_u32(bytes, off + 12, p_vaddr);
        write_u32(bytes, off + 16, p_filesz);
        write_u32(bytes, off + 20, p_memsz);
        write_u32(bytes, off + 24, 5);
        write_u32(bytes, off + 28, 0x1000);
    }

    fn write_shdr(
        bytes: &mut [u8],
        off: usize,
        sh_type: u32,
        sh_offset: u32,
        sh_size: u32,
        sh_link: u32,
        sh_entsize: u32,
    ) {
        write_u32(bytes, off + 4, sh_type);
        write_u32(bytes, off + 16, sh_offset);
        write_u32(bytes, off + 20, sh_size);
        write_u32(bytes, off + 24, sh_link);
        write_u32(bytes, off + 36, sh_entsize);
    }

    fn push_dynsym(out: &mut Vec<u8>, name: u32, value: u32, info: u8, shndx: u16) {
        push_u32(out, name);
        push_u32(out, value);
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
        let off = out.len() as u32;
        out.extend_from_slice(value.as_bytes());
        out.push(0);
        off
    }

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u16(bytes: &mut [u8], off: usize, value: u16) {
        bytes[off..off + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(bytes: &mut [u8], off: usize, value: u32) {
        bytes[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
}
