//! Native PDF primitives behind the `pdfjiff` command-line tool: structural
//! inspection, image recompression and page assembly. Pure Rust, offline, with
//! no application policy or browser dependencies.
//!
//! ```no_run
//! use pdfjiff_core::compress::{try_compress, CompressOptions, CompressionPreset};
//! use pdfjiff_core::pages::PdfMerger;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let input = std::fs::read("report.pdf")?;
//! let info = pdfjiff_core::inspect(&input)?;
//! println!("{:?} pages, encrypted: {}", info.page_count, info.encrypted);
//!
//! let options = CompressOptions::from_preset(CompressionPreset::Balanced);
//! let mut result = try_compress(&input, &options)?;
//! println!("{} -> {} bytes", input.len(), result.compressed_size());
//! std::fs::write("report-small.pdf", result.take())?;
//!
//! let mut merger = PdfMerger::try_new()?;
//! for path in ["cover.pdf", "report-small.pdf"] {
//!     merger.try_add_document(&std::fs::read(path)?)?;
//! }
//! std::fs::write("combined.pdf", merger.try_finish()?)?;
//! # Ok(())
//! # }
//! ```
//!
//! The API is pre-1.0: expect breaking changes between minor versions.
mod builder;
pub mod compress;
mod inspect;
pub mod pages;
pub use builder::{Orientation, PageSize, PdfBuilder};
pub use inspect::{inspect, CoreError, Inspection, PageGeometry};
