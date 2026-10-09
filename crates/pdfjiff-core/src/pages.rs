//! Page-tree operations: merge, and a single primitive that covers reordering,
//! deleting, duplicating, extracting and rotating.
//!
//! Deliberately one primitive rather than five entry points. "Build a new
//! document from this list of source pages, in this order, with these
//! rotations" is what merge-pdf, split-pdf, organize-pages, rotate-pdf,
//! extract-pages and delete-pages all reduce to, and one implementation means
//! one place where page-tree correctness has to be right.
//!
//! The correctness that matters here is **inheritance**. A page can omit
//! `/Resources`, `/MediaBox`, `/CropBox` or `/Rotate` and inherit them from an
//! ancestor `/Pages` node. Rebuilding the tree flat — which is what any of
//! these operations does — orphans those ancestors, so every inherited
//! attribute has to be materialised onto the page first or pages silently lose
//! their size, rotation, or every font and image they draw with.

use std::collections::{BTreeMap, BTreeSet};

use lopdf::{Dictionary, Document, Object, ObjectId};

/// Attributes a page may inherit from an ancestor node in the page tree.
const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// Depth bound while walking `/Parent`. A malformed document can point a node
/// at itself; without this the walk never returns.
const MAX_TREE_DEPTH: usize = 64;

/// Refuse absurd page counts rather than letting a malformed selection allocate
/// without bound. Well beyond any real document.
const MAX_PAGES: usize = 50_000;

// ---------------------------------------------------------------------------
// Document info
// ---------------------------------------------------------------------------

/// Per-page geometry, so callers can lay out thumbnails and show what it is
/// about to change before anything is rewritten.
#[derive(Debug, Default)]
pub struct PdfInfo {
    page_count: u32,
    widths: Vec<f32>,
    heights: Vec<f32>,
    rotations: Vec<i32>,
}

impl PdfInfo {
    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    /// MediaBox widths in points, in page order. Not swapped for rotation —
    /// the caller gets the raw box and the angle and decides.
    pub fn widths(&self) -> Vec<f32> {
        self.widths.clone()
    }

    pub fn heights(&self) -> Vec<f32> {
        self.heights.clone()
    }

    /// Effective `/Rotate` per page, normalised to 0, 90, 180 or 270.
    pub fn rotations(&self) -> Vec<i32> {
        self.rotations.clone()
    }
}

/// Read page count and per-page geometry without rewriting anything.
pub fn try_read_info(data: &[u8]) -> Result<PdfInfo, String> {
    let doc = open_document(data)?;
    Ok(read_info_document(&doc))
}

pub(crate) fn read_info_document(doc: &Document) -> PdfInfo {
    let pages = doc.get_pages();

    let mut info = PdfInfo {
        page_count: pages.len() as u32,
        ..Default::default()
    };

    for &page_id in pages.values() {
        let dict = materialise_inherited(doc, page_id);
        let (width, height) = media_box_size(doc, &dict).unwrap_or((595.276, 841.89));
        info.widths.push(width);
        info.heights.push(height);
        info.rotations.push(normalise_rotation(
            integer(doc, &dict, b"Rotate").unwrap_or(0),
        ));
    }

    info
}

// ---------------------------------------------------------------------------
// Organize: the one primitive
// ---------------------------------------------------------------------------

/// Build a new document from selected pages of `data`.
///
/// * `pages` — 1-based source page numbers, in output order. May repeat a page
///   (duplicate it) and may omit pages (delete them). An empty list is an
///   error, not an empty PDF: a zero-page PDF opens in nothing.
/// * `rotations` — degrees to add to each selected page, parallel to `pages`.
///   Pass an empty slice for no rotation. Values are added to the page's
///   existing `/Rotate` and normalised, so 90 means "a quarter turn from where
///   it is now", not "set to 90".
pub fn try_organize(data: &[u8], pages: &[u32], rotations: &[i32]) -> Result<Vec<u8>, String> {
    if pages.is_empty() {
        return Err("Select at least one page.".to_string());
    }
    if pages.len() > MAX_PAGES {
        return Err("That is more pages than this tool can assemble.".to_string());
    }

    let mut doc = open_document(data)?;
    let source = doc.get_pages();

    // Resolve and materialise first, while the immutable borrow is fine, so a
    // bad page number fails before anything has been rewritten.
    let mut prepared: Vec<Dictionary> = Vec::with_capacity(pages.len());
    for (index, &number) in pages.iter().enumerate() {
        let page_id = *source
            .get(&number)
            .ok_or_else(|| format!("This PDF has no page {number}."))?;

        let mut dict = materialise_inherited(&doc, page_id);
        let extra = rotations.get(index).copied().unwrap_or(0);
        let current = integer(&doc, &dict, b"Rotate").unwrap_or(0);
        let rotate = normalise_rotation(current + i64::from(extra));

        if rotate == 0 {
            dict.remove(b"Rotate");
        } else {
            dict.set("Rotate", Object::Integer(i64::from(rotate)));
        }
        // Set below, once the new tree root exists.
        dict.remove(b"Parent");
        prepared.push(dict);
    }

    rebuild_page_tree(&mut doc, prepared)?;
    save_document(&mut doc)
}

/// Replace the document's page tree with a flat one containing exactly
/// `pages`, then drop everything the new tree no longer reaches.
fn rebuild_page_tree(doc: &mut Document, pages: Vec<Dictionary>) -> Result<(), String> {
    let tree_id = doc.new_object_id();

    let mut kids = Vec::with_capacity(pages.len());
    for mut dict in pages {
        dict.set("Type", Object::Name(b"Page".to_vec()));
        dict.set("Parent", Object::Reference(tree_id));
        kids.push(Object::Reference(doc.add_object(Object::Dictionary(dict))));
    }

    let count = kids.len() as i64;
    let mut tree = Dictionary::new();
    tree.set("Type", Object::Name(b"Pages".to_vec()));
    tree.set("Count", Object::Integer(count));
    tree.set("Kids", Object::Array(kids));
    doc.objects.insert(tree_id, Object::Dictionary(tree));

    let catalog_id = catalog_id(doc)?;
    let catalog = doc
        .get_object_mut(catalog_id)
        .and_then(|o| o.as_dict_mut())
        .map_err(|e| format!("This PDF has no usable catalog: {e}"))?;
    catalog.set("Pages", Object::Reference(tree_id));
    // Outlines and named destinations point at pages that may no longer exist.
    // A dangling destination makes some readers refuse the whole file, and we
    // cannot remap them meaningfully once pages have been dropped or reordered.
    catalog.remove(b"Outlines");
    catalog.remove(b"Names");
    catalog.remove(b"Dests");
    catalog.remove(b"StructTreeRoot");
    catalog.remove(b"PageLabels");

    doc.prune_objects();
    doc.renumber_objects();
    Ok(())
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

/// Incremental merge, so callers can feed documents one at a time and release
/// each source buffer as soon as [`try_add_document`](Self::try_add_document)
/// returns — peak memory is the growing output plus one input, not the whole
/// batch.
pub struct PdfMerger {
    doc: Document,
    /// Page dictionaries collected so far, already materialised and detached
    /// from their original trees.
    pages: Vec<Dictionary>,
}

impl PdfMerger {
    pub fn page_count(&self) -> u32 {
        self.pages.len() as u32
    }

    pub fn try_new() -> Result<PdfMerger, String> {
        let mut doc = Document::with_version("1.7");
        // A catalog has to exist before pages can point at it; the page tree is
        // attached in `try_finish`.
        let catalog_id = doc.new_object_id();
        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        doc.objects.insert(catalog_id, Object::Dictionary(catalog));
        doc.trailer.set("Root", Object::Reference(catalog_id));

        Ok(PdfMerger {
            doc,
            pages: Vec::new(),
        })
    }

    pub fn try_add_document(&mut self, data: &[u8]) -> Result<(), String> {
        let source = open_document(data)?;
        let source_pages = source.get_pages();

        if self.pages.len() + source_pages.len() > MAX_PAGES {
            return Err("That is more pages than this tool can assemble.".to_string());
        }

        // Every id from this document is shifted clear of everything already
        // merged, and all references inside are rewritten to match.
        let offset = self.doc.max_id;
        let mut highest = offset;

        for (id, object) in &source.objects {
            let new_id = (id.0 + offset, id.1);
            highest = highest.max(new_id.0);
            self.doc
                .objects
                .insert(new_id, shift_references(object, offset));
        }
        self.doc.max_id = highest;

        for &page_id in source_pages.values() {
            // Materialise against the SOURCE document, where the parent chain
            // still exists, then detach.
            let mut dict = materialise_inherited(&source, page_id);
            dict.remove(b"Parent");
            self.pages.push(shift_dictionary(&dict, offset));
        }

        Ok(())
    }

    pub fn try_finish(mut self) -> Result<Vec<u8>, String> {
        if self.pages.is_empty() {
            return Err("Add at least one PDF with pages in it.".to_string());
        }
        let pages = std::mem::take(&mut self.pages);
        rebuild_page_tree(&mut self.doc, pages)?;
        save_document(&mut self.doc)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[doc(hidden)]
pub fn open_document(data: &[u8]) -> Result<Document, String> {
    let doc = Document::load_mem(data).map_err(|e| format!("Could not read this PDF: {e}"))?;
    if doc.is_encrypted() {
        return Err("This PDF is password-protected. Remove the password first.".to_string());
    }
    Ok(doc)
}

#[doc(hidden)]
pub fn save_document(doc: &mut Document) -> Result<Vec<u8>, String> {
    doc.compress();
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| format!("Could not write the PDF: {e}"))?;
    Ok(out)
}

fn catalog_id(doc: &Document) -> Result<ObjectId, String> {
    match doc.trailer.get(b"Root") {
        Ok(Object::Reference(id)) => Ok(*id),
        _ => Err("This PDF has no document catalog.".to_string()),
    }
}

/// Copy any inheritable attribute the page does not define onto a detached copy
/// of its dictionary, by walking `/Parent`.
#[doc(hidden)]
pub fn materialise_inherited(doc: &Document, page_id: ObjectId) -> Dictionary {
    let mut dict = match doc.get_dictionary(page_id) {
        Ok(d) => d.clone(),
        Err(_) => Dictionary::new(),
    };

    let mut current = dict.get(b"Parent").ok().cloned();
    let mut seen = BTreeSet::new();
    let mut depth = 0;

    while let Some(Object::Reference(parent_id)) = current {
        // A self-referential or cyclic /Parent chain is malformed but does
        // occur; bound the walk both ways.
        if depth >= MAX_TREE_DEPTH || !seen.insert(parent_id) {
            break;
        }
        depth += 1;

        let Ok(parent) = doc.get_dictionary(parent_id) else {
            break;
        };
        for key in INHERITABLE {
            if !dict.has(key) {
                if let Ok(value) = parent.get(key) {
                    dict.set(String::from_utf8_lossy(key).into_owned(), value.clone());
                }
            }
        }
        current = parent.get(b"Parent").ok().cloned();
    }

    dict
}

/// Rewrite every reference inside an object by `offset`.
fn shift_references(object: &Object, offset: u32) -> Object {
    match object {
        Object::Reference((id, gen)) => Object::Reference((id + offset, *gen)),
        Object::Array(items) => {
            Object::Array(items.iter().map(|o| shift_references(o, offset)).collect())
        }
        Object::Dictionary(dict) => Object::Dictionary(shift_dictionary(dict, offset)),
        Object::Stream(stream) => {
            let mut copy = stream.clone();
            copy.dict = shift_dictionary(&stream.dict, offset);
            Object::Stream(copy)
        }
        other => other.clone(),
    }
}

fn shift_dictionary(dict: &Dictionary, offset: u32) -> Dictionary {
    let mut out = Dictionary::new();
    for (key, value) in dict.iter() {
        out.set(
            String::from_utf8_lossy(key).into_owned(),
            shift_references(value, offset),
        );
    }
    out
}

/// `/Rotate` must be a multiple of 90. Negative angles are legal in the spec
/// and common in the wild, so normalise into 0..360 rather than assuming.
fn normalise_rotation(degrees: i64) -> i32 {
    let snapped = (degrees as f64 / 90.0).round() as i64 * 90;
    (((snapped % 360) + 360) % 360) as i32
}

fn media_box_size(doc: &Document, dict: &Dictionary) -> Option<(f32, f32)> {
    let value = dict.get(b"MediaBox").ok()?;
    let (_, value) = doc.dereference(value).ok()?;
    let items = value.as_array().ok()?;
    if items.len() < 4 {
        return None;
    }

    let mut numbers = [0f32; 4];
    for (slot, item) in numbers.iter_mut().zip(items) {
        let (_, item) = doc.dereference(item).ok()?;
        *slot = item.as_float().ok()?;
    }
    // The box is [x0 y0 x1 y1] but the corners are not guaranteed to be in
    // that order, so take absolute extents.
    Some((
        (numbers[2] - numbers[0]).abs(),
        (numbers[3] - numbers[1]).abs(),
    ))
}

fn integer(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<i64> {
    let value = dict.get(key).ok()?;
    let (_, value) = doc.dereference(value).ok()?;
    value.as_i64().ok()
}

/// Unused today, kept because `prune_objects` needs a reachable set when the
/// caller wants to keep something the tree does not point at.
#[allow(dead_code)]
fn object_ids(objects: &BTreeMap<ObjectId, Object>) -> Vec<ObjectId> {
    objects.keys().copied().collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Orientation, PageSize, PdfBuilder};
    use image::{ImageEncoder, RgbImage};

    /// A PDF whose pages are visually distinguishable by their aspect ratio, so
    /// a reordering test can tell which page ended up where.
    fn pdf_with_sizes(sizes: &[(u32, u32)]) -> Vec<u8> {
        let mut builder =
            PdfBuilder::try_new(PageSize::FitToImage, Orientation::Auto, 0.0, 72.0, 0)
                .expect("builder");
        for &(w, h) in sizes {
            builder.try_add_image(&jpeg(w, h)).expect("add image");
        }
        builder.try_finish().expect("finish")
    }

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut img = RgbImage::new(width, height);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]);
        }
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
            .write_image(img.as_raw(), width, height, image::ExtendedColorType::Rgb8)
            .expect("encode");
        out
    }

    fn info(bytes: &[u8]) -> PdfInfo {
        try_read_info(bytes).expect("read info")
    }

    #[test]
    fn reads_page_count_and_geometry() {
        let pdf = pdf_with_sizes(&[(400, 300), (300, 400), (500, 500)]);
        let read = info(&pdf);

        assert_eq!(read.page_count(), 3);
        assert_eq!(read.widths().len(), 3);
        // FitToImage at 72 dpi maps 1 px to 1 pt.
        assert!((read.widths()[0] - 400.0).abs() < 1.0);
        assert!((read.heights()[0] - 300.0).abs() < 1.0);
        assert!((read.widths()[1] - 300.0).abs() < 1.0);
        assert_eq!(read.rotations(), vec![0, 0, 0]);
    }

    #[test]
    fn reorders_pages() {
        let pdf = pdf_with_sizes(&[(400, 300), (300, 400), (500, 500)]);
        let out = try_organize(&pdf, &[3, 1, 2], &[]).unwrap();
        let read = info(&out);

        assert_eq!(read.page_count(), 3);
        // 500-wide page first, then the 400, then the 300.
        assert!((read.widths()[0] - 500.0).abs() < 1.0);
        assert!((read.widths()[1] - 400.0).abs() < 1.0);
        assert!((read.widths()[2] - 300.0).abs() < 1.0);
    }

    #[test]
    fn deletes_pages_by_omission() {
        let pdf = pdf_with_sizes(&[(400, 300), (300, 400), (500, 500)]);
        let out = try_organize(&pdf, &[1, 3], &[]).unwrap();
        let read = info(&out);

        assert_eq!(read.page_count(), 2);
        assert!((read.widths()[1] - 500.0).abs() < 1.0);
    }

    #[test]
    fn extracts_a_single_page() {
        let pdf = pdf_with_sizes(&[(400, 300), (300, 400), (500, 500)]);
        let out = try_organize(&pdf, &[2], &[]).unwrap();
        assert_eq!(info(&out).page_count(), 1);
        assert!((info(&out).widths()[0] - 300.0).abs() < 1.0);
    }

    #[test]
    fn duplicates_a_page() {
        let pdf = pdf_with_sizes(&[(400, 300)]);
        let out = try_organize(&pdf, &[1, 1, 1], &[]).unwrap();
        assert_eq!(info(&out).page_count(), 3);
        // A duplicated page must be a real page, not a second reference to one:
        // the output has to re-parse and report three.
        Document::load_mem(&out).expect("output must re-parse");
    }

    #[test]
    fn rotates_relative_to_the_current_angle() {
        let pdf = pdf_with_sizes(&[(400, 300), (300, 400)]);

        let once = try_organize(&pdf, &[1, 2], &[90, 0]).unwrap();
        assert_eq!(info(&once).rotations(), vec![90, 0]);

        // Rotation is additive, so applying 90 again lands on 180 rather than
        // resetting to 90.
        let twice = try_organize(&once, &[1, 2], &[90, 270]).unwrap();
        assert_eq!(info(&twice).rotations(), vec![180, 270]);
    }

    #[test]
    fn normalises_odd_rotations() {
        assert_eq!(normalise_rotation(0), 0);
        assert_eq!(normalise_rotation(90), 90);
        assert_eq!(normalise_rotation(360), 0);
        assert_eq!(normalise_rotation(450), 90);
        // Negative angles are legal in the spec and common in the wild.
        assert_eq!(normalise_rotation(-90), 270);
        assert_eq!(normalise_rotation(-450), 270);
        // Not a multiple of 90: snap to the nearest quarter turn.
        assert_eq!(normalise_rotation(89), 90);
        assert_eq!(normalise_rotation(46), 90);
        assert_eq!(normalise_rotation(44), 0);
    }

    #[test]
    fn rejects_an_empty_selection() {
        let pdf = pdf_with_sizes(&[(400, 300)]);
        let err = try_organize(&pdf, &[], &[]).unwrap_err();
        assert!(err.contains("at least one page"), "got: {err}");
    }

    #[test]
    fn rejects_a_page_that_does_not_exist() {
        let pdf = pdf_with_sizes(&[(400, 300)]);
        let err = try_organize(&pdf, &[1, 7], &[]).unwrap_err();
        assert!(err.contains("no page 7"), "got: {err}");
    }

    #[test]
    fn merges_two_documents_in_order() {
        let first = pdf_with_sizes(&[(400, 300), (300, 400)]);
        let second = pdf_with_sizes(&[(500, 500)]);

        let mut merger = PdfMerger::try_new().unwrap();
        merger.try_add_document(&first).unwrap();
        merger.try_add_document(&second).unwrap();
        assert_eq!(merger.page_count(), 3);

        let out = merger.try_finish().unwrap();
        let read = info(&out);

        assert_eq!(read.page_count(), 3);
        assert!((read.widths()[0] - 400.0).abs() < 1.0);
        assert!((read.widths()[1] - 300.0).abs() < 1.0);
        assert!((read.widths()[2] - 500.0).abs() < 1.0);
    }

    #[test]
    fn merged_pages_keep_their_own_resources() {
        // Two documents merged into one must not collide on object ids — the
        // classic merge bug, where the second document's pages end up drawing
        // the first document's images.
        let first = pdf_with_sizes(&[(400, 300)]);
        let second = pdf_with_sizes(&[(500, 500)]);

        let mut merger = PdfMerger::try_new().unwrap();
        merger.try_add_document(&first).unwrap();
        merger.try_add_document(&second).unwrap();
        let out = merger.try_finish().unwrap();

        let doc = Document::load_mem(&out).expect("re-parse");
        let page_ids: Vec<ObjectId> = doc.page_iter().collect();
        assert_eq!(page_ids.len(), 2);

        let resource_ids: Vec<_> = page_ids
            .iter()
            .map(|&id| {
                let dict = doc.get_dictionary(id).expect("page dict");
                format!("{:?}", dict.get(b"Resources").expect("resources"))
            })
            .collect();
        assert_ne!(
            resource_ids[0], resource_ids[1],
            "merged pages must not share one document's resources"
        );
    }

    #[test]
    fn merging_nothing_is_an_error_not_an_empty_pdf() {
        let merger = PdfMerger::try_new().unwrap();
        let err = merger.try_finish().unwrap_err();
        assert!(err.contains("at least one"), "got: {err}");
    }

    #[test]
    fn merge_output_reparses_and_survives_a_second_pass() {
        let a = pdf_with_sizes(&[(400, 300), (300, 400)]);
        let b = pdf_with_sizes(&[(500, 500)]);

        let mut merger = PdfMerger::try_new().unwrap();
        merger.try_add_document(&a).unwrap();
        merger.try_add_document(&b).unwrap();
        let merged = merger.try_finish().unwrap();

        // Feeding our own output back in is the cheapest check that the tree we
        // wrote is one we can also read.
        let reordered = try_organize(&merged, &[3, 2, 1], &[180, 90, 0]).unwrap();
        let read = info(&reordered);
        assert_eq!(read.page_count(), 3);
        assert_eq!(read.rotations(), vec![180, 90, 0]);
        assert!((read.widths()[0] - 500.0).abs() < 1.0);
    }

    #[test]
    fn inherited_attributes_survive_a_rebuild() {
        // Build a document whose page inherits MediaBox and Resources from the
        // Pages node, which is exactly what a flat rebuild would otherwise drop.
        let mut doc = Document::with_version("1.7");

        let content_id = doc.add_object(Object::Stream(lopdf::Stream::new(
            Dictionary::new(),
            b"BT ET".to_vec(),
        )));

        let tree_id = doc.new_object_id();
        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set("Parent", Object::Reference(tree_id));
        page.set("Contents", Object::Reference(content_id));
        // Deliberately NO MediaBox, NO Resources, NO Rotate on the page.
        let page_id = doc.add_object(Object::Dictionary(page));

        let mut resources = Dictionary::new();
        resources.set(
            "ProcSet",
            Object::Array(vec![Object::Name(b"PDF".to_vec())]),
        );

        let mut tree = Dictionary::new();
        tree.set("Type", Object::Name(b"Pages".to_vec()));
        tree.set("Count", Object::Integer(1));
        tree.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        tree.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(612),
                Object::Integer(792),
            ]),
        );
        tree.set("Rotate", Object::Integer(90));
        tree.set("Resources", Object::Dictionary(resources));
        doc.objects.insert(tree_id, Object::Dictionary(tree));

        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(tree_id));
        let catalog_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", Object::Reference(catalog_id));

        let mut source = Vec::new();
        doc.save_to(&mut source).expect("save fixture");

        let out = try_organize(&source, &[1], &[]).unwrap();
        let rebuilt = Document::load_mem(&out).expect("re-parse");
        let page_id = rebuilt.page_iter().next().expect("a page");
        let dict = rebuilt.get_dictionary(page_id).expect("page dict");

        assert!(dict.has(b"MediaBox"), "inherited MediaBox was lost");
        assert!(dict.has(b"Resources"), "inherited Resources were lost");
        assert_eq!(
            info(&out).rotations(),
            vec![90],
            "inherited Rotate was lost"
        );
    }

    #[test]
    fn survives_a_cyclic_parent_chain() {
        // Malformed but seen in the wild: a Pages node whose Parent is itself.
        let mut doc = Document::with_version("1.7");
        let tree_id = doc.new_object_id();

        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set("Parent", Object::Reference(tree_id));
        let page_id = doc.add_object(Object::Dictionary(page));

        let mut tree = Dictionary::new();
        tree.set("Type", Object::Name(b"Pages".to_vec()));
        tree.set("Count", Object::Integer(1));
        tree.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        tree.set("Parent", Object::Reference(tree_id)); // the cycle
        tree.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(200),
                Object::Integer(100),
            ]),
        );
        doc.objects.insert(tree_id, Object::Dictionary(tree));

        // Should terminate rather than hang, and still pick up the MediaBox.
        let dict = materialise_inherited(&doc, page_id);
        assert!(dict.has(b"MediaBox"));
    }
}
