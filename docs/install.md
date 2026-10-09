# Installing PDFJiff

Every method below installs the same single `pdfjiff` binary. Pick whichever you already use.

- [One-line installer (macOS, Linux)](#macos-and-linux-one-line-installer)
- [One-line installer (Windows)](#windows-one-line-installer)
- [Homebrew](#homebrew) · [Scoop](#scoop) · [Cargo](#cargo)
- [Docker](#docker) · [GitHub Actions](#github-actions)
- [Manual download and verification](#manual-download-and-verification)
- [Build from source](#build-from-source)
- [Shell completions](#shell-completions) · [Upgrade](#upgrade) · [Uninstall](#uninstall) · [Troubleshooting](#troubleshooting)

Supported platforms:

| OS | x86-64 | ARM64 |
| --- | --- | --- |
| macOS 11+ | `x86_64-apple-darwin` | `aarch64-apple-darwin` (Apple silicon) |
| Linux, any distro | `x86_64-unknown-linux-musl` | `aarch64-unknown-linux-musl` |
| Windows 10+ | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` |

The Linux builds are fully static, so they run on Alpine, Debian, Fedora, NixOS, inside minimal containers and on older distributions alike.

## macOS and Linux: one-line installer

```bash
curl -fsSL https://github.com/pdfjiff/pdfjiff/releases/latest/download/install.sh | sh
```

The installer detects your OS and CPU and downloads the matching archive from the GitHub release. It **verifies the SHA-256 checksum** and installs `pdfjiff` to `~/.local/bin`. It never uses `sudo` and never edits your shell configuration. If `~/.local/bin` isn't on your `PATH`, it prints the one line to add.

Options go after `sh -s --`:

```bash
curl -fsSL https://github.com/pdfjiff/pdfjiff/releases/latest/download/install.sh | sh -s -- --version 0.1.0
curl -fsSL https://github.com/pdfjiff/pdfjiff/releases/latest/download/install.sh | sh -s -- --to ~/bin
curl -fsSL https://github.com/pdfjiff/pdfjiff/releases/latest/download/install.sh | sudo sh -s -- --to /usr/local/bin
```

Prefer to read scripts before running them? Download `install.sh`, read it, then run `sh install.sh`.

## Windows: one-line installer

In PowerShell (no administrator rights needed):

```powershell
irm https://github.com/pdfjiff/pdfjiff/releases/latest/download/install.ps1 | iex
```

This installs `pdfjiff.exe` to `%LOCALAPPDATA%\Programs\pdfjiff\bin` after verifying its checksum, and adds that folder to your user `PATH`. Open a new terminal afterwards. To pin a version or choose a folder, set environment variables first:

```powershell
$env:PDFJIFF_VERSION = '0.1.0'; $env:PDFJIFF_INSTALL_DIR = 'C:\tools'
irm https://github.com/pdfjiff/pdfjiff/releases/latest/download/install.ps1 | iex
```

## Homebrew

macOS and Linux:

```bash
brew install pdfjiff/tap/pdfjiff
```

Homebrew also installs bash, zsh, fish and PowerShell completions. Upgrade with `brew upgrade pdfjiff`.

## Scoop

Windows:

```powershell
scoop bucket add pdfjiff https://github.com/pdfjiff/scoop-bucket
scoop install pdfjiff
```

## Cargo

If you have Rust installed:

```bash
cargo binstall pdfjiff-cli          # downloads the prebuilt release binary (fast)
cargo install pdfjiff-cli --locked  # compiles from crates.io (a few minutes)
```

Both install `pdfjiff` into `~/.cargo/bin`.

## Docker

```bash
docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/work" ghcr.io/pdfjiff/pdfjiff compress report.pdf
```

- The image is `FROM scratch` and contains only the static binary. It is available for `linux/amd64` and `linux/arm64`.
- `/work` is the working directory, so mount the folder that holds your PDFs there.
- `--user "$(id -u):$(id -g)"` makes output files belong to you. Without it, the container runs as UID 65532, which may not be allowed to write to your folder on Linux.
- Tags: `latest`, `X.Y` and `X.Y.Z`. Pin `X.Y.Z` in automation.

## GitHub Actions

```yaml
steps:
  - uses: actions/checkout@v7
  - uses: pdfjiff/pdfjiff@v0            # installs the latest release and adds it to PATH
    with:
      version: latest                 # or pin, e.g. "0.1.0"
  - run: pdfjiff compress docs/manual.pdf --target 5MB -o dist/manual.pdf
```

This works on Linux, macOS and Windows runners. The action's outputs are `version` and `path`.

## Manual download and verification

Every [release](https://github.com/pdfjiff/pdfjiff/releases) contains:

| File | Contents |
| --- | --- |
| `pdfjiff-<target>.tar.gz` / `.zip` | `pdfjiff-<target>/pdfjiff[.exe]`, `README.md`, `LICENSE-MIT`, `LICENSE-APACHE` and `THIRD-PARTY-LICENSES.md` |
| `SHA256SUMS` | Checksums for every file in the release |
| `install.sh`, `install.ps1` | The installers, tied to that release |
| `THIRD-PARTY-LICENSES.md` | Licenses of the open-source crates compiled into `pdfjiff` |

To verify a download yourself:

```bash
f=pdfjiff-aarch64-apple-darwin.tar.gz
grep " $f\$" SHA256SUMS | shasum -a 256 -c -       # macOS (Linux: sha256sum -c -)
gh attestation verify "$f" --repo pdfjiff/pdfjiff     # build provenance
```

The attestation proves that the archive was built by this repository's release workflow from the tagged commit.

On macOS, a binary downloaded **with a web browser** is quarantined by Gatekeeper. Clear the quarantine with `xattr -d com.apple.quarantine ./pdfjiff`. The installers, Homebrew and Cargo do not trigger this.

## Build from source

```bash
git clone https://github.com/pdfjiff/pdfjiff.git
cd pdfjiff
cargo build --release --locked -p pdfjiff-cli
./target/release/pdfjiff --version
```

This requires Rust 1.89 or newer ([rustup.rs](https://rustup.rs)). There are no C dependencies.

## Shell completions

```bash
pdfjiff completions bash > ~/.local/share/bash-completion/completions/pdfjiff
pdfjiff completions zsh  > ~/.zfunc/_pdfjiff        # with `fpath+=~/.zfunc` before `compinit` in ~/.zshrc
pdfjiff completions fish > ~/.config/fish/completions/pdfjiff.fish
pdfjiff completions powershell >> $PROFILE
```

## Upgrade

Re-run the installer, or use your package manager: `brew upgrade pdfjiff`, `scoop update pdfjiff`, `cargo binstall pdfjiff-cli` or `docker pull ghcr.io/pdfjiff/pdfjiff`.

## Uninstall

| Installed with | Remove with |
| --- | --- |
| `install.sh` | `curl -fsSL https://github.com/pdfjiff/pdfjiff/releases/latest/download/install.sh \| sh -s -- --uninstall` (or `rm ~/.local/bin/pdfjiff`) |
| `install.ps1` | `$env:PDFJIFF_UNINSTALL='1'; irm https://github.com/pdfjiff/pdfjiff/releases/latest/download/install.ps1 \| iex` |
| Homebrew | `brew uninstall pdfjiff` |
| Scoop | `scoop uninstall pdfjiff` |
| Cargo | `cargo uninstall pdfjiff-cli` |

PDFJiff keeps no configuration, cache or background service, so there is nothing else to remove.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| `pdfjiff: command not found` | Add the install folder to your `PATH` using the line the installer printed, then open a new terminal. |
| `checksum mismatch` | The download was corrupted or altered, and nothing was installed. Retry. If it persists, [report it](../SECURITY.md). |
| `download failed` | Check that the version exists on the releases page. Behind a proxy, set `HTTPS_PROXY`. For an offline mirror, set `PDFJIFF_DOWNLOAD_BASE` to a folder URL that contains the archives and `SHA256SUMS`. |
| `... is not writable` | Choose a folder you own with `--to`, or use `sudo` with `--to /usr/local/bin`. |
| PowerShell: `running scripts is disabled` | `irm \| iex` doesn't need script execution. If your policy blocks it, run `Set-ExecutionPolicy -Scope Process Bypass` in that window only. |
| Windows: `could not write pdfjiff.exe` | A `pdfjiff` process is still running. Close it and retry. |
| Docker: `permission denied` writing output | Add `--user "$(id -u):$(id -g)"`. |
