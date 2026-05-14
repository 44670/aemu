const GL_ALPHA: u32 = 0x1906;
const GL_RGB: u32 = 0x1907;
const GL_RGBA: u32 = 0x1908;
const GL_LUMINANCE: u32 = 0x1909;
const GL_LUMINANCE_ALPHA: u32 = 0x190a;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_UNSIGNED_SHORT_4_4_4_4: u32 = 0x8033;
const GL_UNSIGNED_SHORT_5_5_5_1: u32 = 0x8034;
const GL_UNSIGNED_SHORT_5_6_5: u32 = 0x8363;
const GL_BGRA_EXT: u32 = 0x80e1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TexturePayloadStats {
    pub(crate) nonzero_rgb_pixels: usize,
    pub(crate) nonzero_alpha_pixels: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextureUploadMatch<'a> {
    pub(crate) kind: Option<&'a str>,
    pub(crate) texture: u32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) format: u32,
    pub(crate) ty: u32,
}

pub(crate) fn texture_upload_matches(matcher: &str, upload: TextureUploadMatch<'_>) -> bool {
    matcher
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .any(|token| {
            token.eq_ignore_ascii_case("all")
                || upload
                    .kind
                    .is_some_and(|kind| token.eq_ignore_ascii_case(kind))
                || token == upload.texture.to_string()
                || token == format!("tex{}", upload.texture)
                || token.eq_ignore_ascii_case(&format!("0x{:x}", upload.texture))
                || token.eq_ignore_ascii_case(&format!("{}x{}", upload.width, upload.height))
                || token.eq_ignore_ascii_case(&format!("fmt{:04x}", upload.format))
                || token.eq_ignore_ascii_case(&format!("ty{:04x}", upload.ty))
        })
}

pub(crate) fn texture_payload_stats(
    width: i32,
    height: i32,
    format: u32,
    ty: u32,
    payload: &[u8],
) -> Option<TexturePayloadStats> {
    let pixel_count = pixel_count(width, height)?;
    let mut nonzero_rgb_pixels = 0usize;
    let mut nonzero_alpha_pixels = 0usize;
    match (format, ty) {
        (GL_RGBA, GL_UNSIGNED_BYTE) | (GL_BGRA_EXT, GL_UNSIGNED_BYTE) => {
            for pixel in payload.chunks_exact(4).take(pixel_count) {
                if pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0 {
                    nonzero_rgb_pixels += 1;
                }
                if pixel[3] != 0 {
                    nonzero_alpha_pixels += 1;
                }
            }
        }
        (GL_RGB, GL_UNSIGNED_BYTE) => {
            for pixel in payload.chunks_exact(3).take(pixel_count) {
                if pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0 {
                    nonzero_rgb_pixels += 1;
                }
                nonzero_alpha_pixels += 1;
            }
        }
        (GL_ALPHA, GL_UNSIGNED_BYTE) => {
            for alpha in payload.iter().copied().take(pixel_count) {
                if alpha != 0 {
                    nonzero_alpha_pixels += 1;
                }
            }
        }
        (GL_LUMINANCE, GL_UNSIGNED_BYTE) => {
            for luminance in payload.iter().copied().take(pixel_count) {
                if luminance != 0 {
                    nonzero_rgb_pixels += 1;
                }
                nonzero_alpha_pixels += 1;
            }
        }
        (GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE) => {
            for pixel in payload.chunks_exact(2).take(pixel_count) {
                if pixel[0] != 0 {
                    nonzero_rgb_pixels += 1;
                }
                if pixel[1] != 0 {
                    nonzero_alpha_pixels += 1;
                }
            }
        }
        (GL_RGB, GL_UNSIGNED_SHORT_5_6_5) => {
            for pixel in payload.chunks_exact(2).take(pixel_count) {
                let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                if value != 0 {
                    nonzero_rgb_pixels += 1;
                }
                nonzero_alpha_pixels += 1;
            }
        }
        (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4) => {
            for pixel in payload.chunks_exact(2).take(pixel_count) {
                let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                if value & 0x0fff != 0 {
                    nonzero_rgb_pixels += 1;
                }
                if value & 0xf000 != 0 {
                    nonzero_alpha_pixels += 1;
                }
            }
        }
        (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1) => {
            for pixel in payload.chunks_exact(2).take(pixel_count) {
                let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                if value & 0xfffe != 0 {
                    nonzero_rgb_pixels += 1;
                }
                if value & 0x0001 != 0 {
                    nonzero_alpha_pixels += 1;
                }
            }
        }
        _ => return None,
    }
    Some(TexturePayloadStats {
        nonzero_rgb_pixels,
        nonzero_alpha_pixels,
    })
}

pub(crate) fn texture_payload_to_rgb(
    width: i32,
    height: i32,
    format: u32,
    ty: u32,
    payload: &[u8],
) -> Option<Vec<u8>> {
    let pixel_count = pixel_count(width, height)?;
    let mut out = Vec::with_capacity(pixel_count.checked_mul(3)?);
    match (format, ty) {
        (GL_RGBA, GL_UNSIGNED_BYTE) => {
            for pixel in payload.chunks_exact(4).take(pixel_count) {
                out.extend_from_slice(&pixel[..3]);
            }
        }
        (GL_BGRA_EXT, GL_UNSIGNED_BYTE) => {
            for pixel in payload.chunks_exact(4).take(pixel_count) {
                out.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            }
        }
        (GL_RGB, GL_UNSIGNED_BYTE) => {
            for pixel in payload.chunks_exact(3).take(pixel_count) {
                out.extend_from_slice(pixel);
            }
        }
        (GL_ALPHA, GL_UNSIGNED_BYTE) | (GL_LUMINANCE, GL_UNSIGNED_BYTE) => {
            for value in payload.iter().copied().take(pixel_count) {
                out.extend_from_slice(&[value, value, value]);
            }
        }
        (GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE) => {
            for pixel in payload.chunks_exact(2).take(pixel_count) {
                out.extend_from_slice(&[pixel[0], pixel[0], pixel[0]]);
            }
        }
        (GL_RGB, GL_UNSIGNED_SHORT_5_6_5) => {
            for pixel in payload.chunks_exact(2).take(pixel_count) {
                let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                let r = ((value >> 11) & 0x1f) as u8;
                let g = ((value >> 5) & 0x3f) as u8;
                let b = (value & 0x1f) as u8;
                out.extend_from_slice(&[
                    (r << 3) | (r >> 2),
                    (g << 2) | (g >> 4),
                    (b << 3) | (b >> 2),
                ]);
            }
        }
        (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4) => {
            for pixel in payload.chunks_exact(2).take(pixel_count) {
                let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                let r = ((value >> 0) & 0x0f) as u8 * 17;
                let g = ((value >> 4) & 0x0f) as u8 * 17;
                let b = ((value >> 8) & 0x0f) as u8 * 17;
                out.extend_from_slice(&[r, g, b]);
            }
        }
        (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1) => {
            for pixel in payload.chunks_exact(2).take(pixel_count) {
                let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                let r = ((value >> 0) & 0x1f) as u8;
                let g = ((value >> 5) & 0x1f) as u8;
                let b = ((value >> 10) & 0x1f) as u8;
                out.extend_from_slice(&[
                    (r << 3) | (r >> 2),
                    (g << 3) | (g >> 2),
                    (b << 3) | (b >> 2),
                ]);
            }
        }
        _ => return None,
    }
    if out.len() == pixel_count.checked_mul(3)? {
        Some(out)
    } else {
        None
    }
}

fn pixel_count(width: i32, height: i32) -> Option<usize> {
    usize::try_from(width)
        .ok()
        .zip(usize::try_from(height).ok())
        .and_then(|(width, height)| width.checked_mul(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_matcher_accepts_kind_size_texture_and_format_tokens() {
        let upload = TextureUploadMatch {
            kind: Some("texsubimage2d"),
            texture: 325,
            width: 64,
            height: 32,
            format: GL_RGBA,
            ty: GL_UNSIGNED_BYTE,
        };

        assert!(texture_upload_matches("texsubimage2d", upload));
        assert!(texture_upload_matches("64x32", upload));
        assert!(texture_upload_matches("tex325", upload));
        assert!(texture_upload_matches("fmt1908,ty1401", upload));
        assert!(!texture_upload_matches("teximage2d,128x128,tex1", upload));
    }

    #[test]
    fn converts_common_upload_formats_to_rgb() {
        assert_eq!(
            texture_payload_to_rgb(
                1,
                1,
                GL_BGRA_EXT,
                GL_UNSIGNED_BYTE,
                &[0x10, 0x20, 0x30, 0xff]
            )
            .unwrap(),
            vec![0x30, 0x20, 0x10]
        );
        assert_eq!(
            texture_payload_to_rgb(1, 1, GL_RGB, GL_UNSIGNED_SHORT_5_6_5, &[0x00, 0xf8]).unwrap(),
            vec![0xff, 0x00, 0x00]
        );
        assert_eq!(
            texture_payload_to_rgb(2, 1, GL_ALPHA, GL_UNSIGNED_BYTE, &[0x11, 0xee]).unwrap(),
            vec![0x11, 0x11, 0x11, 0xee, 0xee, 0xee]
        );
    }

    #[test]
    fn rejects_short_upload_payloads_for_png_conversion() {
        assert!(texture_payload_to_rgb(2, 1, GL_RGBA, GL_UNSIGNED_BYTE, &[1, 2, 3, 4]).is_none());
    }

    #[test]
    fn counts_nonzero_rgba_pixels() {
        let stats = texture_payload_stats(
            2,
            1,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            &[0, 0, 0, 0x80, 0x12, 0, 0, 0],
        )
        .unwrap();

        assert_eq!(stats.nonzero_rgb_pixels, 1);
        assert_eq!(stats.nonzero_alpha_pixels, 1);
    }
}
