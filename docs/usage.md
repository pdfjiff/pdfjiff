# Usage reference

This page documents `pdfjiff` 0.1. The binary itself is always the authority: run `pdfjiff --help`, `pdfjiff <command> --help` and `pdfjiff capabilities --json`.

- [Global options](#global-options)
- [inspect](#inspect) · [compress](#compress) · [merge](#merge) · [capabilities](#capabilities) · [completions](#completions)
- [Safety model](#safety-model)
- [JSON output](#json-output)
- [Exit codes](#exit-codes) · [Error codes](#error-codes)

## Global options

These options work with every command:

| Option | Default | Meaning |
| --- | --- | --- |
| `--json` | off | Print exactly one JSON result on stdout. Warnings go into the result instead of stderr. |
| `--max-input-mib N` | 128 | Refuse any single input larger than N MiB (1–4096). |
| `--max-total-input-mib N` | 256 | Refuse a merge whose inputs total more than N MiB (1–8192). |
| `-h`, `--help` | | Show help. |
| `-V`, `--version` | | Print the version. |

Size limits cap the bytes read from disk. They are not a limit on peak memory use.

## inspect

```bash
pdfjiff inspect report.pdf [--json]
```

Reads the document's structure without changing anything. It reports:

| Field | Meaning |
| --- | --- |
| `page_count` | Number of pages. `null` if encryption prevents reading the page tree. |
| `pages[]` | `width_pt`, `height_pt` (MediaBox, 1 pt = 1/72 inch) and effective `rotation` (0/90/180/270) |
| `encrypted` | Whether the file is encrypted |
| `signatures_detected` | Whether signature dictionaries were found. This is **not** signature validation. |
| `assembly_sensitive_features` | Structures `merge` cannot preserve: bookmarks, forms, named destinations or embedded files, destinations, accessibility tags, page labels, XMP document metadata, document actions, optional content layers, output color profiles, and page annotations or links |

```console
$ pdfjiff inspect appendix.pdf --json
{"coverage":{"rendered":false,"signature_validation":false},"error":null,"operation":"document.inspect","outputs":[],"result":{"assembly_sensitive_features":[],"encrypted":false,"page_count":2,"pages":[{"height_pt":841.8897705078125,"rotation":0,"width_pt":595.2755737304688},{"height_pt":841.8897705078125,"rotation":0,"width_pt":595.2755737304688}],"signatures_detected":false},"schema_version":1,"stats":{"elapsed_ms":0.36218999999999996,"input_bytes":2426},"status":"succeeded","warnings":[]}
```

## compress

```bash
pdfjiff compress INPUT [-o OUTPUT] [--preset quality|balanced|small] [--lossless] [--target SIZE]
                       [--overwrite] [--dry-run] [--allow-signature-invalidation] [--json]
```

Recompresses embedded images and optimizes the document structure. Without `-o`, the output is `INPUT-compressed.pdf` in the same folder.

| Preset | JPEG quality | Longest image edge | Use it for |
| --- | --- | --- | --- |
| `quality` | 86 | 3200 px | Print, archiving |
| `balanced` (default) | 72 | 2000 px | Sharing and email |
| `small` | 50 | 1400 px | Upload limits, previews |

- An image is replaced only when the re-encoded version is smaller. If nothing gets smaller, the output is identical to the input and the result reports `"unchanged": true`.
- `--lossless` leaves every image's pixels untouched and optimizes only the structure (streams, objects). It cannot be combined with `--preset`.
- `--target SIZE` sets a maximum output size, for example `500KB`, `2MB` or `3MiB`. Accepted units are `B`, `KB` (1000), `MB` (1000²), `KiB` (1024) and `MiB` (1024²), and the number must be a whole number. PDFJiff first tries your preset, then up to five increasingly compact settings (lower quality, smaller images), and keeps the smallest result. If even the most compact attempt is too big, it exits with code `6` (`TARGET_UNREACHABLE`) and writes nothing. It never rasterizes pages or removes content to reach a target. With `--lossless`, only one attempt is made.
- Metadata, forms and attachments are kept.

```console
$ pdfjiff compress report.pdf --target 1MB -o upload.pdf
pdf.compress: succeeded
Output: /home/you/docs/upload.pdf
```

## merge

```bash
pdfjiff merge A.pdf B.pdf [C.pdf ...] -o OUTPUT [--allow-structure-loss]
                       [--overwrite] [--dry-run] [--allow-signature-invalidation] [--json]
```

Merges pages in exactly the order you list the files. `-o/--output` is required.

- Document metadata (title, author and so on) is not combined, and the command warns about this.
- If an input has structures that page assembly cannot keep (for example bookmarks, forms, links or XMP metadata; `inspect` lists them), `merge` stops with `UNSUPPORTED_PRESERVATION`. Many PDFs exported from office software include at least one of these. Add `--allow-structure-loss` to proceed; each dropped structure is listed in `warnings`.
- The total input size is limited by `--max-total-input-mib`.

## capabilities

```bash
pdfjiff capabilities --json
```

Lists the operations this build supports, their features and limitations, what is not implemented yet, and the active limits. Scripts and AI agents should check this instead of assuming what a version can do.

## completions

```bash
pdfjiff completions bash > ~/.local/share/bash-completion/completions/pdfjiff
pdfjiff completions zsh  > ~/.zfunc/_pdfjiff          # add `fpath+=~/.zfunc` before compinit
pdfjiff completions fish > ~/.config/fish/completions/pdfjiff.fish
pdfjiff completions powershell >> $PROFILE
```

Supported shells are `bash`, `zsh`, `fish`, `powershell` and `elvish`. Homebrew installs completions for you automatically.

## Safety model

PDFJiff behaves the same way for every command:

1. **Inputs are read-only.** An output can never be one of the inputs, including through hard links or symbolic links. This holds even with `--overwrite`.
2. **No accidental overwrites.** If the output exists, the command fails with `OUTPUT_EXISTS` unless you pass `--overwrite`.
3. **Atomic publication.** The output is written to a temporary file in the destination folder, flushed to disk, and then renamed into place. A failed run leaves the previous file untouched and no partial output behind. A crash or power loss during the write can leave a hidden temporary file, but never a truncated PDF at your output path.
4. **Ordinary permissions.** New files get your usual default permissions (your umask). A file replaced with `--overwrite` keeps its existing permissions.
5. **Explicit acknowledgements.** Rewriting a signed PDF invalidates its signatures, so it requires `--allow-signature-invalidation`. Dropping document structures in a merge requires `--allow-structure-loss`.
6. **Dry runs.** `--dry-run` reads and checks the inputs and reports the planned output (`"status": "planned"`) without writing anything. It does not guarantee that a target is reachable.
7. **Cancellation.** Ctrl-C stops the run between processing stages. The command exits with code `130` and publishes nothing, unless publication had already started.
8. **Offline.** PDFJiff makes no network requests, downloads nothing and starts no background process.

Paths must be valid Unicode (UTF-8). Reading from stdin and writing to stdout are not supported yet.

## JSON output

With `--json`, stdout contains exactly one JSON object and a trailing newline. That applies to failures too, including invalid arguments. (`--help` and `--version` always print plain text.)

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | `1`. Only incremented for breaking changes. |
| `operation` | string | `document.inspect`, `pdf.compress`, `pdf.merge`, `capabilities` or `cli` (argument errors) |
| `status` | string | `succeeded`, `planned` (dry run) or `failed` |
| `result` | object or null | Operation-specific result (see each command) |
| `stats` | object | Measurements such as `input_bytes`, `output_bytes`, `pages`, `attempts` and `elapsed_ms`. `elapsed_ms` is informational, not a benchmark. |
| `outputs` | array | `[{"path": "/absolute/path.pdf"}]` for files actually written; empty on failure and dry runs |
| `warnings` | array of strings | Anything you should know, for example signatures invalidated or structures dropped |
| `coverage` | object | What the run did **not** verify: `rendered: false`, `signature_validation: false` |
| `error` | object or null | On failure: `code` (stable), `message` (for people), `hint` (next step) |

Compatibility: new fields may be added in any release, so ignore fields you don't recognize. Renaming or removing a field, or changing its meaning, requires a new `schema_version`.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success, or a successful dry run |
| `1` | Internal or setup failure |
| `2` | Invalid arguments or an output problem (exists, is an input, invalid path) |
| `3` | Input missing, unreadable, not a valid PDF, signed without acknowledgement, or has structures that cannot be preserved |
| `4` | Password required |
| `6` | Size limit exceeded or target unreachable |
| `7` | Processing or writing the output failed |
| `130` | Cancelled (Ctrl-C) |

## Error codes

Match on `error.code`, never on `message`.

| Code | Exit | What happened | What to do |
| --- | --- | --- | --- |
| `INVALID_ARGUMENT` | 2 | Arguments could not be parsed | Check `pdfjiff <command> --help` |
| `OUTPUT_REQUIRED` | 2 | `merge` was run without `-o` | Add `-o combined.pdf` |
| `OUTPUT_EXISTS` | 2 | The output already exists | Choose another path or add `--overwrite` |
| `OUTPUT_IS_INPUT` | 2 | The output refers to an input file | Choose a different output path |
| `INVALID_OUTPUT` | 2 | The output folder is missing, or the output is a directory or symlink | Create the folder, or choose a regular file path |
| `UNSUPPORTED_OUTPUT` | 2 | `-o -` (stdout) was used | Write to a file |
| `UNSUPPORTED_PATH` | 2 | The path is not valid UTF-8 | Rename the file or folder |
| `INPUT_UNAVAILABLE` | 3 | The input cannot be opened or read | Check the path and permissions |
| `INVALID_INPUT` | 3 | The input is not a regular file | Pass a file, not a directory |
| `INVALID_PDF` | 3 | The input is not a readable PDF | Check the file is a valid PDF |
| `SIGNED_PDF` | 3 | The input is signed | Add `--allow-signature-invalidation` if that is intended |
| `UNSUPPORTED_PRESERVATION` | 3 | `merge` would drop document structures | Add `--allow-structure-loss` after reviewing, or use another tool |
| `PASSWORD_REQUIRED` | 4 | The PDF is encrypted | Decrypt a copy first |
| `INPUT_LIMIT` | 6 | An input exceeds the byte limit | Raise `--max-input-mib` / `--max-total-input-mib` if you have enough memory |
| `TARGET_UNREACHABLE` | 6 | Even the most compact attempt is larger than `--target` | Raise the target |
| `PROCESSING_FAILED` | 7 | The PDF could not be processed | Report a bug with the `inspect --json` output |
| `OUTPUT_UNAVAILABLE` | 7 | A temporary output could not be created | Check folder permissions and free space |
| `OUTPUT_WRITE_FAILED` | 7 | Writing or publishing the output failed | Check free space and permissions. The previous output was not replaced. |
| `SIGNAL_SETUP_FAILED` | 1 | The Ctrl-C handler could not be installed | Report a bug |
| `CANCELLED` | 130 | Cancelled by Ctrl-C | Retry when ready |
