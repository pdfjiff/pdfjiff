use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "pdfjiff",
    version,
    about = "Local PDF inspection, compression and merging",
    long_about = "Fast, private PDF tools that run entirely on your machine. Inputs are never modified, and outputs are never overwritten without --overwrite. Run `pdfjiff capabilities` to see what this build supports."
)]
pub struct Cli {
    /// Emit one versioned JSON result; diagnostics never pollute stdout.
    #[arg(long, global = true)]
    pub json: bool,
    /// Maximum bytes per input, expressed as an integer number of MiB.
    #[arg(long, global = true, default_value_t = 128, value_parser = clap::value_parser!(u64).range(1..=4096))]
    pub max_input_mib: u64,
    /// Maximum sum of input bytes for a merge, in MiB (not a peak-RAM guarantee).
    #[arg(long, global = true, default_value_t = 256, value_parser = clap::value_parser!(u64).range(1..=8192))]
    pub max_total_input_mib: u64,
    #[command(subcommand)]
    pub command: Option<Command>,
}
#[derive(Subcommand)]
pub enum Command {
    /// Read page geometry and basic structural/protection indicators.
    Inspect { input: PathBuf },
    /// Recompress images (lossy presets), or optimize structure with --lossless.
    Compress(Compress),
    /// Merge PDFs in the exact input order. Requires --output.
    Merge(Merge),
    /// Report actual capabilities and deliberate prerelease limitations.
    Capabilities,
    /// Print a shell completion script to stdout.
    #[command(after_help = "Examples:
  pdfjiff completions bash > ~/.local/share/bash-completion/completions/pdfjiff
  pdfjiff completions zsh > ~/.zfunc/_pdfjiff
  pdfjiff completions fish > ~/.config/fish/completions/pdfjiff.fish
  pdfjiff completions powershell >> $PROFILE")]
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}
#[derive(Args)]
pub struct WriteOptions {
    /// Output path. Compress defaults to INPUT-compressed.pdf; merge requires it.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Atomically replace an existing distinct output. Never permits input overwrite.
    #[arg(long)]
    pub overwrite: bool,
    /// Inspect prerequisites and the proposed output without writing a PDF.
    #[arg(long)]
    pub dry_run: bool,
    /// Acknowledge that rewriting a signed PDF invalidates its signatures.
    #[arg(long)]
    pub allow_signature_invalidation: bool,
}
#[derive(Args)]
pub struct Compress {
    pub input: PathBuf,
    #[command(flatten)]
    pub write: WriteOptions,
    #[arg(long, value_enum, default_value_t = Preset::Balanced)]
    pub preset: Preset,
    /// Leave image pixels untouched; optimize document structure only.
    #[arg(long, conflicts_with = "preset")]
    pub lossless: bool,
    /// Maximum output size: integer bytes, KB, MB, KiB or MiB. No raster fallback.
    #[arg(long, value_parser = parse_size)]
    pub target: Option<u64>,
}
#[derive(Clone, Copy, ValueEnum)]
pub enum Preset {
    Quality,
    Balanced,
    Small,
}
impl Preset {
    pub fn settings(self) -> (pdfjiff_core::compress::CompressionPreset, u8, u32) {
        use pdfjiff_core::compress::CompressionPreset as Core;
        match self {
            Self::Quality => (Core::Quality, 86, 3200),
            Self::Balanced => (Core::Balanced, 72, 2000),
            Self::Small => (Core::Small, 50, 1400),
        }
    }
}
#[derive(Args)]
pub struct Merge {
    #[arg(required = true, num_args = 2..)]
    pub inputs: Vec<PathBuf>,
    #[command(flatten)]
    pub write: WriteOptions,
    /// Permit reported loss of document-level forms, bookmarks, tags, etc.
    #[arg(long)]
    pub allow_structure_loss: bool,
}
pub fn parse_size(value: &str) -> std::result::Result<u64, String> {
    let value = value.trim();
    let index = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    let number = value[..index]
        .parse::<u64>()
        .map_err(|_| "Use a positive integer size, such as 200KiB or 2MB.".to_owned())?;
    let scale = match value[index..].to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "kb" => 1000,
        "mb" => 1_000_000,
        "kib" => 1024,
        "mib" => 1024 * 1024,
        _ => return Err("Supported units: B, KB, MB, KiB, MiB.".into()),
    };
    number
        .checked_mul(scale)
        .filter(|n| *n > 0)
        .ok_or_else(|| "Size must be positive and fit into 64 bits.".into())
}
