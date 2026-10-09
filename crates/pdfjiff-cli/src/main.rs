mod args;
mod error;
mod files;

use args::{Cli, Command};
use clap::{CommandFactory, Parser};
use error::{Failure, Result};
use files::{default_output, read_input, Destination};
use pdfjiff_core::{
    compress::{try_compress, CompressOptions},
    inspect,
    pages::PdfMerger,
    Inspection,
};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::PathBuf,
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

fn envelope(
    operation: &str,
    status: &str,
    result: Value,
    stats: Value,
    outputs: Vec<PathBuf>,
    warnings: Vec<String>,
) -> Value {
    json!({"schema_version": 1, "operation": operation, "status": status,
        "result": result, "stats": stats,
        "outputs": outputs.into_iter().map(|path| json!({"path":path})).collect::<Vec<_>>(),
        "warnings": warnings, "coverage": {"rendered":false,"signature_validation":false}, "error":null})
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Failure::new(
            "CANCELLED",
            "Operation cancelled; no new output was published.",
            "Retry when ready.",
            130,
        ))
    } else {
        Ok(())
    }
}
fn validate_write(info: &Inspection, allow_signatures: bool) -> Result<Vec<String>> {
    if info.encrypted {
        return Err(pdfjiff_core::CoreError::PasswordRequired.into());
    }
    if info.signatures_detected && !allow_signatures {
        return Err(Failure::new(
            "SIGNED_PDF",
            "Rewriting this PDF may invalidate its signatures.",
            "Use --allow-signature-invalidation only when that change is intended.",
            3,
        ));
    }
    Ok(if info.signatures_detected {
        vec!["Existing digital signatures may be invalidated.".into()]
    } else {
        vec![]
    })
}
fn required_output(output: &Option<PathBuf>) -> Result<PathBuf> {
    output.clone().ok_or_else(|| {
        Failure::new(
            "OUTPUT_REQUIRED",
            "Merge requires --output.",
            "Example: pdfjiff merge a.pdf b.pdf --output merged.pdf",
            2,
        )
    })
}
fn execute(cli: &Cli, cancel: &AtomicBool) -> Result<Value> {
    check_cancel(cancel)?;
    let limit = cli.max_input_mib * 1024 * 1024;
    match cli.command.as_ref().expect("handled help before execution") {
        Command::Completions { .. } => unreachable!("completions are handled before execution"),
        Command::Capabilities => Ok(envelope(
            "capabilities",
            "succeeded",
            json!({
                "version":env!("CARGO_PKG_VERSION"), "stage":"native-prerelease", "network_required":false,
                "operations":[
                    {"id":"document.inspect","ready":true,"limitations":["structure only; no render or signature validation"]},
                    {"id":"pdf.compress","ready":true,"features":["presets","lossless structure optimization","bounded target search"],"limitations":["no raster fallback","encrypted inputs require prior decryption"]},
                    {"id":"pdf.merge","ready":true,"limitations":["document-level structures may require --allow-structure-loss","metadata is not combined"]}
                ],
                "not_implemented":["rendering","OCR","server","dashboard","pipeline","batch","stdin/stdout artifacts","password input"],
                "cancellation":"between native processing stages; no hard deadline within a decode",
                "limits":{"max_input_mib":cli.max_input_mib,"max_total_input_mib":cli.max_total_input_mib,"hard_rss_limit":false}
            }),
            json!({}),
            vec![],
            vec![],
        )),
        Command::Inspect { input } => {
            let data = read_input(input, limit)?;
            let info = inspect(&data)?;
            check_cancel(cancel)?;
            Ok(envelope(
                "document.inspect",
                "succeeded",
                json!(info),
                json!({"input_bytes":data.len()}),
                vec![],
                vec![],
            ))
        }
        Command::Compress(options) => {
            let output = options
                .write
                .output
                .clone()
                .unwrap_or_else(|| default_output(&options.input));
            let dest = Destination::prepare(
                &output,
                std::slice::from_ref(&options.input),
                options.write.overwrite,
                options.write.dry_run,
            )?;
            let data = read_input(&options.input, limit)?;
            let info = inspect(&data)?;
            let warnings = validate_write(&info, options.write.allow_signature_invalidation)?;
            if options.write.dry_run {
                return Ok(envelope(
                    "pdf.compress",
                    "planned",
                    json!({"output":dest.path(),"target_bytes":options.target,"lossless":options.lossless}),
                    json!({"input_bytes":data.len()}),
                    vec![],
                    warnings,
                ));
            }
            let (preset, quality, edge) = options.preset.settings();
            let settings = if options.lossless {
                CompressOptions::lossless()
            } else {
                CompressOptions::from_preset(preset)
            };
            check_cancel(cancel)?;
            let mut best = try_compress(&data, &settings).map_err(Failure::processing)?;
            let mut attempts = 1;
            // Explicit bounded search; never rasterize or strip document structures.
            if let Some(target) = options.target {
                if !options.lossless && u64::from(best.compressed_size()) > target {
                    for (q, max_edge) in [(65, 1800), (50, 1400), (35, 1000), (25, 800), (15, 600)]
                    {
                        if q >= quality || max_edge >= edge {
                            continue;
                        }
                        check_cancel(cancel)?;
                        let candidate = try_compress(
                            &data,
                            &CompressOptions::new(preset, q, max_edge, false, false, false, false),
                        )
                        .map_err(Failure::processing)?;
                        attempts += 1;
                        if candidate.compressed_size() < best.compressed_size() {
                            best = candidate;
                        }
                        if u64::from(best.compressed_size()) <= target {
                            break;
                        }
                    }
                }
                if u64::from(best.compressed_size()) > target {
                    return Err(Failure::new("TARGET_UNREACHABLE", format!("Requested {target} bytes; smallest result was {} bytes after {attempts} attempts. No output was published.", best.compressed_size()), "Increase the target or choose another workflow. Rasterization is not performed implicitly.", 6));
                }
            }
            let stats = json!({"input_bytes":data.len(),"output_bytes":best.compressed_size(),"pages":best.page_count(),"images_recompressed":best.images_recompressed(),"images_downsampled":best.images_downsampled(),"attempts":attempts,"target_bytes":options.target,"target_reached":options.target.map(|_|true)});
            let bytes = best.take();
            let output_info = inspect(&bytes)?;
            if output_info.page_count != info.page_count {
                return Err(Failure::processing(
                    "Output page count changed unexpectedly.",
                ));
            }
            check_cancel(cancel)?;
            let path = dest.publish(&bytes)?;
            Ok(envelope(
                "pdf.compress",
                "succeeded",
                json!({"unchanged":bytes==data}),
                stats,
                vec![path],
                warnings,
            ))
        }
        Command::Merge(options) => {
            let output = required_output(&options.write.output)?;
            let dest = Destination::prepare(
                &output,
                &options.inputs,
                options.write.overwrite,
                options.write.dry_run,
            )?;
            let mut merger = PdfMerger::try_new().map_err(Failure::processing)?;
            let mut warnings =
                vec!["Merge assembles pages; document metadata is not combined.".to_owned()];
            let mut input_bytes = 0_u64;
            let mut page_count = 0_u64;
            for input in &options.inputs {
                check_cancel(cancel)?;
                let remaining = cli.max_total_input_mib * 1024 * 1024 - input_bytes;
                let data = read_input(input, limit.min(remaining))?;
                input_bytes += data.len() as u64;
                let info = inspect(&data)?;
                warnings.extend(validate_write(
                    &info,
                    options.write.allow_signature_invalidation,
                )?);
                if !info.assembly_sensitive_features.is_empty() {
                    let description = format!(
                        "{} contains document-level {} that this merger cannot preserve.",
                        input.display(),
                        info.assembly_sensitive_features.join(", ")
                    );
                    if !options.allow_structure_loss {
                        return Err(Failure::new("UNSUPPORTED_PRESERVATION", description, "Use --allow-structure-loss only after reviewing the reported loss, or use another merger.", 3));
                    }
                    warnings.push(description);
                }
                page_count += u64::from(info.page_count.unwrap_or(0));
                if !options.write.dry_run {
                    merger
                        .try_add_document(&data)
                        .map_err(Failure::processing)?;
                }
            }
            if options.write.dry_run {
                return Ok(envelope(
                    "pdf.merge",
                    "planned",
                    json!({"output":dest.path(),"input_order":options.inputs}),
                    json!({"input_bytes":input_bytes,"pages":page_count}),
                    vec![],
                    warnings,
                ));
            }
            check_cancel(cancel)?;
            let bytes = merger.try_finish().map_err(Failure::processing)?;
            let info = inspect(&bytes)?;
            if info.page_count.map(u64::from) != Some(page_count) {
                return Err(Failure::processing(
                    "Merged output page count did not match inputs.",
                ));
            }
            check_cancel(cancel)?;
            let path = dest.publish(&bytes)?;
            Ok(envelope(
                "pdf.merge",
                "succeeded",
                json!({"input_order":options.inputs}),
                json!({"input_bytes":input_bytes,"output_bytes":bytes.len(),"pages":page_count}),
                vec![path],
                warnings,
            ))
        }
    }
}
fn operation(cli: &Cli) -> &'static str {
    match cli.command.as_ref() {
        Some(Command::Inspect { .. }) => "document.inspect",
        Some(Command::Compress(_)) => "pdf.compress",
        Some(Command::Merge(_)) => "pdf.merge",
        _ => "capabilities",
    }
}
fn emit(value: &Value, json_output: bool) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    if json_output {
        serde_json::to_writer(&mut stdout, value)?;
        writeln!(stdout)?;
    } else {
        writeln!(
            stdout,
            "{}: {}",
            value["operation"].as_str().unwrap_or("pdfjiff"),
            value["status"].as_str().unwrap_or("unknown")
        )?;
        if let Some(outputs) = value["outputs"].as_array() {
            for output in outputs {
                writeln!(stdout, "Output: {}", output["path"].as_str().unwrap_or(""))?;
            }
        }
        if value["operation"] == "document.inspect"
            || value["operation"] == "capabilities"
            || value["status"] == "planned"
        {
            writeln!(
                stdout,
                "{}",
                serde_json::to_string_pretty(&value["result"])?
            )?;
        } else {
            writeln!(stdout, "{}", serde_json::to_string_pretty(&value["stats"])?)?;
        }
        if let Some(warnings) = value["warnings"].as_array() {
            for warning in warnings {
                eprintln!("Warning: {}", warning.as_str().unwrap_or(""));
            }
        }
    }
    Ok(())
}
fn fail(failure: Failure, op: &str, json_output: bool) -> ExitCode {
    let code = failure.exit;
    if json_output {
        let mut result = envelope(op, "failed", Value::Null, json!({}), vec![], vec![]);
        result["error"] = json!(failure);
        if emit(&result, true).is_err() {
            return ExitCode::from(1);
        }
    } else {
        eprintln!("{}: {}\n{}", failure.code, failure.message, failure.hint);
    }
    ExitCode::from(code)
}
fn main() -> ExitCode {
    let started = Instant::now();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if error.use_stderr() {
                return fail(
                    Failure::new(
                        "INVALID_ARGUMENT",
                        error.to_string(),
                        "Run pdfjiff --help or pdfjiff COMMAND --help.",
                        2,
                    ),
                    "cli",
                    std::env::args_os().any(|a| a == "--json"),
                );
            }
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
    };
    if cli.command.is_none() {
        let _ = Cli::command().print_help();
        println!();
        return ExitCode::SUCCESS;
    }
    if let Some(Command::Completions { shell }) = &cli.command {
        let mut script = Vec::new();
        clap_complete::generate(*shell, &mut Cli::command(), "pdfjiff", &mut script);
        return match std::io::stdout().lock().write_all(&script) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::from(1),
        };
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let handler = Arc::clone(&cancel);
    if let Err(error) = ctrlc::set_handler(move || handler.store(true, Ordering::Relaxed)) {
        return fail(
            Failure::new(
                "SIGNAL_SETUP_FAILED",
                error.to_string(),
                "Check whether another signal handler is installed.",
                1,
            ),
            operation(&cli),
            cli.json,
        );
    }
    match execute(&cli, &cancel) {
        Ok(mut result) => {
            result["stats"]["elapsed_ms"] = json!(started.elapsed().as_secs_f64() * 1000.0);
            if emit(&result, cli.json).is_err() {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => fail(error, operation(&cli), cli.json),
    }
}
