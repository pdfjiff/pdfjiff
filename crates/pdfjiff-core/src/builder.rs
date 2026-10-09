//! Incremental image-to-PDF assembly. JPEGs can be embedded without decoding.
use image::{DynamicImage, ImageEncoder, ImageFormat};
use miniz_oxide::deflate::compress_to_vec_zlib;
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref};
const MM_TO_PT: f32 = 72.0 / 25.4;
const A4_PT: (f32, f32) = (595.276, 841.89); // portrait (w, h)
const LETTER_PT: (f32, f32) = (612.0, 792.0);
const ZLIB_LEVEL: u8 = 6;

// ---------------------------------------------------------------------------
// Public option enums
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PageSize {
    /// Every page is sized to its image (plus margins). Scale is always 1:1.
    FitToImage = 0,
    A4 = 1,
    Letter = 2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Per-image: landscape pages for landscape images. Ignored for FitToImage.
    Auto = 0,
    Portrait = 1,
    Landscape = 2,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

pub struct PdfBuilder {
    pdf: Pdf,
    next_ref: i32,
    catalog_id: Ref,
    page_tree_id: Ref,
    page_ids: Vec<Ref>,
    page_size: PageSize,
    orientation: Orientation,
    margin_pt: f32,
    dpi: f32,
    recompress_quality: u8,
}

// Host-independent assembly methods.
impl PdfBuilder {
    pub fn page_count(&self) -> u32 {
        self.page_ids.len() as u32
    }
    pub fn try_new(
        page_size: PageSize,
        orientation: Orientation,
        margin_mm: f32,
        dpi: f32,
        recompress_quality: u8,
    ) -> Result<PdfBuilder, String> {
        let dpi = if dpi.is_finite() && dpi >= 18.0 {
            dpi
        } else {
            96.0
        };
        let margin_pt = margin_mm.clamp(0.0, 100.0) * MM_TO_PT;
        Ok(PdfBuilder {
            pdf: Pdf::new(),
            next_ref: 3, // 1 = catalog, 2 = page tree, written in finish()
            catalog_id: Ref::new(1),
            page_tree_id: Ref::new(2),
            page_ids: Vec::new(),
            page_size,
            orientation,
            margin_pt,
            dpi,
            recompress_quality: recompress_quality.min(100),
        })
    }

    pub fn try_add_image(&mut self, data: &[u8]) -> Result<(), String> {
        let prepared = prepare_image(data, self.recompress_quality)?;
        self.write_page(prepared);
        Ok(())
    }

    pub fn try_finish(mut self) -> Result<Vec<u8>, String> {
        if self.page_ids.is_empty() {
            return Err("Cannot create a PDF with zero pages.".to_string());
        }
        self.pdf.catalog(self.catalog_id).pages(self.page_tree_id);
        let count = self.page_ids.len() as i32;
        self.pdf
            .pages(self.page_tree_id)
            .kids(self.page_ids.iter().copied())
            .count(count);
        Ok(self.pdf.finish())
    }
}

// Internal (not exported) methods.
impl PdfBuilder {
    fn alloc(&mut self) -> Ref {
        let r = Ref::new(self.next_ref);
        self.next_ref += 1;
        r
    }

    fn write_page(&mut self, img: PreparedImage) {
        // --- geometry (points) ---
        let img_w_pt = img.width as f32 * 72.0 / self.dpi;
        let img_h_pt = img.height as f32 * 72.0 / self.dpi;
        let m = self.margin_pt;

        let (page_w, page_h) = match self.page_size {
            PageSize::FitToImage => (img_w_pt + 2.0 * m, img_h_pt + 2.0 * m),
            PageSize::A4 => orient(A4_PT, self.orientation, img_w_pt, img_h_pt),
            PageSize::Letter => orient(LETTER_PT, self.orientation, img_w_pt, img_h_pt),
        };

        let scale = if self.page_size == PageSize::FitToImage {
            1.0
        } else {
            let avail_w = (page_w - 2.0 * m).max(1.0);
            let avail_h = (page_h - 2.0 * m).max(1.0);
            (avail_w / img_w_pt).min(avail_h / img_h_pt)
        };
        let draw_w = img_w_pt * scale;
        let draw_h = img_h_pt * scale;
        let x = (page_w - draw_w) / 2.0;
        let y = (page_h - draw_h) / 2.0;

        // --- object refs ---
        let image_id = self.alloc();
        let smask_id = img.smask.as_ref().map(|_| self.alloc());
        let content_id = self.alloc();
        let page_id = self.alloc();

        // --- image XObject ---
        {
            let payload: &[u8] = match &img.encoded {
                Encoded::Dct { data, .. } => data,
                Encoded::FlateRgb(d) | Encoded::FlateGray(d) => d,
            };
            let mut x_obj = self.pdf.image_xobject(image_id, payload);
            x_obj.width(img.width as i32);
            x_obj.height(img.height as i32);
            x_obj.bits_per_component(8);
            match &img.encoded {
                Encoded::Dct {
                    components,
                    adobe_inverted,
                    ..
                } => {
                    x_obj.filter(Filter::DctDecode);
                    match components {
                        1 => {
                            x_obj.color_space().device_gray();
                        }
                        4 => {
                            x_obj.color_space().device_cmyk();
                            // Adobe APP14 CMYK JPEGs store inverted samples.
                            if *adobe_inverted {
                                x_obj.decode([1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0]);
                            }
                        }
                        _ => {
                            x_obj.color_space().device_rgb();
                        }
                    }
                }
                Encoded::FlateRgb(_) => {
                    x_obj.filter(Filter::FlateDecode);
                    x_obj.color_space().device_rgb();
                }
                Encoded::FlateGray(_) => {
                    x_obj.filter(Filter::FlateDecode);
                    x_obj.color_space().device_gray();
                }
            }
            if let Some(sid) = smask_id {
                x_obj.s_mask(sid);
            }
            x_obj.finish();
        }

        // --- soft mask (alpha channel) ---
        if let (Some(sid), Some(alpha)) = (smask_id, &img.smask) {
            let mut sm = self.pdf.image_xobject(sid, alpha);
            sm.width(img.width as i32);
            sm.height(img.height as i32);
            sm.bits_per_component(8);
            sm.filter(Filter::FlateDecode);
            sm.color_space().device_gray();
            sm.finish();
        }

        // --- content stream: paint the unit-square image scaled into place ---
        let mut content = Content::new();
        content.save_state();
        content.transform([draw_w, 0.0, 0.0, draw_h, x, y]);
        content.x_object(Name(b"Im0"));
        content.restore_state();
        let content_bytes = content.finish();
        self.pdf.stream(content_id, &content_bytes);

        // --- page ---
        {
            let mut page = self.pdf.page(page_id);
            page.media_box(Rect::new(0.0, 0.0, page_w, page_h));
            page.parent(self.page_tree_id);
            page.contents(content_id);
            page.resources().x_objects().pair(Name(b"Im0"), image_id);
            page.finish();
        }
        self.page_ids.push(page_id);
    }
}

fn orient(portrait: (f32, f32), o: Orientation, img_w_pt: f32, img_h_pt: f32) -> (f32, f32) {
    let landscape = match o {
        Orientation::Portrait => false,
        Orientation::Landscape => true,
        Orientation::Auto => img_w_pt > img_h_pt,
    };
    if landscape {
        (portrait.1, portrait.0)
    } else {
        portrait
    }
}

// ---------------------------------------------------------------------------
// Image preparation
// ---------------------------------------------------------------------------

enum Encoded {
    /// Raw JPEG bytes, embedded untouched (or produced by optional re-encode).
    Dct {
        data: Vec<u8>,
        components: u8,
        adobe_inverted: bool,
    },
    /// zlib-compressed raw RGB triplets.
    FlateRgb(Vec<u8>),
    /// zlib-compressed raw 8-bit gray samples.
    FlateGray(Vec<u8>),
}

struct PreparedImage {
    width: u32,
    height: u32,
    encoded: Encoded,
    /// zlib-compressed 8-bit alpha samples, if the image has transparency.
    smask: Option<Vec<u8>>,
}

fn detect_format(data: &[u8]) -> Option<ImageFormat> {
    if data.len() >= 3 && data[..3] == [0xFF, 0xD8, 0xFF] {
        Some(ImageFormat::Jpeg)
    } else if data.len() >= 8 && data[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        Some(ImageFormat::Png)
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

fn prepare_image(data: &[u8], recompress_quality: u8) -> Result<PreparedImage, String> {
    match detect_format(data)
        .ok_or_else(|| "Unsupported file. Supported inputs: JPG, PNG, WebP.".to_string())?
    {
        ImageFormat::Jpeg => prepare_jpeg(data, recompress_quality),
        fmt => prepare_decoded(data, fmt, recompress_quality),
    }
}

/// JPEG handling.
///
/// * `recompress_quality == 0`: passthrough — only the SOF header is parsed and
///   the original bytes are embedded as-is (no decode, no pixel buffers).
/// * `recompress_quality > 0`: the frame is decoded and re-encoded at that
///   quality, and the SMALLER of {original, re-encoded} is embedded, so turning
///   the knob can only ever shrink the output. This is what makes a
///   target-file-size search effective on JPEG-heavy batches. CMYK JPEGs are
///   always passed through (decoding them is unreliable; passthrough is always
///   correct).
fn prepare_jpeg(data: &[u8], recompress_quality: u8) -> Result<PreparedImage, String> {
    let (width, height, components) =
        parse_jpeg_sof(data).ok_or_else(|| "Could not parse JPEG header.".to_string())?;
    check_dimensions(width, height)?;
    if !matches!(components, 1 | 3 | 4) {
        return Err(format!("Unsupported JPEG component count: {components}"));
    }

    if recompress_quality > 0 && components != 4 {
        if let Some(prepared) = reencode_jpeg(data, width, height, components, recompress_quality) {
            return Ok(prepared);
        }
        // Decode/encode failed or didn't shrink — fall through to passthrough.
    }

    Ok(PreparedImage {
        width,
        height,
        encoded: Encoded::Dct {
            data: data.to_vec(),
            components,
            adobe_inverted: components == 4 && has_adobe_app14(data),
        },
        smask: None,
    })
}

/// Best-effort lossy JPEG re-encode. Returns None when decoding fails or the
/// result would not be smaller than the original bytes.
fn reencode_jpeg(
    data: &[u8],
    width: u32,
    height: u32,
    components: u8,
    quality: u8,
) -> Option<PreparedImage> {
    let img = image::load_from_memory_with_format(data, ImageFormat::Jpeg).ok()?;
    let mut out = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);

    // Keep grayscale as a single channel — converting to RGB would triple the
    // uncompressed data for no visual gain.
    let (raw, color, out_components): (Vec<u8>, image::ExtendedColorType, u8) = if components == 1 {
        (img.into_luma8().into_raw(), image::ExtendedColorType::L8, 1)
    } else {
        (
            img.into_rgb8().into_raw(),
            image::ExtendedColorType::Rgb8,
            3,
        )
    };

    encoder.write_image(&raw, width, height, color).ok()?;
    if out.len() >= data.len() {
        return None;
    }
    Some(PreparedImage {
        width,
        height,
        encoded: Encoded::Dct {
            data: out,
            components: out_components,
            adobe_inverted: false,
        },
        smask: None,
    })
}

fn prepare_decoded(
    data: &[u8],
    fmt: ImageFormat,
    recompress_quality: u8,
) -> Result<PreparedImage, String> {
    let img = image::load_from_memory_with_format(data, fmt)
        .map_err(|e| format!("Failed to decode image: {e}"))?;
    let (width, height) = (img.width(), img.height());
    check_dimensions(width, height)?;

    if img.color().has_alpha() {
        // Split interleaved RGBA into RGB + A, drop raw buffers as we go.
        let raw = img.into_rgba8().into_raw();
        let n = raw.len() / 4;
        let mut rgb = Vec::with_capacity(n * 3);
        let mut alpha = Vec::with_capacity(n);
        let mut fully_opaque = true;
        for px in raw.chunks_exact(4) {
            rgb.extend_from_slice(&px[..3]);
            alpha.push(px[3]);
            if px[3] != 255 {
                fully_opaque = false;
            }
        }
        drop(raw);

        if fully_opaque {
            // Alpha channel present but unused — skip the SMask entirely.
            return finish_opaque_rgb(rgb, width, height, recompress_quality);
        }
        let rgb_z = compress_to_vec_zlib(&rgb, ZLIB_LEVEL);
        drop(rgb);
        let alpha_z = compress_to_vec_zlib(&alpha, ZLIB_LEVEL);
        Ok(PreparedImage {
            width,
            height,
            encoded: Encoded::FlateRgb(rgb_z),
            smask: Some(alpha_z),
        })
    } else if let DynamicImage::ImageLuma8(gray) = img {
        let raw = gray.into_raw();
        Ok(PreparedImage {
            width,
            height,
            encoded: Encoded::FlateGray(compress_to_vec_zlib(&raw, ZLIB_LEVEL)),
            smask: None,
        })
    } else {
        finish_opaque_rgb(
            img.into_rgb8().into_raw(),
            width,
            height,
            recompress_quality,
        )
    }
}

fn finish_opaque_rgb(
    rgb: Vec<u8>,
    width: u32,
    height: u32,
    recompress_quality: u8,
) -> Result<PreparedImage, String> {
    let encoded = if recompress_quality > 0 {
        // Optional lossy path: re-encode opaque PNG/WebP as JPEG to keep the
        // output PDF small for photographic content.
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, recompress_quality)
            .write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("JPEG re-encode failed: {e}"))?;
        Encoded::Dct {
            data: out,
            components: 3,
            adobe_inverted: false,
        }
    } else {
        Encoded::FlateRgb(compress_to_vec_zlib(&rgb, ZLIB_LEVEL))
    };
    Ok(PreparedImage {
        width,
        height,
        encoded,
        smask: None,
    })
}

fn check_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("Image has zero width or height.".to_string());
    }
    // Cap the decoded RGBA footprint (~1 GB) so 32-bit hosts such as wasm32
    // cannot run out of address space.
    let pixels = (width as u64) * (height as u64);
    if pixels * 4 > 1 << 30 {
        return Err(format!("Image is too large to process ({width}x{height})."));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Minimal JPEG marker scanning (dimensions + Adobe APP14 detection)
// ---------------------------------------------------------------------------

/// Returns (width, height, component count) from the first SOFn marker.
fn parse_jpeg_sof(data: &[u8]) -> Option<(u32, u32, u8)> {
    scan_jpeg_markers(data, |marker, seg| {
        let is_sof = matches!(
            marker,
            0xC0 | 0xC1
                | 0xC2
                | 0xC3
                | 0xC5
                | 0xC6
                | 0xC7
                | 0xC9
                | 0xCA
                | 0xCB
                | 0xCD
                | 0xCE
                | 0xCF
        );
        if is_sof && seg.len() >= 6 {
            let height = u16::from_be_bytes([seg[1], seg[2]]) as u32;
            let width = u16::from_be_bytes([seg[3], seg[4]]) as u32;
            Some((width, height, seg[5]))
        } else {
            None
        }
    })
}

fn has_adobe_app14(data: &[u8]) -> bool {
    scan_jpeg_markers(data, |marker, seg| {
        (marker == 0xEE && seg.len() >= 5 && &seg[..5] == b"Adobe").then_some(())
    })
    .is_some()
}

/// Walks JPEG segments up to SOS, calling `f(marker, payload)` for each sized
/// segment. Payload excludes the 2 length bytes.
fn scan_jpeg_markers<T>(data: &[u8], f: impl Fn(u8, &[u8]) -> Option<T>) -> Option<T> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            // Corrupt stream — bail rather than scanning entropy-coded data.
            return None;
        }
        let marker = data[i + 1];
        i += 2;
        match marker {
            0xFF => {
                // Fill byte; resynchronize.
                i -= 1;
            }
            0xD8 | 0x01 | 0xD0..=0xD7 => { /* standalone markers, no payload */ }
            0xD9 | 0xDA => return None, // EOI / start of scan: header section over
            _ => {
                if i + 2 > data.len() {
                    return None;
                }
                let len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
                if len < 2 || i + len > data.len() {
                    return None;
                }
                if let Some(t) = f(marker, &data[i + 2..i + len]) {
                    return Some(t);
                }
                i += len;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_jpeg_sof() {
        // SOI, APP0 (len 4, junk), SOF0 for a 640x480 3-component image, SOS.
        let mut jpg = vec![0xFF, 0xD8];
        jpg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]);
        jpg.extend_from_slice(&[
            0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0xE0, 0x02, 0x80, 0x03, 0x01, 0x22, 0x00, 0x02,
            0x11, 0x01, 0x03, 0x11, 0x01,
        ]);
        jpg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        assert_eq!(parse_jpeg_sof(&jpg), Some((640, 480, 3)));
    }

    #[test]
    fn rejects_unknown_magic() {
        assert!(detect_format(b"GIF89a....").is_none());
    }
}
