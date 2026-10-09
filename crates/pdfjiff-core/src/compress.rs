//! PDF compression / optimisation.
//!
//! Complements `PdfBuilder` (which *creates* PDFs from images) with a pass that
//! *shrinks* an existing PDF. The implementation is independent of its caller and runs locally.
//!
//! Strategy, in the order the wins actually land:
//!
//! 1. **Image re-encode** — the dominant cost in almost every real-world PDF.
//!    Embedded DCTDecode (JPEG) images are decoded and re-encoded at the target
//!    quality; FlateDecode images whose colour space we fully understand are
//!    converted to DCTDecode, which is usually a large win for scans and
//!    photos. Every replacement is guarded: if the new bytes are not smaller,
//!    the original stream is kept untouched. The pass can only ever shrink.
//! 2. **Downsampling** — an image whose longest edge exceeds `max_edge` is
//!    scaled down before re-encoding. This is what makes the "smallest file"
//!    preset meaningfully smaller than a pure quality drop.
//! 3. **Structural stripping** — annotations, AcroForm data, embedded file
//!    attachments and XMP/document metadata, each behind its own flag.
//! 4. **Object hygiene** — zero-length streams dropped, orphans pruned, object
//!    numbers compacted, remaining streams Flate-compressed.
//!
//! Conservative by construction: anything we cannot decode with confidence
//! (CMYK JPEG, Indexed / Separation colour, sub-byte bit depths, CCITTFax, JBIG2,
//! JPXDecode) is left exactly as it was rather than risking a corrupted page.

use image::{DynamicImage, ImageEncoder, ImageFormat, RgbImage};
use lopdf::{Dictionary, Document, Object, ObjectId, SaveOptions};

/// Never attempt to decode an image stream larger than this. A malformed
/// /Width * /Height pair should not be able to ask for a 40 GB allocation.
const MAX_IMAGE_PIXELS: u64 = 80_000_000;

/// Cap on a single decompressed stream, so a zip-bomb-shaped PDF cannot exhaust
/// memory (including the 4 GiB heap of a wasm32 host).
const MAX_STREAM_BYTES: usize = 256 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Preset shorthand for the quality/downsample pair. A preset only supplies
/// defaults — an explicit `image_quality` or `max_edge` always wins.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompressionPreset {
    /// Aggressive: quality 50, longest edge capped at 1400 px.
    Small = 0,
    /// The recommended default: quality 72, longest edge capped at 2000 px.
    Balanced = 1,
    /// Quality 86, with only very large images capped at 3200 px.
    Quality = 2,
}

impl CompressionPreset {
    fn default_quality(self) -> u8 {
        match self {
            CompressionPreset::Small => 50,
            CompressionPreset::Balanced => 72,
            CompressionPreset::Quality => 86,
        }
    }

    fn default_max_edge(self) -> u32 {
        match self {
            CompressionPreset::Small => 1400,
            CompressionPreset::Balanced => 2000,
            CompressionPreset::Quality => 3200,
        }
    }
}

/// Knobs for one compression run. Start from [`CompressOptions::from_preset`],
/// [`CompressOptions::lossless`] or [`CompressOptions::new`].
#[derive(Clone, Copy, Debug)]
pub struct CompressOptions {
    preset: CompressionPreset,
    lossless: bool,
    image_quality: u8,
    max_edge: u32,
    remove_annotations: bool,
    remove_forms: bool,
    remove_embedded_files: bool,
    remove_metadata: bool,
}

impl CompressOptions {
    /// * `image_quality` — 1..=100 JPEG quality. Pass 0 to take the preset's
    ///   default. Images are never re-encoded upward: if the re-encode is not
    ///   smaller than the original stream, the original is kept.
    /// * `max_edge` — downsample any image whose longest edge exceeds this, in
    ///   pixels. Pass 0 to take the preset's default; pass `u32::MAX` for
    ///   "never downsample" regardless of preset.
    pub fn new(
        preset: CompressionPreset,
        image_quality: u8,
        max_edge: u32,
        remove_annotations: bool,
        remove_forms: bool,
        remove_embedded_files: bool,
        remove_metadata: bool,
    ) -> CompressOptions {
        CompressOptions {
            lossless: false,
            preset,
            image_quality,
            max_edge,
            remove_annotations,
            remove_forms,
            remove_embedded_files,
            remove_metadata,
        }
    }

    /// The preset's defaults, with nothing stripped.
    pub fn from_preset(preset: CompressionPreset) -> CompressOptions {
        CompressOptions {
            lossless: false,
            preset,
            image_quality: 0,
            max_edge: 0,
            remove_annotations: false,
            remove_forms: false,
            remove_embedded_files: false,
            remove_metadata: false,
        }
    }
}

impl CompressOptions {
    /// Preserve image pixels; optimize only document structure.
    pub fn lossless() -> Self {
        let mut options = Self::from_preset(CompressionPreset::Quality);
        options.lossless = true;
        options
    }
    fn quality(&self) -> u8 {
        match self.image_quality {
            0 => self.preset.default_quality(),
            q => q.min(100),
        }
    }

    fn max_edge(&self) -> u32 {
        match self.max_edge {
            0 => self.preset.default_max_edge(),
            u32::MAX => 0,
            e => e,
        }
    }
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

/// Outcome of one run. [`take`](Self::take) moves the output bytes out
/// instead of copying them.
#[derive(Debug)]
pub struct CompressResult {
    bytes: Option<Vec<u8>>,
    original_size: u32,
    compressed_size: u32,
    page_count: u32,
    images_recompressed: u32,
    images_downsampled: u32,
}

impl CompressResult {
    /// Move the PDF bytes out. Returns an empty array if called twice.
    pub fn take(&mut self) -> Vec<u8> {
        self.bytes.take().unwrap_or_default()
    }

    pub fn original_size(&self) -> u32 {
        self.original_size
    }

    pub fn compressed_size(&self) -> u32 {
        self.compressed_size
    }

    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    pub fn images_recompressed(&self) -> u32 {
        self.images_recompressed
    }

    pub fn images_downsampled(&self) -> u32 {
        self.images_downsampled
    }

    /// Fraction of the original size removed, 0.0..=1.0.
    pub fn saved_ratio(&self) -> f64 {
        if self.original_size == 0 {
            return 0.0;
        }
        let saved = self.original_size.saturating_sub(self.compressed_size);
        f64::from(saved) / f64::from(self.original_size)
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Compress `data` in place and return the new document.
///
/// The result is only ever handed back when it is genuinely smaller than the
/// input; a PDF that is already well optimised comes back byte-identical, with
/// [`saved_ratio`](CompressResult::saved_ratio) `== 0.0`, rather than slightly larger.
pub fn try_compress(data: &[u8], options: &CompressOptions) -> Result<CompressResult, String> {
    if data.len() > u32::MAX as usize {
        return Err("PDF is too large to process.".to_string());
    }

    let mut doc = Document::load_mem(data).map_err(|e| format!("Could not read this PDF: {e}"))?;

    if doc.is_encrypted() {
        // An encrypted document would need the password to rewrite streams.
        // Failing loudly beats silently emitting an unreadable file.
        return Err("This PDF is password-protected. Remove the password first.".to_string());
    }

    if options.remove_annotations {
        strip_annotations(&mut doc);
    }
    if options.remove_forms {
        strip_forms(&mut doc);
    }
    if options.remove_embedded_files {
        strip_embedded_files(&mut doc);
    }
    if options.remove_metadata {
        strip_metadata(&mut doc);
    }

    let tally = if options.lossless {
        Tally::default()
    } else {
        recompress_images(&mut doc, options.quality(), options.max_edge())
    };

    doc.delete_zero_length_streams();
    doc.prune_objects();
    doc.renumber_objects();
    doc.compress();

    let page_count = doc.get_pages().len() as u32;

    // PDF 1.5 object streams pack the many tiny dictionaries and arrays that a
    // traditional xref writer emits one-by-one. This is lossless and often a
    // material win on digitally-created PDFs where there are few images to
    // recompress. Level 9 only applies to those structural streams; JPEG image
    // quality remains controlled exclusively by the reader's setting above.
    let save_options = SaveOptions::builder()
        .use_object_streams(true)
        .use_xref_streams(true)
        .max_objects_per_stream(200)
        .compression_level(9)
        .build();
    let mut out = Vec::with_capacity(data.len());
    doc.save_with_options(&mut out, save_options)
        .map_err(|e| format!("Could not write the optimized PDF: {e}"))?;

    // Never hand back something bigger than we were given.
    if out.len() >= data.len() {
        out = data.to_vec();
    }

    Ok(CompressResult {
        original_size: data.len() as u32,
        compressed_size: out.len() as u32,
        bytes: Some(out),
        page_count,
        images_recompressed: tally.recompressed,
        images_downsampled: tally.downsampled,
    })
}

// ---------------------------------------------------------------------------
// Structural stripping
// ---------------------------------------------------------------------------

/// Drop `/Annots` from every page. Covers comments, highlights, stamps and
/// form widgets — the visible field boxes disappear along with the data.
fn strip_annotations(doc: &mut Document) {
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    for id in page_ids {
        if let Ok(dict) = doc.get_dictionary_mut(id) {
            dict.remove(b"Annots");
        }
    }
}

/// Drop the interactive form dictionary from the catalog. Field *appearances*
/// that were flattened into page content are unaffected; only the fillable
/// layer goes.
fn strip_forms(doc: &mut Document) {
    if let Ok(catalog) = doc.catalog_mut() {
        catalog.remove(b"AcroForm");
        catalog.remove(b"XFA");
    }
}

/// Drop file attachments: the catalog's `/Names /EmbeddedFiles` tree, and the
/// `/EF` payload on any FileAttachment annotation that survived.
fn strip_embedded_files(doc: &mut Document) {
    let names_id = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Names").ok())
        .and_then(|n| match n {
            Object::Reference(id) => Some(*id),
            _ => None,
        });

    if let Some(id) = names_id {
        if let Ok(dict) = doc.get_dictionary_mut(id) {
            dict.remove(b"EmbeddedFiles");
        }
    } else if let Ok(catalog) = doc.catalog_mut() {
        if let Ok(Object::Dictionary(names)) = catalog.get_mut(b"Names") {
            names.remove(b"EmbeddedFiles");
        }
    }

    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        if let Ok(Object::Dictionary(dict)) = doc.get_object_mut(id) {
            if matches!(dict.get(b"Type"), Ok(Object::Name(t)) if t == b"Filespec") {
                dict.remove(b"EF");
            }
        }
    }
}

/// Drop the XMP metadata stream and the document information dictionary.
fn strip_metadata(doc: &mut Document) {
    if let Ok(catalog) = doc.catalog_mut() {
        catalog.remove(b"Metadata");
    }
    doc.trailer.remove(b"Info");
}

// ---------------------------------------------------------------------------
// Image re-encoding
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Tally {
    recompressed: u32,
    downsampled: u32,
}

/// What we managed to turn an image stream into, before deciding whether to
/// keep it.
struct Reencoded {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    grayscale: bool,
    downsampled: bool,
}

fn recompress_images(doc: &mut Document, quality: u8, max_edge: u32) -> Tally {
    let mut tally = Tally::default();
    if quality == 0 {
        return tally;
    }

    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();

    for id in ids {
        let Some(new) = plan_reencode(doc, id, quality, max_edge) else {
            continue;
        };

        let Ok(Object::Stream(stream)) = doc.get_object_mut(id) else {
            continue;
        };

        // Guard: only adopt the re-encode if it is actually smaller.
        if new.bytes.len() >= stream.content.len() {
            continue;
        }

        stream
            .dict
            .set("Width", Object::Integer(i64::from(new.width)));
        stream
            .dict
            .set("Height", Object::Integer(i64::from(new.height)));
        stream.dict.set("BitsPerComponent", Object::Integer(8));
        stream.dict.set(
            "ColorSpace",
            Object::Name(
                if new.grayscale {
                    &b"DeviceGray"[..]
                } else {
                    &b"DeviceRGB"[..]
                }
                .to_vec(),
            ),
        );
        stream
            .dict
            .set("Filter", Object::Name(b"DCTDecode".to_vec()));
        // DecodeParms describes the *old* filter chain; carrying it over would
        // make the DCT stream unreadable.
        stream.dict.remove(b"DecodeParms");
        stream.dict.remove(b"DP");
        stream.set_content(new.bytes);

        tally.recompressed += 1;
        if new.downsampled {
            tally.downsampled += 1;
        }
    }

    tally
}

/// Decide what a given object should become, without holding a mutable borrow
/// of the document across the (expensive) decode.
fn plan_reencode(doc: &Document, id: ObjectId, quality: u8, max_edge: u32) -> Option<Reencoded> {
    let Ok(Object::Stream(stream)) = doc.get_object(id) else {
        return None;
    };
    let dict = &stream.dict;

    if !matches!(dict.get(b"Subtype"), Ok(Object::Name(s)) if s == b"Image") {
        return None;
    }
    // An /ImageMask is a 1-bit stencil, not a picture. JPEG cannot represent it.
    if matches!(dict.get(b"ImageMask"), Ok(Object::Boolean(true))) {
        return None;
    }

    let width = integer(doc, dict, b"Width")? as u32;
    let height = integer(doc, dict, b"Height")? as u32;
    if width == 0 || height == 0 {
        return None;
    }
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return None;
    }

    // A /Decode array remaps sample values (most often inverting them). Our
    // re-encode writes plain samples, which would silently flip such an image.
    if dict.get(b"Decode").is_ok() {
        return None;
    }

    let filters = filter_names(dict);
    let parms = decode_parms(dict, filters.len());

    // The chain ends in an image codec, or in nothing (raw samples). Everything
    // before that is transport encoding we have to unwrap first — real PDFs
    // routinely ship `[/ASCII85Decode /DCTDecode]`, and feeding the still-ASCII85
    // bytes to a JPEG decoder just fails.
    let ends_in_codec = filters.last().is_some_and(|f| is_image_codec(f));
    let split = if ends_in_codec {
        filters.len() - 1
    } else {
        filters.len()
    };

    // JPXDecode / JBIG2Decode / CCITTFaxDecode: specialised codecs whose output
    // we cannot reconstruct faithfully. Leave those streams untouched.
    if ends_in_codec && filters[split] != b"DCTDecode" {
        return None;
    }

    let bytes = decode_chain(&stream.content, &filters[..split], &parms)?;

    let decoded: DynamicImage = if ends_in_codec {
        // Re-encoding a CMYK JPEG through an RGB round-trip shifts colour.
        // Passthrough is always correct, so leave it alone.
        if components_of(doc, dict) == Some(4) {
            return None;
        }
        image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg).ok()?
    } else {
        raw_to_image(doc, dict, &bytes, width, height)?
    };

    let (img, downsampled) = maybe_downsample(decoded, max_edge);

    // A stream with soft-mask transparency keeps its own SMask object; the base
    // image is opaque, so a JPEG body is safe. Alpha inside the base image is
    // not, and DynamicImage has already dropped it in the conversions below —
    // so only convert when we know the source had no alpha of its own.
    let source_is_grayscale = matches!(
        img,
        DynamicImage::ImageLuma8(_) | DynamicImage::ImageLuma16(_)
    );

    if source_is_grayscale {
        let luma = img.to_luma8();
        let (w, h) = (luma.width(), luma.height());
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
            .write_image(luma.as_raw(), w, h, image::ExtendedColorType::L8)
            .ok()?;
        return Some(Reencoded {
            bytes,
            width: w,
            height: h,
            grayscale: true,
            downsampled,
        });
    }

    let rgb = img.into_rgb8();
    let (w, h) = (rgb.width(), rgb.height());

    // Many scanners label monochrome pages DeviceRGB and store three almost
    // identical channels. JPEG decoding can introduce a 1-2 level chroma
    // wobble, so allow a tiny tolerance and write one luminance channel. The
    // maximum colour change is below one perceptual step, while scan-heavy
    // documents avoid carrying two redundant channels.
    if rgb_is_effectively_grayscale(&rgb) {
        let luma = DynamicImage::ImageRgb8(rgb).to_luma8();
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
            .write_image(luma.as_raw(), w, h, image::ExtendedColorType::L8)
            .ok()?;
        return Some(Reencoded {
            bytes,
            width: w,
            height: h,
            grayscale: true,
            downsampled,
        });
    }

    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .write_image(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .ok()?;

    Some(Reencoded {
        bytes,
        width: w,
        height: h,
        grayscale: false,
        downsampled,
    })
}

/// RGB scans that are visually monochrome are common. A tolerance of 3 out of
/// 255 absorbs JPEG's chroma rounding without treating tinted paper or coloured
/// marks as grey.
fn rgb_is_effectively_grayscale(image: &RgbImage) -> bool {
    image.pixels().all(|pixel| {
        let [r, g, b] = pixel.0;
        r.max(g).max(b) - r.min(g).min(b) <= 3
    })
}

fn maybe_downsample(img: DynamicImage, max_edge: u32) -> (DynamicImage, bool) {
    if max_edge == 0 {
        return (img, false);
    }
    let longest = img.width().max(img.height());
    if longest <= max_edge {
        return (img, false);
    }
    let scale = f64::from(max_edge) / f64::from(longest);
    let w = ((f64::from(img.width()) * scale).round() as u32).max(1);
    let h = ((f64::from(img.height()) * scale).round() as u32).max(1);
    (
        img.resize_exact(w, h, image::imageops::FilterType::Lanczos3),
        true,
    )
}

/// Rebuild a `DynamicImage` from an uncompressed PDF sample stream. Only the
/// unambiguous 8-bit DeviceGray / DeviceRGB cases are handled; everything else
/// (Indexed, Separation, ICCBased with an unexpected /N, 1/2/4-bit depths)
/// returns None so the caller leaves the stream alone.
fn raw_to_image(
    doc: &Document,
    dict: &Dictionary,
    raw: &[u8],
    width: u32,
    height: u32,
) -> Option<DynamicImage> {
    if integer(doc, dict, b"BitsPerComponent")? != 8 {
        return None;
    }
    let components = components_of(doc, dict)?;
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(components as usize)?;
    if raw.len() < expected {
        return None;
    }
    let pixels = raw[..expected].to_vec();

    match components {
        1 => image::GrayImage::from_raw(width, height, pixels).map(DynamicImage::ImageLuma8),
        3 => RgbImage::from_raw(width, height, pixels).map(DynamicImage::ImageRgb8),
        _ => None,
    }
}

/// Component count implied by the stream's colour space, or None when the
/// space is one we deliberately refuse to guess at.
fn components_of(doc: &Document, dict: &Dictionary) -> Option<u8> {
    let space = dict.get(b"ColorSpace").ok()?;
    let (_, space) = doc.dereference(space).ok()?;

    match space {
        Object::Name(name) => match name.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" => Some(1),
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(3),
            b"DeviceCMYK" | b"CMYK" => Some(4),
            _ => None,
        },
        Object::Array(items) => {
            let head = items.first()?;
            let (_, head) = doc.dereference(head).ok()?;
            let Object::Name(name) = head else {
                return None;
            };
            match name.as_slice() {
                b"ICCBased" => {
                    let (_, stream) = doc.dereference(items.get(1)?).ok()?;
                    let n = stream
                        .as_stream()
                        .ok()?
                        .dict
                        .get(b"N")
                        .ok()?
                        .as_i64()
                        .ok()?;
                    u8::try_from(n).ok().filter(|n| matches!(n, 1 | 3 | 4))
                }
                b"CalGray" => Some(1),
                b"CalRGB" | b"Lab" => Some(3),
                // Indexed / Separation / DeviceN need the palette or tint
                // transform applied before the samples mean anything.
                _ => None,
            }
        }
        _ => None,
    }
}

/// Codecs that produce an image rather than raw samples, and so must terminate
/// the filter chain.
fn is_image_codec(name: &[u8]) -> bool {
    matches!(
        name,
        b"DCTDecode" | b"DCT" | b"JPXDecode" | b"JBIG2Decode" | b"CCITTFaxDecode" | b"CCF"
    )
}

/// `/DecodeParms` is either a single dictionary (applying to the one filter
/// that needs it) or an array positionally aligned with `/Filter`. Normalised
/// here to one slot per filter.
fn decode_parms(dict: &Dictionary, filter_count: usize) -> Vec<Option<Object>> {
    match dict.get(b"DecodeParms").or_else(|_| dict.get(b"DP")) {
        Ok(Object::Array(items)) => (0..filter_count)
            .map(|i| items.get(i).cloned().filter(|o| !matches!(o, Object::Null)))
            .collect(),
        Ok(object @ Object::Dictionary(_)) if filter_count > 0 => {
            // A lone dictionary belongs to whichever filter consumes parameters;
            // with a single filter that is unambiguous, and with a chain the
            // predictor always sits on the compressing filter, which is the last.
            let mut slots = vec![None; filter_count];
            slots[filter_count - 1] = Some(object.clone());
            slots
        }
        _ => vec![None; filter_count],
    }
}

/// Apply `filters` in order to `content`. Returns None the moment a filter is
/// one we cannot undo, so the caller leaves the stream alone.
fn decode_chain(content: &[u8], filters: &[Vec<u8>], parms: &[Option<Object>]) -> Option<Vec<u8>> {
    let mut bytes = content.to_vec();
    for (i, filter) in filters.iter().enumerate() {
        bytes = apply_filter(filter, bytes, parms.get(i).and_then(Option::as_ref))?;
        if bytes.len() > MAX_STREAM_BYTES {
            return None;
        }
    }
    Some(bytes)
}

/// Undo one filter. Flate / LZW / ASCII85 are delegated to lopdf via a
/// throwaway single-filter stream, which gets us its predictor handling for
/// free; the two trivial ones it does not implement are done here.
fn apply_filter(name: &[u8], input: Vec<u8>, parms: Option<&Object>) -> Option<Vec<u8>> {
    match name {
        b"ASCIIHexDecode" | b"AHx" => decode_ascii_hex(&input),
        b"RunLengthDecode" | b"RL" => decode_run_length(&input),
        b"FlateDecode" | b"Fl" | b"LZWDecode" | b"LZW" | b"ASCII85Decode" | b"A85" => {
            let mut dict = Dictionary::new();
            dict.set("Filter", Object::Name(name.to_vec()));
            if let Some(p) = parms {
                dict.set("DecodeParms", p.clone());
            }
            lopdf::Stream::new(dict, input)
                .decompressed_content_with_limit(MAX_STREAM_BYTES)
                .ok()
        }
        // Crypt, and anything we have not accounted for.
        _ => None,
    }
}

/// ASCIIHexDecode: hex pairs, whitespace ignored, `>` terminates. An odd
/// trailing digit is padded with 0 per the spec.
fn decode_ascii_hex(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 2);
    let mut high: Option<u8> = None;
    for &byte in input {
        if byte == b'>' {
            break;
        }
        if byte.is_ascii_whitespace() {
            continue;
        }
        let nibble = (byte as char).to_digit(16)? as u8;
        match high.take() {
            None => high = Some(nibble),
            Some(h) => out.push((h << 4) | nibble),
        }
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    Some(out)
}

/// RunLengthDecode: length byte 0..=127 means copy the next n+1 literally,
/// 129..=255 means repeat the next byte 257-n times, 128 ends the data.
fn decode_run_length(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 2);
    let mut i = 0;
    while i < input.len() {
        let length = input[i];
        i += 1;
        match length {
            128 => break,
            0..=127 => {
                let count = length as usize + 1;
                let end = i.checked_add(count)?;
                out.extend_from_slice(input.get(i..end)?);
                i = end;
            }
            _ => {
                let byte = *input.get(i)?;
                i += 1;
                out.extend(std::iter::repeat_n(byte, 257 - length as usize));
            }
        }
        if out.len() > MAX_STREAM_BYTES {
            return None;
        }
    }
    Some(out)
}

/// `/Filter` is either a single name or an array of them.
fn filter_names(dict: &Dictionary) -> Vec<Vec<u8>> {
    match dict.get(b"Filter") {
        Ok(Object::Name(name)) => vec![name.clone()],
        Ok(Object::Array(items)) => items
            .iter()
            .filter_map(|o| match o {
                Object::Name(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Read an integer entry, following a reference if the value is indirect.
fn integer(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<i64> {
    let value = dict.get(key).ok()?;
    let (_, value) = doc.dereference(value).ok()?;
    value.as_i64().ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Orientation, PageSize, PdfBuilder};

    /// A photo-ish JPEG: smooth gradients plus noise, so it behaves like real
    /// image data under re-encoding rather than compressing to nothing.
    fn sample_jpeg(width: u32, height: u32, quality: u8) -> Vec<u8> {
        let mut img = RgbImage::new(width, height);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let noise = ((x * 7 + y * 13) % 32) as u8;
            *px = image::Rgb([
                (x * 255 / width.max(1)) as u8,
                (y * 255 / height.max(1)) as u8,
                128u8.wrapping_add(noise),
            ]);
        }
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
            .write_image(img.as_raw(), width, height, image::ExtendedColorType::Rgb8)
            .expect("encode sample jpeg");
        out
    }

    /// Build a real PDF with the crate's own builder, so the compressor is
    /// exercised against a document we know the exact shape of.
    fn pdf_with_images(count: usize, width: u32, height: u32) -> Vec<u8> {
        let mut builder =
            PdfBuilder::try_new(PageSize::FitToImage, Orientation::Auto, 0.0, 96.0, 0)
                .expect("builder");
        for _ in 0..count {
            builder
                .try_add_image(&sample_jpeg(width, height, 95))
                .expect("add image");
        }
        builder.try_finish().expect("finish")
    }

    fn opts(preset: CompressionPreset) -> CompressOptions {
        CompressOptions::from_preset(preset)
    }

    #[test]
    fn shrinks_a_photo_heavy_pdf() {
        let original = pdf_with_images(3, 900, 700);
        let result = try_compress(&original, &opts(CompressionPreset::Balanced)).unwrap();

        assert!(
            result.compressed_size < result.original_size,
            "expected shrink, got {} -> {}",
            result.original_size,
            result.compressed_size
        );
        assert_eq!(result.page_count, 3);
        assert_eq!(result.images_recompressed, 3);
        assert!(result.saved_ratio() > 0.0);
    }

    #[test]
    fn output_is_still_a_readable_pdf() {
        let original = pdf_with_images(2, 640, 480);
        let mut result = try_compress(&original, &opts(CompressionPreset::Small)).unwrap();
        let bytes = result.take();

        assert!(bytes.starts_with(b"%PDF-"), "missing PDF header");
        let reparsed = Document::load_mem(&bytes).expect("output must re-parse");
        assert_eq!(reparsed.get_pages().len(), 2);
        assert!(
            reparsed.version.as_str() >= "1.5",
            "optimized output should use compact PDF 1.5 object streams"
        );
    }

    #[test]
    fn take_moves_the_buffer_once() {
        let original = pdf_with_images(1, 320, 240);
        let mut result = try_compress(&original, &opts(CompressionPreset::Balanced)).unwrap();
        assert!(!result.take().is_empty());
        assert!(result.take().is_empty(), "second take must not clone");
    }

    #[test]
    fn small_preset_downsamples_oversized_images() {
        // 2000 px longest edge, above Small's 1400 px cap.
        let original = pdf_with_images(1, 2000, 1200);
        let result = try_compress(&original, &opts(CompressionPreset::Small)).unwrap();
        assert_eq!(result.images_downsampled, 1);

        // Quality preset has no cap, so the same input is left at full size.
        let result = try_compress(&original, &opts(CompressionPreset::Quality)).unwrap();
        assert_eq!(result.images_downsampled, 0);
    }

    #[test]
    fn lower_quality_yields_a_smaller_file() {
        let original = pdf_with_images(2, 1000, 800);
        let small = try_compress(&original, &opts(CompressionPreset::Small)).unwrap();
        let quality = try_compress(&original, &opts(CompressionPreset::Quality)).unwrap();
        assert!(
            small.compressed_size < quality.compressed_size,
            "small={} quality={}",
            small.compressed_size,
            quality.compressed_size
        );
    }

    #[test]
    fn never_returns_more_bytes_than_it_was_given() {
        // Already-optimised input: compressing twice must not grow it.
        let original = pdf_with_images(1, 400, 300);
        let mut once = try_compress(&original, &opts(CompressionPreset::Small)).unwrap();
        let once = once.take();
        let twice = try_compress(&once, &opts(CompressionPreset::Small)).unwrap();
        assert!(twice.compressed_size <= once.len() as u32);
    }

    #[test]
    fn explicit_quality_overrides_the_preset() {
        let mut o = CompressOptions::from_preset(CompressionPreset::Quality);
        assert_eq!(o.quality(), 86);
        o.image_quality = 30;
        assert_eq!(o.quality(), 30);
    }

    #[test]
    fn max_edge_sentinel_disables_downsampling() {
        let mut o = CompressOptions::from_preset(CompressionPreset::Small);
        assert_eq!(o.max_edge(), 1400);
        o.max_edge = u32::MAX;
        assert_eq!(o.max_edge(), 0, "u32::MAX means never downsample");
    }

    #[test]
    fn strips_annotations_forms_and_metadata() {
        let original = pdf_with_images(1, 320, 240);
        let options =
            CompressOptions::new(CompressionPreset::Balanced, 0, 0, true, true, true, true);
        let mut result = try_compress(&original, &options).unwrap();
        let doc = Document::load_mem(&result.take()).expect("re-parse");

        assert!(doc.catalog().unwrap().get(b"AcroForm").is_err());
        assert!(doc.trailer.get(b"Info").is_err());
        for page_id in doc.page_iter() {
            let page = doc.get_dictionary(page_id).unwrap();
            assert!(page.get(b"Annots").is_err());
        }
    }

    /// The rungs the app's target-size search walks, mirrored from
    /// TARGET_LADDER in components/workers/pdf.worker.ts.
    ///
    /// Duplicated deliberately: the search only terminates usefully if each
    /// rung really is smaller than the last, and that is a property of THIS
    /// crate, not of the TypeScript. If someone retunes the ladder over there
    /// without checking here, this test is what catches a rung that does not
    /// actually buy anything.
    const APP_TARGET_LADDER: [(u8, u32); 12] = [
        (86, 3200),
        (78, 2600),
        (72, 2000),
        (64, 1750),
        (56, 1500),
        (48, 1300),
        (40, 1100),
        (32, 900),
        (26, 750),
        (20, 600),
        (14, 480),
        (10, 360),
    ];

    #[test]
    fn target_ladder_shrinks_monotonically() {
        // 1700 px exceeds every downsample rung, so all six are exercised.
        // Kept small because these tests run unoptimized and Lanczos is the
        // dominant cost.
        let original = pdf_with_images(2, 1700, 1200);

        let mut previous = u32::MAX;
        let mut sizes = Vec::new();

        for (quality, max_edge) in APP_TARGET_LADDER {
            let options = CompressOptions::new(
                CompressionPreset::Balanced,
                quality,
                max_edge,
                false,
                false,
                false,
                false,
            );
            let size = try_compress(&original, &options).unwrap().compressed_size();
            sizes.push(size);
            assert!(
                size < previous,
                "rung ({quality}, {max_edge}) produced {size}, not smaller than {previous}; \
                 sizes so far: {sizes:?}"
            );
            previous = size;
        }

        // The whole ladder has to be worth walking, not just each step.
        let first = sizes[0];
        let last = *sizes.last().unwrap();
        assert!(
            last * 3 < first,
            "ladder only got {first} -> {last}; too little range to hit a target"
        );
    }

    #[test]
    fn target_ladder_reaches_a_demanding_size() {
        let original = pdf_with_images(2, 1700, 1200);
        let target = 60_000u32;

        let reached = APP_TARGET_LADDER.iter().any(|&(quality, max_edge)| {
            let options = CompressOptions::new(
                CompressionPreset::Balanced,
                quality,
                max_edge,
                false,
                false,
                false,
                false,
            );
            try_compress(&original, &options).unwrap().compressed_size() <= target
        });

        assert!(
            reached,
            "no rung reached {target} bytes — the search would report sizeExceeded \
             for a target a real user would consider reasonable"
        );
    }

    #[test]
    fn rejects_input_that_is_not_a_pdf() {
        let err =
            try_compress(b"not a pdf at all", &opts(CompressionPreset::Balanced)).unwrap_err();
        assert!(err.contains("Could not read"), "unexpected error: {err}");
    }

    #[test]
    fn refuses_cmyk_jpeg_rather_than_shifting_colour() {
        // components_of must report 4 for DeviceCMYK, which is the signal
        // plan_reencode uses to bail out.
        let doc = Document::new();
        let mut dict = Dictionary::new();
        dict.set("ColorSpace", Object::Name(b"DeviceCMYK".to_vec()));
        assert_eq!(components_of(&doc, &dict), Some(4));
    }

    #[test]
    fn converts_effectively_monochrome_rgb_scans_to_one_channel() {
        let width = 80;
        let height = 60;
        let mut pixels = Vec::with_capacity(width * height * 3);
        for index in 0..(width * height) {
            let value = (index % 220) as u8;
            pixels.extend_from_slice(&[value, value.saturating_add(2), value]);
        }

        let mut doc = Document::new();
        let mut dict = Dictionary::new();
        dict.set("Type", Object::Name(b"XObject".to_vec()));
        dict.set("Subtype", Object::Name(b"Image".to_vec()));
        dict.set("Width", Object::Integer(width as i64));
        dict.set("Height", Object::Integer(height as i64));
        dict.set("BitsPerComponent", Object::Integer(8));
        dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
        let id = doc.add_object(Object::Stream(lopdf::Stream::new(dict, pixels)));

        let planned = plan_reencode(&doc, id, 72, 0).expect("RGB scan should re-encode");
        assert!(
            planned.grayscale,
            "redundant RGB channels should be removed"
        );

        let mut colour = RgbImage::new(2, 1);
        colour.put_pixel(0, 0, image::Rgb([120, 122, 119]));
        colour.put_pixel(1, 0, image::Rgb([120, 132, 119]));
        assert!(
            !rgb_is_effectively_grayscale(&colour),
            "a genuinely coloured mark must stay RGB"
        );
    }

    #[test]
    fn refuses_indexed_colour_spaces() {
        let doc = Document::new();
        let mut dict = Dictionary::new();
        dict.set(
            "ColorSpace",
            Object::Array(vec![
                Object::Name(b"Indexed".to_vec()),
                Object::Name(b"DeviceRGB".to_vec()),
                Object::Integer(255),
            ]),
        );
        assert_eq!(components_of(&doc, &dict), None);
    }

    #[test]
    fn reads_component_count_from_iccbased_streams() {
        let mut doc = Document::new();
        let mut icc = Dictionary::new();
        icc.set("N", Object::Integer(3));
        let id = doc.add_object(Object::Stream(lopdf::Stream::new(icc, Vec::new())));

        let mut dict = Dictionary::new();
        dict.set(
            "ColorSpace",
            Object::Array(vec![
                Object::Name(b"ICCBased".to_vec()),
                Object::Reference(id),
            ]),
        );
        assert_eq!(components_of(&doc, &dict), Some(3));
    }

    /// Minimal ASCII85 encoder, test-only — mirrors what real PDF producers do.
    fn encode_ascii85(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in data.chunks(4) {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            let value = u32::from_be_bytes(word);
            if value == 0 && chunk.len() == 4 {
                out.push(b'z');
                continue;
            }
            let mut digits = [0u8; 5];
            let mut v = value;
            for slot in digits.iter_mut().rev() {
                *slot = b'!' + (v % 85) as u8;
                v /= 85;
            }
            out.extend_from_slice(&digits[..chunk.len() + 1]);
        }
        out.extend_from_slice(b"~>");
        out
    }

    /// Regression: reportlab and many other producers emit
    /// `/Filter [/ASCII85Decode /DCTDecode]`. Feeding the still-ASCII85 bytes
    /// to the JPEG decoder silently skipped every such image.
    #[test]
    fn unwraps_transport_filters_before_the_image_codec() {
        let jpeg = sample_jpeg(400, 300, 95);
        let ascii85 = encode_ascii85(&jpeg);

        let mut doc = Document::new();
        let mut dict = Dictionary::new();
        dict.set("Type", Object::Name(b"XObject".to_vec()));
        dict.set("Subtype", Object::Name(b"Image".to_vec()));
        dict.set("Width", Object::Integer(400));
        dict.set("Height", Object::Integer(300));
        dict.set("BitsPerComponent", Object::Integer(8));
        dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
        dict.set(
            "Filter",
            Object::Array(vec![
                Object::Name(b"ASCII85Decode".to_vec()),
                Object::Name(b"DCTDecode".to_vec()),
            ]),
        );
        let id = doc.add_object(Object::Stream(lopdf::Stream::new(dict, ascii85)));

        let planned = plan_reencode(&doc, id, 40, 0);
        assert!(
            planned.is_some(),
            "chained ASCII85 + DCT image must be decodable"
        );
        let planned = planned.unwrap();
        assert_eq!((planned.width, planned.height), (400, 300));
        assert!(
            planned.bytes.starts_with(&[0xFF, 0xD8]),
            "expected JPEG SOI"
        );
    }

    #[test]
    fn leaves_streams_with_a_decode_array_alone() {
        let jpeg = sample_jpeg(64, 64, 90);
        let mut doc = Document::new();
        let mut dict = Dictionary::new();
        dict.set("Subtype", Object::Name(b"Image".to_vec()));
        dict.set("Width", Object::Integer(64));
        dict.set("Height", Object::Integer(64));
        dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
        dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
        // Inverted samples: re-encoding plain RGB would flip the image.
        dict.set(
            "Decode",
            Object::Array(vec![
                Object::Real(1.0),
                Object::Real(0.0),
                Object::Real(1.0),
                Object::Real(0.0),
                Object::Real(1.0),
                Object::Real(0.0),
            ]),
        );
        let id = doc.add_object(Object::Stream(lopdf::Stream::new(dict, jpeg)));
        assert!(plan_reencode(&doc, id, 40, 0).is_none());
    }

    #[test]
    fn skips_codecs_it_cannot_decode() {
        for codec in [
            b"JPXDecode".to_vec(),
            b"JBIG2Decode".to_vec(),
            b"CCITTFaxDecode".to_vec(),
        ] {
            let mut doc = Document::new();
            let mut dict = Dictionary::new();
            dict.set("Subtype", Object::Name(b"Image".to_vec()));
            dict.set("Width", Object::Integer(64));
            dict.set("Height", Object::Integer(64));
            dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
            dict.set("Filter", Object::Name(codec.clone()));
            let id = doc.add_object(Object::Stream(lopdf::Stream::new(dict, vec![0u8; 128])));
            assert!(
                plan_reencode(&doc, id, 40, 0).is_none(),
                "{} must be left alone",
                String::from_utf8_lossy(&codec)
            );
        }
    }

    #[test]
    fn decodes_ascii_hex() {
        assert_eq!(decode_ascii_hex(b"48656C6C6F>").unwrap(), b"Hello");
        // Whitespace ignored, odd trailing digit padded with zero.
        assert_eq!(
            decode_ascii_hex(b"48 65 6C 6C 6F 7>").unwrap(),
            b"Hello\x70"
        );
        assert!(decode_ascii_hex(b"zz>").is_none());
    }

    #[test]
    fn decodes_run_length() {
        // literal run of 3, then "AB" repeated, then EOD
        let input = [2u8, b'x', b'y', b'z', 254, b'A', 128];
        assert_eq!(decode_run_length(&input).unwrap(), b"xyzAAA");
        // truncated literal run must fail rather than panic
        assert!(decode_run_length(&[5u8, b'a']).is_none());
    }

    #[test]
    fn normalises_decode_parms_shapes() {
        let mut dict = Dictionary::new();
        assert_eq!(decode_parms(&dict, 2).len(), 2);

        let mut parms = Dictionary::new();
        parms.set("Predictor", Object::Integer(12));
        dict.set("DecodeParms", Object::Dictionary(parms));
        let slots = decode_parms(&dict, 2);
        // A lone dictionary belongs to the compressing (last) filter.
        assert!(slots[0].is_none());
        assert!(slots[1].is_some());

        dict.set(
            "DecodeParms",
            Object::Array(vec![Object::Null, Object::Dictionary(Dictionary::new())]),
        );
        let slots = decode_parms(&dict, 2);
        assert!(slots[0].is_none(), "Null must normalise to None");
        assert!(slots[1].is_some());
    }

    #[test]
    fn parses_filter_arrays_and_single_names() {
        let mut dict = Dictionary::new();
        dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
        assert_eq!(filter_names(&dict), vec![b"DCTDecode".to_vec()]);

        dict.set(
            "Filter",
            Object::Array(vec![
                Object::Name(b"ASCII85Decode".to_vec()),
                Object::Name(b"FlateDecode".to_vec()),
            ]),
        );
        assert_eq!(filter_names(&dict).last().unwrap(), b"FlateDecode");
    }
}
