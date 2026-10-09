# Releasing PDFJiff (maintainers)

A release is one tag push. The [Release workflow](../.github/workflows/release.yml) builds, packages, attests and publishes everything. The [Installers workflow](../.github/workflows/installers.yml) then installs the result through every public channel to check that it works.

- [What a release produces](#what-a-release-produces)
- [One-time setup](#one-time-setup)
- [Cut a release](#cut-a-release)
- [Pre-releases](#pre-releases)
- [When something goes wrong](#when-something-goes-wrong)
- [How the pipeline is secured](#how-the-pipeline-is-secured)

## What a release produces

| Channel | Produced by | Enabled |
| --- | --- | --- |
| GitHub release: 6 archives, `SHA256SUMS`, `install.sh`, `install.ps1` | `publish` job | Always |
| Build-provenance attestations for every archive and the image | `publish`, `docker` | Always |
| `ghcr.io/pdfjiff/pdfjiff:{X.Y.Z, X.Y, latest}` (amd64 + arm64) | `docker` | Always |
| `vX` tag for `uses: pdfjiff/pdfjiff@vX` | `action-tag` | Stable releases |
| crates.io: `pdfjiff-core`, `pdfjiff-cli` | `crates` | Repository variable `PUBLISH_CRATES=true` |
| Homebrew formula `Formula/pdfjiff.rb` | `homebrew` | Repository variable `HOMEBREW_TAP` |
| Scoop manifest `bucket/pdfjiff.json` | `scoop` | Repository variable `SCOOP_BUCKET` |

Release notes combine the matching `CHANGELOG.md` section with GitHub's generated list of merged PRs and new contributors.

## One-time setup

### 1. Repository settings

Run the configuration script once with the [GitHub CLI](https://cli.github.com) (you need admin rights). It is safe to re-run.

```bash
gh auth login
bash .github/scripts/configure-repo.sh pdfjiff/pdfjiff --dry-run   # preview every call
bash .github/scripts/configure-repo.sh pdfjiff/pdfjiff
```

It sets the following:

- description, homepage and topics
- squash-only merges and branch deletion after merge
- labels: `good first issue`, `help wanted`, `needs triage`, platform labels and more
- Discussions on, wiki and projects off
- Dependabot alerts and security updates
- secret scanning with push protection
- private vulnerability reporting
- a read-only default `GITHUB_TOKEN`
- a **Protect main** ruleset: pull requests must pass **CI passed**, force pushes and deletion are blocked, and repository admins can bypass to push sync commits

GitHub has no API for a few settings, so do these by hand. The script prints this list too:

- **Settings → General → Social preview:** upload `docs/assets/social-preview.png` (1280×640).
- **Account security:** turn on two-factor authentication. If the repository belongs to an organization, require it for all members.

### 2. Docker image (GHCR)

Nothing to configure, because the workflow uses `GITHUB_TOKEN`. After the **first** release, open the package page (**Your profile → Packages → pdfjiff**) and set **visibility to Public**. Also connect it to the repository so it appears in the sidebar.

### 3. Homebrew tap

1. Create a public repository named `homebrew-tap` (for example `pdfjiff/homebrew-tap`) with a README.
2. Create a [fine-grained token](https://github.com/settings/personal-access-tokens/new) that has access to **only** that repository, with **Contents: Read and write** permission.
3. In pdfjiff's **Settings → Secrets and variables → Actions**, add the secret `HOMEBREW_TAP_TOKEN` and the variable `HOMEBREW_TAP=pdfjiff/homebrew-tap`.

Users can then run `brew install pdfjiff/tap/pdfjiff`.

### 4. Scoop bucket

Create `pdfjiff/scoop-bucket` with an empty `bucket/` folder, and a fine-grained token scoped to it. Then add the secret `SCOOP_BUCKET_TOKEN` and the variable `SCOOP_BUCKET=pdfjiff/scoop-bucket`.

### 5. crates.io

crates.io [trusted publishing](https://crates.io/docs/trusted-publishing) removes long-lived tokens, but it can only be configured after each crate exists.

1. **First release only:** create an API token at <https://crates.io/settings/tokens> (scope: `publish-new`, `publish-update`). Add it as the secret `CARGO_REGISTRY_TOKEN`, and set the variable `PUBLISH_CRATES=true`.
2. After the first release, open each crate's **Settings → Trusted Publishing** on crates.io. Add this repository, workflow `release.yml` and environment `crates-io`, then **delete** the `CARGO_REGISTRY_TOKEN` secret. The workflow switches to trusted publishing automatically.

### 6. GitHub Marketplace (optional)

The root `action.yml` is a Marketplace-ready action named **Setup PDFJiff**. When editing a release on GitHub, tick **Publish this Action to the GitHub Marketplace**.

## Cut a release

1. **Make sure `main` is green** and contains everything you want to ship.
2. **Bump the version** in the root `Cargo.toml`: set `workspace.package.version` and the `pdfjiff-core` version under `workspace.dependencies` to the new version (for example `0.2.0`). Then run `cargo check` to refresh `Cargo.lock`.
3. **Update `CHANGELOG.md`:** move the `Unreleased` entries under `## [0.2.0] - YYYY-MM-DD` with today's date, and update the compare links at the bottom.
4. Open a PR titled `release: v0.2.0` and merge it when CI passes.
5. **Tag and push:**

   ```bash
   git switch main && git pull --ff-only
   git tag -a v0.2.0 -m "PDFJiff 0.2.0"
   git push origin v0.2.0
   ```

6. Watch **Actions → Release**. The `plan` job stops early if the tag, the Cargo version or the CHANGELOG date disagree, before anything is built or published.
7. When the Release workflow finishes, **Actions → Installers** runs automatically. It installs the release with `install.sh`, `install.ps1`, Homebrew, Docker and the Action on Linux, macOS and Windows. If it fails, treat the release as broken.

## Pre-releases

Tags with a suffix, such as `v0.2.0-rc.1`, create a GitHub **pre-release**. The Docker image is tagged only `0.2.0-rc.1`, and Homebrew, Scoop, `latest` and the `vX` Action tag are not changed. If `PUBLISH_CRATES` is on, crates.io publishes the pre-release version, and Cargo does not select pre-releases unless asked.

## When something goes wrong

| Situation | What to do |
| --- | --- |
| `plan` fails (version, date or dependency mismatch) | Fix it on `main`, then move the tag: `git tag -fa vX.Y.Z -m ... && git push -f origin vX.Y.Z`. Nothing has been published yet. |
| One build target fails | Nothing has been published yet. Fix the problem, move the tag, and push again. |
| A later job fails (Docker, crates, Homebrew, Scoop) | The GitHub release exists. Fix the configuration, then re-run the failed jobs from the workflow run page. |
| Re-run everything for an existing tag | Delete the GitHub release, then run **Release → Run workflow** with the tag. |
| A published release is broken | Mark it as a pre-release on GitHub (so `latest` points at the previous one), publish a fixed patch release, and `cargo yank --version X.Y.Z pdfjiff-cli pdfjiff-core` if needed. The next stable release overwrites the Homebrew and Scoop manifests. |

## How the pipeline is secured

- Every third-party action is pinned to a full commit SHA, and Dependabot proposes updates weekly.
- Workflows default to no permissions. Each job asks for only what it needs, such as `contents: write` to create the release or `packages: write` for GHCR.
- Archives are packaged reproducibly (fixed timestamps and modes) and published with SHA-256 checksums and Sigstore-backed build-provenance attestations. The installers refuse any file whose checksum does not match.
- Tap and bucket tokens are fine-grained and limited to one repository each. crates.io uses short-lived OIDC tokens after the first release.
- The Docker image contains only the static binary, runs as a non-root user and has no shell.
