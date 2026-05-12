use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    pub name: String,
    pub compression: ZipCompression,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
}

impl ZipEntry {
    pub fn saved_percent(&self) -> Option<u64> {
        if self.uncompressed_size == 0 {
            return None;
        }
        let saved = self
            .uncompressed_size
            .saturating_sub(self.compressed_size)
            .saturating_mul(100);
        Some(saved / self.uncompressed_size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipCompression {
    Stored,
    Deflated,
    Other(u16),
}

impl ZipCompression {
    fn from_method(method: u16) -> Self {
        match method {
            0 => Self::Stored,
            8 => Self::Deflated,
            other => Self::Other(other),
        }
    }
}

impl fmt::Display for ZipCompression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stored => write!(f, "stored"),
            Self::Deflated => write!(f, "deflated"),
            Self::Other(method) => write!(f, "method {method}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipProbeError {
    Io(String),
    EndOfCentralDirectoryMissing,
    Truncated(&'static str),
    BadCentralDirectory,
    UnsupportedZip64,
    InvalidName,
}

impl fmt::Display for ZipProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::EndOfCentralDirectoryMissing => {
                write!(f, "ZIP end of central directory not found")
            }
            Self::Truncated(what) => write!(f, "truncated ZIP {what}"),
            Self::BadCentralDirectory => write!(f, "bad ZIP central directory"),
            Self::UnsupportedZip64 => write!(f, "ZIP64 archives are not supported by this probe"),
            Self::InvalidName => write!(f, "ZIP entry name is not valid UTF-8"),
        }
    }
}

impl std::error::Error for ZipProbeError {}

pub fn read_zip_entries(path: &Path) -> Result<Vec<ZipEntry>, ZipProbeError> {
    let bytes = fs::read(path).map_err(|err| ZipProbeError::Io(err.to_string()))?;
    parse_zip_entries(&bytes)
}

pub fn parse_zip_entries(bytes: &[u8]) -> Result<Vec<ZipEntry>, ZipProbeError> {
    let eocd = find_eocd(bytes)?;
    let total_entries = le_u16(bytes, eocd + 10)?;
    let central_size = le_u32(bytes, eocd + 12)?;
    let central_offset = le_u32(bytes, eocd + 16)?;

    if total_entries == u16::MAX || central_size == u32::MAX || central_offset == u32::MAX {
        return Err(ZipProbeError::UnsupportedZip64);
    }

    let mut pos = central_offset as usize;
    let end = pos
        .checked_add(central_size as usize)
        .ok_or(ZipProbeError::BadCentralDirectory)?;
    if end > bytes.len() {
        return Err(ZipProbeError::Truncated("central directory"));
    }

    let mut entries = Vec::with_capacity(total_entries as usize);
    for _ in 0..total_entries {
        if pos + 46 > end {
            return Err(ZipProbeError::Truncated("central directory entry"));
        }
        if le_u32(bytes, pos)? != 0x0201_4b50 {
            return Err(ZipProbeError::BadCentralDirectory);
        }

        let compression = ZipCompression::from_method(le_u16(bytes, pos + 10)?);
        let compressed_size = le_u32(bytes, pos + 20)?;
        let uncompressed_size = le_u32(bytes, pos + 24)?;
        let name_len = le_u16(bytes, pos + 28)? as usize;
        let extra_len = le_u16(bytes, pos + 30)? as usize;
        let comment_len = le_u16(bytes, pos + 32)? as usize;

        if compressed_size == u32::MAX || uncompressed_size == u32::MAX {
            return Err(ZipProbeError::UnsupportedZip64);
        }

        let name_start = pos + 46;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or(ZipProbeError::BadCentralDirectory)?;
        let entry_end = name_end
            .checked_add(extra_len)
            .and_then(|value| value.checked_add(comment_len))
            .ok_or(ZipProbeError::BadCentralDirectory)?;
        if entry_end > end {
            return Err(ZipProbeError::Truncated("central directory entry name"));
        }

        let name = std::str::from_utf8(&bytes[name_start..name_end])
            .map_err(|_| ZipProbeError::InvalidName)?
            .to_string();
        entries.push(ZipEntry {
            name,
            compression,
            compressed_size: u64::from(compressed_size),
            uncompressed_size: u64::from(uncompressed_size),
        });

        pos = entry_end;
    }

    Ok(entries)
}

fn find_eocd(bytes: &[u8]) -> Result<usize, ZipProbeError> {
    if bytes.len() < 22 {
        return Err(ZipProbeError::EndOfCentralDirectoryMissing);
    }

    let min = bytes.len().saturating_sub(22 + u16::MAX as usize);
    for pos in (min..=bytes.len() - 22).rev() {
        if le_u32(bytes, pos)? != 0x0605_4b50 {
            continue;
        }
        let comment_len = le_u16(bytes, pos + 20)? as usize;
        if pos + 22 + comment_len <= bytes.len() {
            return Ok(pos);
        }
    }

    Err(ZipProbeError::EndOfCentralDirectoryMissing)
}

fn le_u16(bytes: &[u8], off: usize) -> Result<u16, ZipProbeError> {
    let raw = bytes
        .get(off..off + 2)
        .ok_or(ZipProbeError::Truncated("u16"))?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn le_u32(bytes: &[u8], off: usize) -> Result<u32, ZipProbeError> {
    let raw = bytes
        .get(off..off + 4)
        .ok_or(ZipProbeError::Truncated("u32"))?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

#[cfg(test)]
mod tests {
    use super::{ZipCompression, parse_zip_entries};

    #[test]
    fn parses_central_directory_entry() {
        let name = b"lib/x.so";
        let mut bytes = Vec::new();
        push_u32(&mut bytes, 0x0201_4b50);
        push_u16(&mut bytes, 20);
        push_u16(&mut bytes, 20);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 8);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 5);
        push_u32(&mut bytes, 10);
        push_u16(&mut bytes, name.len() as u16);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        bytes.extend_from_slice(name);

        let central_size = bytes.len() as u32;
        push_u32(&mut bytes, 0x0605_4b50);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 1);
        push_u32(&mut bytes, central_size);
        push_u32(&mut bytes, 0);
        push_u16(&mut bytes, 0);

        let entries = parse_zip_entries(&bytes).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "lib/x.so");
        assert_eq!(entries[0].compression, ZipCompression::Deflated);
        assert_eq!(entries[0].compressed_size, 5);
        assert_eq!(entries[0].uncompressed_size, 10);
        assert_eq!(entries[0].saved_percent(), Some(50));
    }

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }
}
