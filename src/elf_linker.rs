use std::fmt;

use crate::armv6::Memory;
use crate::elf_dynamic::{ArmRelocationKind, ElfRelocation};

pub trait SymbolResolver {
    fn resolve(&self, name: &str) -> Option<u32>;
}

impl<F> SymbolResolver for F
where
    F: for<'a> Fn(&'a str) -> Option<u32>,
{
    fn resolve(&self, name: &str) -> Option<u32> {
        self(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfLinkError {
    AddressOverflow { offset: u32 },
    MissingSymbolName { symbol_index: u32 },
    UnresolvedSymbol(String),
    UnsupportedRelocation(ArmRelocationKind),
    Memory(String),
}

impl fmt::Display for ElfLinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AddressOverflow { offset } => {
                write!(f, "relocation address overflow for offset {offset:#010x}")
            }
            Self::MissingSymbolName { symbol_index } => {
                write!(
                    f,
                    "relocation references unnamed symbol index {symbol_index}"
                )
            }
            Self::UnresolvedSymbol(name) => write!(f, "unresolved symbol: {name}"),
            Self::UnsupportedRelocation(kind) => write!(f, "unsupported relocation: {kind}"),
            Self::Memory(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ElfLinkError {}

pub fn apply_arm_rel_relocations<M, R>(
    memory: &mut M,
    load_bias: u32,
    relocations: &[ElfRelocation],
    resolver: &R,
) -> Result<(), ElfLinkError>
where
    M: Memory,
    R: SymbolResolver,
{
    for relocation in relocations {
        let place =
            load_bias
                .checked_add(relocation.offset)
                .ok_or(ElfLinkError::AddressOverflow {
                    offset: relocation.offset,
                })?;
        apply_one(memory, place, load_bias, relocation, resolver)?;
    }
    Ok(())
}

fn apply_one<M, R>(
    memory: &mut M,
    place: u32,
    load_bias: u32,
    relocation: &ElfRelocation,
    resolver: &R,
) -> Result<(), ElfLinkError>
where
    M: Memory,
    R: SymbolResolver,
{
    match relocation.kind {
        ArmRelocationKind::Relative => {
            let addend = load32(memory, place)?;
            store32(memory, place, load_bias.wrapping_add(addend))
        }
        ArmRelocationKind::Abs32 | ArmRelocationKind::Target1 => {
            let symbol = resolve_relocation_symbol(relocation, resolver)?;
            let addend = load32(memory, place)?;
            store32(memory, place, symbol.wrapping_add(addend))
        }
        ArmRelocationKind::Rel32 => {
            let symbol = resolve_relocation_symbol(relocation, resolver)?;
            let addend = load32(memory, place)?;
            store32(
                memory,
                place,
                symbol.wrapping_add(addend).wrapping_sub(place),
            )
        }
        ArmRelocationKind::GlobDat | ArmRelocationKind::JumpSlot => {
            let symbol = resolve_relocation_symbol(relocation, resolver)?;
            store32(memory, place, symbol)
        }
        ArmRelocationKind::V4Bx => Ok(()),
        ArmRelocationKind::Copy | ArmRelocationKind::Other(_) => {
            Err(ElfLinkError::UnsupportedRelocation(relocation.kind))
        }
    }
}

fn resolve_relocation_symbol<R>(
    relocation: &ElfRelocation,
    resolver: &R,
) -> Result<u32, ElfLinkError>
where
    R: SymbolResolver,
{
    let name = relocation
        .symbol_name
        .as_deref()
        .ok_or(ElfLinkError::MissingSymbolName {
            symbol_index: relocation.symbol_index,
        })?;
    resolver
        .resolve(name)
        .ok_or_else(|| ElfLinkError::UnresolvedSymbol(name.to_string()))
}

fn load32<M: Memory>(memory: &mut M, addr: u32) -> Result<u32, ElfLinkError> {
    memory
        .load32(addr)
        .map_err(|err| ElfLinkError::Memory(err.to_string()))
}

fn store32<M: Memory>(memory: &mut M, addr: u32, value: u32) -> Result<(), ElfLinkError> {
    memory
        .store32(addr, value)
        .map_err(|err| ElfLinkError::Memory(err.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::armv6::{Memory, VecMemory};
    use crate::elf_dynamic::{ArmRelocationKind, ElfRelocation, RelocationTable};

    use super::*;

    #[test]
    fn applies_core_arm_rel_relocations() {
        let load_bias = 0x7000_0000;
        let mut memory = VecMemory::new(0x7000_1000, 0x100);
        memory.store32(0x7000_1000, 0x20).unwrap();
        memory.store32(0x7000_1004, 4).unwrap();
        memory.store32(0x7000_1008, 0xdead_beef).unwrap();
        memory.store32(0x7000_100c, 4).unwrap();

        let relocations = vec![
            relocation(0x1000, ArmRelocationKind::Relative, 0, None),
            relocation(0x1004, ArmRelocationKind::Abs32, 1, Some("abs")),
            relocation(0x1008, ArmRelocationKind::GlobDat, 2, Some("glob")),
            relocation(0x100c, ArmRelocationKind::Rel32, 3, Some("rel")),
        ];
        apply_arm_rel_relocations(&mut memory, load_bias, &relocations, &test_resolver).unwrap();

        assert_eq!(memory.load32(0x7000_1000).unwrap(), 0x7000_0020);
        assert_eq!(memory.load32(0x7000_1004).unwrap(), 0x7100_0004);
        assert_eq!(memory.load32(0x7000_1008).unwrap(), 0x7200_0000);
        assert_eq!(memory.load32(0x7000_100c).unwrap(), 0x0000_0ff8);
    }

    #[test]
    fn reports_unresolved_relocation_symbol() {
        let mut memory = VecMemory::new(0x1000, 0x10);
        let err = apply_arm_rel_relocations(
            &mut memory,
            0,
            &[relocation(
                0x1000,
                ArmRelocationKind::JumpSlot,
                1,
                Some("missing"),
            )],
            &missing_resolver,
        )
        .unwrap_err();

        assert_eq!(err, ElfLinkError::UnresolvedSymbol("missing".to_string()));
    }

    fn relocation(
        offset: u32,
        kind: ArmRelocationKind,
        symbol_index: u32,
        symbol_name: Option<&str>,
    ) -> ElfRelocation {
        ElfRelocation {
            table: RelocationTable::Rel,
            offset,
            kind,
            symbol_index,
            symbol_name: symbol_name.map(|name| name.to_string()),
        }
    }

    fn test_resolver(name: &str) -> Option<u32> {
        match name {
            "abs" => Some(0x7100_0000),
            "glob" => Some(0x7200_0000),
            "rel" => Some(0x7000_2000),
            _ => None,
        }
    }

    fn missing_resolver(_name: &str) -> Option<u32> {
        None
    }
}
