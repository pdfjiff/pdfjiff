use lopdf::{Document, Object};
use serde::Serialize;
use std::fmt;

/// Stable error categories for callers. Messages are for people, not matching.
#[derive(Debug)]
pub enum CoreError {
    InvalidInput(String),
    PasswordRequired,
    Processing(String),
}
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) | Self::Processing(message) => f.write_str(message),
            Self::PasswordRequired => f.write_str(
                "This PDF needs a password. Decrypt a copy before using this prerelease.",
            ),
        }
    }
}
impl std::error::Error for CoreError {}

#[derive(Debug, Serialize)]
pub struct PageGeometry {
    pub width_pt: f32,
    pub height_pt: f32,
    pub rotation: i32,
}
#[derive(Debug, Serialize)]
pub struct Inspection {
    /// Unknown until decrypted when encryption prevents reading page structure.
    pub page_count: Option<u32>,
    pub pages: Vec<PageGeometry>,
    pub encrypted: bool,
    pub signatures_detected: bool,
    /// Document-level structures the current page assembler cannot preserve.
    pub assembly_sensitive_features: Vec<String>,
}

/// Inspect structure only; this does not render pages or validate signatures.
pub fn inspect(data: &[u8]) -> Result<Inspection, CoreError> {
    if !data.starts_with(b"%PDF-") {
        return Err(CoreError::InvalidInput(
            "Input does not start with a PDF header.".into(),
        ));
    }
    let doc = Document::load_mem(data)
        .map_err(|e| CoreError::InvalidInput(format!("Could not read PDF: {e}")))?;
    let encrypted = doc.is_encrypted();
    let signatures_detected = doc.objects.values().any(|object| {
        object.as_dict().is_ok_and(|dict| {
            dict.get(b"FT")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"Sig")
                || (dict.has(b"ByteRange") && dict.has(b"Contents"))
        })
    });
    let mut features = Vec::new();
    if let Ok(catalog) = doc.catalog() {
        for (key, label) in [
            (b"Outlines".as_slice(), "bookmarks"),
            (b"AcroForm".as_slice(), "forms"),
            (b"Names".as_slice(), "named destinations or embedded files"),
            (b"Dests".as_slice(), "destinations"),
            (b"StructTreeRoot".as_slice(), "accessibility tags"),
            (b"PageLabels".as_slice(), "page labels"),
            (b"Metadata".as_slice(), "document metadata"),
            (b"OpenAction".as_slice(), "document actions"),
            (b"OCProperties".as_slice(), "optional content layers"),
            (b"OutputIntents".as_slice(), "output color profiles"),
        ] {
            if catalog.has(key) {
                features.push(label.to_owned());
            }
        }
    }
    if encrypted {
        return Ok(Inspection {
            page_count: None,
            pages: vec![],
            encrypted,
            signatures_detected,
            assembly_sensitive_features: features,
        });
    }
    if doc.get_pages().values().any(|id| {
        doc.get_object(*id)
            .and_then(Object::as_dict)
            .is_ok_and(|page| page.has(b"Annots"))
    }) {
        features.push("page annotations or links".to_owned());
    }
    let info = crate::pages::read_info_document(&doc);
    if info.page_count() == 0 {
        return Err(CoreError::InvalidInput("PDF has no readable pages.".into()));
    }
    let pages = info
        .widths()
        .into_iter()
        .zip(info.heights())
        .zip(info.rotations())
        .map(|((width_pt, height_pt), rotation)| PageGeometry {
            width_pt,
            height_pt,
            rotation,
        })
        .collect();
    Ok(Inspection {
        page_count: Some(info.page_count()),
        pages,
        encrypted,
        signatures_detected,
        assembly_sensitive_features: features,
    })
}
