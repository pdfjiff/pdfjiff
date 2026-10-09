# Development workflow

This page describes how work moves through the repository. For setup and pull-request mechanics, see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Branches and checks

- `main` is always releasable. Changes land through pull requests that pass the **CI passed** check. Maintainer sync commits may be pushed directly, and CI runs on them in the same way. The check covers:
  - formatting, clippy (`-D warnings`) and docs
  - tests on Linux, macOS and Windows
  - a minimum-Rust-version check
  - a crates.io packaging dry run
  - `cargo deny` license and advisory checks, plus third-party notice generation
  - a TruffleHog secret scan of new commits (all history weekly)
  - shellcheck, release tooling tests and actionlint
- PRs that change an installer, `action.yml` or the release scripts also run the **Installers** workflow. It builds a local release and installs it with `install.sh`, `install.ps1` (PowerShell 7 and 5.1) and the GitHub Action.
- PRs are squash-merged. The PR title becomes the commit message.

## Where changes come from

Most changes are community pull requests or maintainer pull requests. PDFJiff's maintainers also use these crates in a downstream product, so some maintainer changes arrive as a reviewed *sync* commit that brings over several downstream fixes at once. Sync commits follow the same CI rules, touch only the files in this repository, and are described in public terms. Accepted community changes are carried downstream with credit, and this repository's history is never rewritten.

## Compatibility promises

Scripts and other programs depend on these, so they change deliberately:

| Surface | Promise |
| --- | --- |
| CLI flags and commands | Not removed or renamed without a deprecation period across at least one minor release |
| JSON envelope (`schema_version`) | Fields are only added. Removals, renames and meaning changes bump `schema_version`. |
| Error codes and exit codes | Stable. New codes may be added. |
| `pdfjiff-core` Rust API | Semantic versioning. Before 1.0, minor versions may break it, and the CHANGELOG says how. |

User-visible changes need an entry under `Unreleased` in `CHANGELOG.md` and documentation in `docs/usage.md`.

## Releases

Maintainers cut releases by pushing a version tag. Everything else is automated, as described in [releasing.md](releasing.md). A merged pull request is not a release. Releases happen when there is something worth shipping.
