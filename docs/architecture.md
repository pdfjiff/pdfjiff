# Architecture

PDFJiff is a Cargo workspace with two crates and no server component.

```text
            ┌──────────────────────────── pdfjiff-cli ─────────────────────────────┐
 argv ────▶ │ args.rs      clap definitions, size parsing, presets                 │
            │ main.rs      one function per command, JSON envelope, exit codes     │
            │ files.rs     input limits, alias checks, atomic output publication   │
            │ error.rs     stable error codes + hints                              │
            └───────────────┬──────────────────────────────────────────────────────┘
                            │ &[u8] in, Vec<u8> out (no file system access)
            ┌───────────────▼──────────── pdfjiff-core ────────────────────────────┐
            │ inspect.rs   structure, encryption, signatures, preservation risks   │
            │ compress.rs  image recompression / downsampling, lossless rewrite    │
            │ pages.rs     page info, organize (select/rotate), incremental merge  │
            │ builder.rs   images → PDF                                            │
            └──────────────────────────────────────────────────────────────────────┘
                 lopdf (parse/write) · pdf-writer · image (png/jpeg/webp) · miniz_oxide
```

## Design principles

- **The core is pure.** `pdfjiff-core` turns bytes into bytes. It never touches the file system, reads the environment, accesses the network or prints. That makes it easy to embed, test and fuzz, and it is why the same algorithms can run in other hosts, including WebAssembly.
- **The CLI owns policy.** Byte limits, overwrite rules, alias detection, atomic writes, cancellation, JSON and exit codes all live in `pdfjiff-cli`, so every command gets them automatically.
- **Nothing is claimed that wasn't verified.** The JSON `coverage` block says what the run did not check. `capabilities` lists what a build supports and what it does not.
- **Smaller or unchanged.** Compression returns a re-encoded image or document only when it is actually smaller. Target-size search is bounded (at most six attempts) and never rasterizes pages.

## Data flow of a write command

1. Parse arguments. On error, return `INVALID_ARGUMENT` (exit 2) as JSON when `--json` is set.
2. `Destination::prepare`: resolve the output folder, refuse outputs that are an input (including links), refuse existing outputs without `--overwrite`, and create a temporary file next to the destination.
3. `read_input`: read at most `--max-input-mib`, and fail before parsing if the file is larger.
4. `inspect`: reject encrypted inputs, require acknowledgements for signatures or structure loss, and stop here for `--dry-run`.
5. Run the core algorithm. The cancellation flag is checked between stages.
6. Check the output again. For example, the page count must match.
7. `Destination::publish`: write the file, `fsync`, and rename it into place (without replacing an existing file unless `--overwrite` was given).

## Platforms

Release binaries are built for macOS, Linux (static musl) and Windows on x86-64 and ARM64. CI runs the full test suite on Linux, macOS and Windows for every change, and smoke-tests each native release binary.

## Adding functionality

New algorithms go in `pdfjiff-core` with unit tests. New commands follow the checklist in [CONTRIBUTING.md](../CONTRIBUTING.md#adding-a-command). Keep everything local and offline; server or UI components, if they come, will be separate crates on top of the same core.
