use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const PNG_COLOR_RGB: u8 = 2;
const PNG_FILTER_NONE: u8 = 0;

pub fn encode_rgb_png(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 {
        return Err(format!("PNG dimensions must be nonzero: {width}x{height}"));
    }
    let width_usize =
        usize::try_from(width).map_err(|_| format!("PNG width too large: {width}"))?;
    let height_usize =
        usize::try_from(height).map_err(|_| format!("PNG height too large: {height}"))?;
    let row_bytes = width_usize
        .checked_mul(3)
        .ok_or_else(|| format!("PNG row too large: {width}x{height}"))?;
    let expected = row_bytes
        .checked_mul(height_usize)
        .ok_or_else(|| format!("PNG image too large: {width}x{height}"))?;
    if rgb.len() != expected {
        return Err(format!(
            "PNG RGB payload length mismatch: got {}, expected {expected} for {width}x{height}",
            rgb.len()
        ));
    }

    let mut filtered = Vec::with_capacity(
        expected
            .checked_add(height_usize)
            .ok_or_else(|| format!("PNG filtered image too large: {width}x{height}"))?,
    );
    for row in rgb.chunks_exact(row_bytes) {
        filtered.push(PNG_FILTER_NONE);
        filtered.extend_from_slice(row);
    }

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(&filtered)
        .map_err(|err| format!("PNG zlib write failed: {err}"))?;
    let compressed = encoder
        .finish()
        .map_err(|err| format!("PNG zlib finish failed: {err}"))?;

    let mut out = Vec::new();
    out.extend_from_slice(PNG_SIGNATURE);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8);
    ihdr.push(PNG_COLOR_RGB);
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(0);
    append_png_chunk(&mut out, b"IHDR", &ihdr);
    append_png_chunk(&mut out, b"IDAT", &compressed);
    append_png_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

fn append_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);

    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

struct Crc32(u32);

impl Crc32 {
    fn new() -> Self {
        Self(0xffff_ffff)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = 0u32.wrapping_sub(self.0 & 1);
                self.0 = (self.0 >> 1) ^ (0xedb8_8320 & mask);
            }
        }
    }

    fn finish(self) -> u32 {
        !self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_rgb_png_signature_and_header() {
        let png = encode_rgb_png(
            2,
            1,
            &[
                0xff, 0x00, 0x00, //
                0x00, 0xff, 0x00,
            ],
        )
        .unwrap();

        assert_eq!(&png[..8], PNG_SIGNATURE);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 2);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 1);
        assert_eq!(png[24], 8);
        assert_eq!(png[25], PNG_COLOR_RGB);
        assert!(png.windows(4).any(|window| window == b"IDAT"));
        assert_eq!(&png[png.len() - 12 + 4..png.len() - 4], b"IEND");
    }

    #[test]
    fn rejects_mismatched_rgb_payload() {
        let err = encode_rgb_png(2, 1, &[0, 1, 2]).unwrap_err();
        assert!(err.contains("payload length mismatch"));
    }
}
