//! Pictures embedded in a package, made into something Tern's `image` node draws: PNG, JPEG,
//! GIF, WebP at most 4096 px a side, or SVG at most 256 KiB. BMP and TIFF are converted to PNG;
//! oversized rasters are scaled down. EMF/WMF and anything else are left out (the renderer
//! shows a placeholder).

use image::{ImageFormat, ImageReader};
use std::io::Cursor;

const MAX_SIDE: u32 = 4096;
const MAX_SVG: usize = 256 * 1024;

/// A picture ready to write: bytes, MIME type, file extension and pixel size (0 when unknown).
pub struct Picture {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub ext: &'static str,
    pub w: u32,
    pub h: u32,
}

/// Converts raw part bytes into a drawable picture, or `None` (with a short reason).
pub fn prepare(bytes: Vec<u8>) -> Result<Picture, &'static str> {
    if is_svg(&bytes) {
        if bytes.len() > MAX_SVG {
            return Err("SVG 过大");
        }
        return Ok(Picture { bytes, mime: "image/svg+xml", ext: "svg", w: 0, h: 0 });
    }
    if bytes.starts_with(&[0x01, 0, 0, 0]) || bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || bytes.starts_with(&[1, 0, 9, 0]) {
        return Err("EMF/WMF 图元文件");
    }
    let format = image::guess_format(&bytes).map_err(|_| "未知图片格式")?;
    let (mime, ext, native) = match format {
        ImageFormat::Png => ("image/png", "png", true),
        ImageFormat::Jpeg => ("image/jpeg", "jpg", true),
        ImageFormat::Gif => ("image/gif", "gif", true),
        ImageFormat::WebP => ("image/webp", "webp", true),
        ImageFormat::Bmp | ImageFormat::Tiff => ("image/png", "png", false),
        _ => return Err("不支持的图片格式"),
    };
    let reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let (w, h) = reader.into_dimensions().map_err(|_| "图片损坏")?;
    if native && w <= MAX_SIDE && h <= MAX_SIDE {
        return Ok(Picture { bytes, mime, ext, w, h });
    }
    // Decode, scale to fit, re-encode (JPEG stays JPEG to keep photos small).
    let img = image::load_from_memory_with_format(&bytes, format).map_err(|_| "图片损坏")?;
    let img = if w > MAX_SIDE || h > MAX_SIDE { img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle) } else { img };
    let mut out = Vec::new();
    let (mime, ext) = if format == ImageFormat::Jpeg {
        img.to_rgb8().write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg).map_err(|_| "图片编码失败")?;
        ("image/jpeg", "jpg")
    } else {
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).map_err(|_| "图片编码失败")?;
        ("image/png", "png")
    };
    Ok(Picture { bytes: out, mime, ext, w: img.width(), h: img.height() })
}

fn is_svg(b: &[u8]) -> bool {
    let head = &b[..b.len().min(512)];
    let s = String::from_utf8_lossy(head);
    let t = s.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<?xml") || t.starts_with("<svg") || t.starts_with("<!--")) && s.contains("<svg")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 10, 10]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn small_png_passes_through() {
        let p = prepare(png(3, 2)).unwrap();
        assert_eq!((p.mime, p.w, p.h), ("image/png", 3, 2));
    }

    #[test]
    fn huge_raster_is_scaled() {
        let p = prepare(png(5000, 100)).unwrap();
        assert_eq!(p.w, 4096);
        assert!(p.h <= 100);
    }

    #[test]
    fn bmp_becomes_png() {
        let img = image::RgbImage::from_pixel(4, 4, image::Rgb([0, 0, 0]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Bmp).unwrap();
        let p = prepare(out).unwrap();
        assert_eq!(p.mime, "image/png");
        assert!(p.bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn emf_is_refused() {
        assert!(prepare(vec![1, 0, 0, 0, 0x6C, 0, 0, 0]).is_err());
    }
}
