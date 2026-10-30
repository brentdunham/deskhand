//! Screenshot post-processing: pixel conversion, blank-frame detection,
//! downscaling, and PNG encoding. Platform-independent.

use base64::Engine;

/// 8-bit RGB pixels, row-major, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Converts 32-bit BGRA (the Windows DIB layout) to RGB, dropping alpha.
pub fn bgra_to_rgb(bgra: &[u8], width: u32, height: u32) -> RgbImage {
    let mut data = Vec::with_capacity(width as usize * height as usize * 3);
    for px in bgra.chunks_exact(4) {
        data.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    RgbImage {
        width,
        height,
        data,
    }
}

/// Every pixel is black or nearly so. DRM-protected video (Netflix, Disney+,
/// and similar) captures this way, so such frames are dropped rather than
/// sent as a useless image.
pub fn is_blank(img: &RgbImage) -> bool {
    img.data.iter().all(|&c| c <= 8)
}

/// Shrinks the image so its longer edge is at most `max_edge`, averaging the
/// source pixels each destination pixel covers. Never enlarges.
pub fn downscale(img: &RgbImage, max_edge: u32) -> RgbImage {
    let longest = img.width.max(img.height);
    if longest <= max_edge || img.width == 0 || img.height == 0 {
        return img.clone();
    }
    let scale = f64::from(max_edge) / f64::from(longest);
    let dw = ((f64::from(img.width) * scale).round() as u32).max(1);
    let dh = ((f64::from(img.height) * scale).round() as u32).max(1);
    let (sw, sh) = (img.width as usize, img.height as usize);
    let mut data = Vec::with_capacity(dw as usize * dh as usize * 3);
    for dy in 0..dh as usize {
        let y0 = dy * sh / dh as usize;
        let y1 = ((dy + 1) * sh / dh as usize).max(y0 + 1);
        for dx in 0..dw as usize {
            let x0 = dx * sw / dw as usize;
            let x1 = ((dx + 1) * sw / dw as usize).max(x0 + 1);
            let mut sum = [0u64; 3];
            for y in y0..y1 {
                let row = &img.data[(y * sw + x0) * 3..(y * sw + x1) * 3];
                for px in row.chunks_exact(3) {
                    sum[0] += u64::from(px[0]);
                    sum[1] += u64::from(px[1]);
                    sum[2] += u64::from(px[2]);
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            data.extend(sum.iter().map(|s| (s / n) as u8));
        }
    }
    RgbImage {
        width: dw,
        height: dh,
        data,
    }
}

pub fn encode_png(img: &RgbImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, img.width, img.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(&img.data)
        .map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn to_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> RgbImage {
        RgbImage {
            width: w,
            height: h,
            data: rgb.repeat((w * h) as usize),
        }
    }

    #[test]
    fn converts_bgra() {
        let img = bgra_to_rgb(&[1, 2, 3, 255, 10, 20, 30, 0], 2, 1);
        assert_eq!(img.data, [3, 2, 1, 30, 20, 10]);
    }

    #[test]
    fn detects_blank_frames() {
        assert!(is_blank(&solid(4, 4, [0, 0, 0])));
        assert!(is_blank(&solid(4, 4, [8, 3, 0])));
        let mut dark_theme = solid(4, 4, [30, 30, 30]);
        dark_theme.data[0] = 200;
        assert!(!is_blank(&dark_theme));
    }

    #[test]
    fn downscales_preserving_aspect_and_averaging() {
        let img = solid(3000, 1500, [100, 150, 200]);
        let small = downscale(&img, 1568);
        assert_eq!((small.width, small.height), (1568, 784));
        assert!(small.data.chunks_exact(3).all(|p| p == [100, 150, 200]));

        // Half white, half black averages to grey at 2x reduction.
        let mut stripes = solid(4, 2, [0, 0, 0]);
        for y in 0..2 {
            for x in [0usize, 2] {
                let i = (y * 4 + x) * 3;
                stripes.data[i..i + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        let half = downscale(&stripes, 2);
        assert_eq!((half.width, half.height), (2, 1));
        assert_eq!(&half.data[..3], &[127, 127, 127]);
    }

    #[test]
    fn never_enlarges() {
        let img = solid(100, 50, [1, 2, 3]);
        assert_eq!(downscale(&img, 1568), img);
    }

    #[test]
    fn encodes_valid_png() {
        let png = encode_png(&solid(3, 2, [9, 9, 9])).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let decoder = png::Decoder::new(std::io::Cursor::new(&png));
        let reader = decoder.read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (3, 2));
        assert!(!to_base64(&png).is_empty());
    }
}
