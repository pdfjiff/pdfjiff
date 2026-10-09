//! Filesystem policy is separate from PDF processing so all commands share it.
use crate::error::{Failure, Result};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

pub fn read_input(path: &Path, limit: u64) -> Result<Vec<u8>> {
    require_utf8(path)?;
    let file = File::open(path).map_err(|e| {
        Failure::new(
            "INPUT_UNAVAILABLE",
            format!("{}: {e}", path.display()),
            "Check the input path and read permissions.",
            3,
        )
    })?;
    let metadata = file.metadata().map_err(|e| {
        Failure::new(
            "INPUT_UNAVAILABLE",
            e.to_string(),
            "Check the input file.",
            3,
        )
    })?;
    if !metadata.is_file() {
        return Err(Failure::new(
            "INVALID_INPUT",
            "Input must be a regular file.",
            "stdin and directories are not supported in this prerelease.",
            3,
        ));
    }
    if metadata.len() > limit {
        return Err(size_error(limit));
    }
    let mut data = Vec::new();
    file.take(limit + 1).read_to_end(&mut data).map_err(|e| {
        Failure::new(
            "INPUT_UNAVAILABLE",
            e.to_string(),
            "Check the input file.",
            3,
        )
    })?;
    if data.len() as u64 > limit {
        return Err(size_error(limit));
    }
    Ok(data)
}
fn size_error(limit: u64) -> Failure {
    Failure::new(
        "INPUT_LIMIT",
        format!("Input exceeds the {limit}-byte limit."),
        "Use --max-input-mib or --max-total-input-mib only if enough memory is available.",
        6,
    )
}

pub fn default_output(input: &Path) -> PathBuf {
    let mut name = input.file_stem().unwrap_or_default().to_os_string();
    name.push("-compressed.pdf");
    input.with_file_name(name)
}

pub struct Destination {
    path: PathBuf,
    inputs: Vec<PathBuf>,
    overwrite: bool,
    temp: Option<NamedTempFile>,
}
impl Destination {
    pub fn prepare(
        path: &Path,
        inputs: &[PathBuf],
        overwrite: bool,
        dry_run: bool,
    ) -> Result<Self> {
        if path == Path::new("-") {
            return Err(Failure::new(
                "UNSUPPORTED_OUTPUT",
                "Raw stdout output is not supported yet.",
                "Choose a file path; --json is available for structured results.",
                2,
            ));
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = fs::canonicalize(parent).map_err(|e| {
            Failure::new(
                "INVALID_OUTPUT",
                e.to_string(),
                "Create the output directory first.",
                2,
            )
        })?;
        let name = path.file_name().ok_or_else(|| {
            Failure::new(
                "INVALID_OUTPUT",
                "Output requires a filename.",
                "Choose a PDF filename.",
                2,
            )
        })?;
        let mut value = Self {
            path: parent.join(name),
            inputs: inputs.to_vec(),
            overwrite,
            temp: None,
        };
        require_utf8(&value.path)?;
        value.validate()?;
        if !dry_run {
            value.temp = Some(new_temp(&parent).map_err(|e| {
                Failure::new(
                    "OUTPUT_UNAVAILABLE",
                    e.to_string(),
                    "Check destination permissions and free space.",
                    7,
                )
            })?);
        }
        Ok(value)
    }
    fn validate(&self) -> Result<()> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(value) => Some(value),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(Failure::new(
                    "INVALID_OUTPUT",
                    error.to_string(),
                    "Check destination permissions.",
                    2,
                ))
            }
        };
        for input in &self.inputs {
            let canonical = fs::canonicalize(input).map_err(|e| {
                Failure::new(
                    "INPUT_UNAVAILABLE",
                    format!("{}: {e}", input.display()),
                    "Check the input path.",
                    3,
                )
            })?;
            let same = self.path == canonical
                || (metadata.is_some()
                    && same_file::is_same_file(input, &self.path).unwrap_or(false));
            if same {
                return Err(Failure::new(
                    "OUTPUT_IS_INPUT",
                    "The output refers to an input file.",
                    "Choose a distinct output path; --overwrite never modifies an input.",
                    2,
                ));
            }
        }
        if let Some(metadata) = metadata {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(Failure::new(
                    "INVALID_OUTPUT",
                    "Output must not be a directory or symbolic link.",
                    "Choose a distinct regular-file destination.",
                    2,
                ));
            }
            if !self.overwrite {
                return Err(Failure::new(
                    "OUTPUT_EXISTS",
                    format!("{} already exists.", self.path.display()),
                    "Choose --output or explicitly use --overwrite.",
                    2,
                ));
            }
        }
        Ok(())
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn publish(mut self, bytes: &[u8]) -> Result<PathBuf> {
        self.validate()?;
        let mut temp = self
            .temp
            .take()
            .ok_or_else(|| Failure::processing("Cannot publish a dry-run destination."))?;
        temp.write_all(bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| {
                Failure::new(
                    "OUTPUT_WRITE_FAILED",
                    e.to_string(),
                    "Check disk space; the previous output was not replaced.",
                    7,
                )
            })?;
        #[cfg(unix)]
        if self.overwrite {
            // A replaced output keeps its existing permissions; best effort.
            if let Ok(existing) = fs::metadata(&self.path) {
                let _ = fs::set_permissions(temp.path(), existing.permissions());
            }
        }
        // Same-volume atomic replacement, or no-clobber publication. Never truncate an existing output.
        let result = if self.overwrite {
            temp.persist(&self.path)
        } else {
            temp.persist_noclobber(&self.path)
        };
        result.map_err(|e| {
            if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                Failure::new(
                    "OUTPUT_EXISTS",
                    "Another process created the destination.",
                    "Choose another output path.",
                    2,
                )
            } else {
                Failure::new(
                    "OUTPUT_WRITE_FAILED",
                    e.error.to_string(),
                    "Check permissions and disk space.",
                    7,
                )
            }
        })?;
        Ok(self.path)
    }
}

/// Temporary output in the destination directory. New outputs get ordinary
/// file permissions (0o666 minus the umask) instead of tempfile's owner-only
/// 0o600 default, so they behave like files written by other tools.
fn new_temp(parent: &Path) -> std::io::Result<NamedTempFile> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o666))
            .tempfile_in(parent)
    }
    #[cfg(not(unix))]
    {
        NamedTempFile::new_in(parent)
    }
}

fn require_utf8(path: &Path) -> Result<()> {
    if path.to_str().is_none() {
        return Err(Failure::new(
            "UNSUPPORTED_PATH",
            "Path is not valid UTF-8.",
            "Rename the file or directory to a Unicode name.",
            2,
        ));
    }
    Ok(())
}
