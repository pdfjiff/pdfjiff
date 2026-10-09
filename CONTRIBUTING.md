# Contributing to PDFJiff

Thanks for helping. All kinds of contributions are useful, including a bug report with a clear reproduction, a docs fix, a test for an unusual PDF or a new command.

This guide covers:

- [Ways to contribute](#ways-to-contribute)
- [Set up in a few minutes](#set-up-in-a-few-minutes)
- [Project layout](#project-layout)
- [Everyday commands](#everyday-commands)
- [Tests and sample PDFs](#tests-and-sample-pdfs)
- [Guidelines](#guidelines)
- [Adding a command](#adding-a-command)
- [Pull requests](#pull-requests)
- [How changes flow](#how-changes-flow)

## Ways to contribute

- **Report a bug** using the [bug report form](https://github.com/pdfjiff/pdfjiff/issues/new/choose). Include `pdfjiff --version` and the `--json` output. A reproduction is usually more useful than a patch.
- **Ask or propose** in [Discussions](https://github.com/pdfjiff/pdfjiff/discussions) before starting large changes. Agreeing on the approach first saves you from rewriting.
- **Pick up an issue** labelled [`good first issue`](https://github.com/pdfjiff/pdfjiff/labels/good%20first%20issue) or [`help wanted`](https://github.com/pdfjiff/pdfjiff/labels/help%20wanted). Comment on the issue so nobody duplicates your work.
- **Improve the docs.** If something confused you, it will confuse the next person too.

To report a security issue, follow [SECURITY.md](SECURITY.md); do not open a public issue.

## Set up in a few minutes

You need [Rust](https://rustup.rs) 1.89 or newer and Git. There are no other system dependencies. On Windows, the rustup installer prompts you to install the Visual Studio C++ build tools it needs.

```bash
git clone https://github.com/pdfjiff/pdfjiff.git
cd pdfjiff
cargo test --workspace          # first build downloads and compiles dependencies
cargo run -p pdfjiff-cli -- --help
```

If the tests pass, you're ready.

## Project layout

```text
pdfjiff/
├── crates/
│   ├── pdfjiff-core/     Library: inspection, compression and page assembly. No I/O policy.
│   │   └── src/          compress.rs, pages.rs, builder.rs, inspect.rs
│   └── pdfjiff-cli/      The `pdfjiff` binary: arguments, file safety, JSON output
│       ├── src/          main.rs (commands), args.rs (flags), files.rs (atomic output), error.rs
│       └── tests/cli.rs  End-to-end tests that run the real binary
├── docs/                 User and maintainer documentation
├── install.sh / .ps1     One-line installers (also attached to each release)
├── action.yml            GitHub Action that installs pdfjiff
├── Dockerfile            Minimal image built from release binaries
└── .github/              CI, release pipeline, issue and PR templates
```

`pdfjiff-core` stays free of CLI concerns, so other programs can embed it. File handling, limits, JSON and exit codes belong in `pdfjiff-cli`.

## Everyday commands

| Task | Command |
| --- | --- |
| Run the CLI | `cargo run -p pdfjiff-cli -- compress sample.pdf` |
| All tests | `cargo test --workspace` |
| One test | `cargo test -p pdfjiff-cli merge_preserves_input_order` |
| Format | `cargo fmt --all` |
| Lint (as CI does) | `cargo clippy --workspace --all-targets -- -D warnings` |
| API docs | `cargo doc --workspace --no-deps --open` |
| Dependency policy | `cargo deny check` (install with `cargo install cargo-deny`) |
| Release build | `cargo build --release -p pdfjiff-cli` → `target/release/pdfjiff` |

CI runs formatting, clippy, tests on Linux, macOS and Windows, a minimum-Rust-version check, a docs build, `cargo deny`, a secret scan and installer script checks. If these pass locally, CI will almost certainly pass too.

## Tests and sample PDFs

- Every behavior change needs a test that fails without the change.
- CLI behavior is tested end to end in `crates/pdfjiff-cli/tests/cli.rs`. These tests run the compiled binary and check its JSON output, exit codes and the files it writes.
- Algorithm tests live next to the code in `crates/pdfjiff-core/src`.
- **Generate fixtures in code** (see `fixture()` in `tests/cli.rs`) instead of committing binary PDFs. Generated fixtures are small, readable and easy to review.
- **Never commit real documents.** Do not attach them to issues either, unless you are sure they contain nothing personal or confidential. If a bug only reproduces with one file, first try to reduce it to a minimal synthetic PDF.

## Guidelines

- **Safety defaults are a feature.** Never modify an input, never overwrite an output without `--overwrite`, and always publish outputs atomically. Changes that weaken these need an explicit discussion first.
- **Offline and private.** No network access, telemetry or background services.
- **Stable contracts.** Scripts depend on the JSON envelope (`schema_version`), the error codes and the exit codes, so treat them as public API. Adding fields is fine. Renaming or removing them needs a schema version bump and a CHANGELOG entry.
- **Honest output.** Report what actually happened. When a feature is incomplete, say so in warnings or `capabilities`.
- **Safe Rust.** The codebase has no `unsafe`. Keep it that way unless a maintainer agrees otherwise.
- **Dependencies** must be pure Rust where possible, permissively licensed (see `deny.toml`) and justified in the PR description.
- **Style.** Use `rustfmt` defaults and keep clippy clean. Prefer clear names over comments, and use comments to explain *why*.

## Adding a command

1. Add the arguments to `crates/pdfjiff-cli/src/args.rs`, with `--help` text written for users.
2. Put the algorithm in `pdfjiff-core` and handle file safety and the JSON envelope in `main.rs`, reusing `Destination` for outputs.
3. Give the operation a dotted id (for example `pdf.split`) and list it in `capabilities`, including its limitations.
4. Add end-to-end tests covering success, `--dry-run`, refusal to overwrite and at least one failure code.
5. Document it in `docs/usage.md` and the README command table, and add an entry under `Unreleased` in `CHANGELOG.md`.

## Pull requests

1. Fork the repository and create a branch from `main`.
2. Keep each PR focused. Two small PRs get reviewed faster than one large one.
3. Fill in the PR template: what changed, why, and how you tested it. For file-system or console changes, include your OS.
4. Make sure CI passes. Maintainers aim to give a first review within a week.
5. PRs are squash-merged, so the PR title becomes the commit message. [Conventional Commits](https://www.conventionalcommits.org) prefixes (`feat:`, `fix:`, `docs:`) help us write release notes.

By submitting a contribution, you agree that it is licensed under the project's dual MIT OR Apache-2.0 license, as described in the [README](README.md#license). You don't need to sign a CLA.

## How changes flow

PDFJiff's maintainers also use these crates in a larger downstream product. Because of that:

- Maintainer work sometimes arrives as a reviewed commit that syncs several downstream changes at once. These commits follow the same CI and review rules as everyone else's.
- Merged community PRs are carried downstream with attribution. Your commit stays in this repository's history, and release notes credit you.
- This repository is the source of truth for the open-source crates. Everything needed to build, test and release them is here.

Releases are cut by maintainers. The process is described in [docs/releasing.md](docs/releasing.md).

## Code of Conduct

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
