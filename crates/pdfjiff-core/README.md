# pdfjiff-core

Native, pure-Rust PDF primitives behind the [`pdfjiff`](https://github.com/pdfjiff/pdfjiff) command-line tool:

- **Inspection:** page count, page geometry and rotation, encryption, detected signatures and document-level structures
- **Compression:** image recompression with quality/size presets, lossless structural optimization and downsampling. The result is returned only when it is smaller than the input.
- **Assembly:** incremental merging, page selection and rotation, and building PDFs from images

It makes no network calls and has no C dependencies, and it contains no application or browser policy.

```toml
[dependencies]
pdfjiff-core = "0.1"
```

```rust
use pdfjiff_core::compress::{try_compress, CompressOptions, CompressionPreset};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::fs::read("report.pdf")?;
    let info = pdfjiff_core::inspect(&input)?;
    println!("{:?} pages, encrypted: {}", info.page_count, info.encrypted);

    let options = CompressOptions::from_preset(CompressionPreset::Balanced);
    let mut result = try_compress(&input, &options)?;
    println!("{} -> {} bytes", input.len(), result.compressed_size());
    std::fs::write("report-small.pdf", result.take())?;
    Ok(())
}
```

The API is pre-1.0 and may change between minor versions. The full API is documented on [docs.rs](https://docs.rs/pdfjiff-core).

Licensed under either of [Apache-2.0](https://github.com/pdfjiff/pdfjiff/blob/main/LICENSE-APACHE) or [MIT](https://github.com/pdfjiff/pdfjiff/blob/main/LICENSE-MIT), at your option.
