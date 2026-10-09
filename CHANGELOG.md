# Changelog

All notable changes to PDFJiff are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, minor versions may contain breaking changes. Any breaking change is called out explicitly.

The release workflow publishes the section that matches the tag as the GitHub release notes. Set its date before tagging.

## [Unreleased]

## [0.1.0] - Unreleased

First public release.

### Added

- `pdfjiff inspect`: page count, per-page geometry and rotation, encryption, detected signatures and document-level structures.
- `pdfjiff compress`: `quality`, `balanced` and `small` presets; `--lossless` structural optimization; `--target` bounded size search of up to six attempts, with no silent rasterization.
- `pdfjiff merge`: merges in argument order and reports any structures it cannot preserve.
- `pdfjiff capabilities`: machine-readable list of supported operations and known limitations.
- `pdfjiff completions`: shell completions for bash, zsh, fish, PowerShell and elvish.
- `--json` output: one versioned result (`schema_version: 1`) with stable error codes, recovery hints and documented exit codes.
- Safety defaults: inputs are never modified, overwriting requires `--overwrite`, outputs are published atomically, and `--dry-run` is available. Rewriting a signed PDF or dropping document structures requires explicit acknowledgement. Input byte limits and Ctrl-C cancellation are supported.
- `pdfjiff-core` library crate with inspection, compression, merging and image-to-PDF building primitives.
- Prebuilt binaries for macOS, Linux and Windows on x86-64 and ARM64. Distribution through install scripts, Homebrew, Scoop, crates.io (`cargo install` / `cargo binstall`), a Docker image and a GitHub Action.

[Unreleased]: https://github.com/pdfjiff/pdfjiff/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pdfjiff/pdfjiff/releases/tag/v0.1.0
