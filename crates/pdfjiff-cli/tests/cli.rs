use lopdf::{dictionary, Document, Object, Stream};
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn fixture(path: &Path, widths: &[i64], feature: Option<&str>) {
    let mut doc = Document::with_version("1.7");
    let root = doc.new_object_id();
    let font =
        doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let mut pages = Vec::new();
    for width in widths {
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"BT /F1 12 Tf 20 20 Td (PDFJiff fixture) Tj ET".to_vec(),
        ));
        let page = doc.add_object(dictionary! {"Type"=>"Page", "Parent"=>root,
        "MediaBox"=> vec![0.into(),0.into(),(*width).into(),400.into()], "Contents"=>content,
        "Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}}});
        pages.push(Object::Reference(page));
    }
    doc.objects.insert(
        root,
        Object::Dictionary(
            dictionary! {"Type"=>"Pages","Kids"=>pages,"Count"=>widths.len() as i64},
        ),
    );
    let mut catalog = dictionary! {"Type"=>"Catalog","Pages"=>root};
    if feature == Some("forms") {
        catalog.set("AcroForm", dictionary! {"Fields"=>Vec::<Object>::new()});
    }
    if feature == Some("signed") {
        doc.add_object(dictionary!{"Type"=>"Sig","ByteRange"=>vec![0.into(),1.into(),2.into(),3.into()],"Contents"=>Object::string_literal("fixture")});
    }
    let catalog = doc.add_object(catalog);
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}
fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pdfjiff"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}
fn json(output: &Output, exit: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("one JSON result");
    assert_eq!(result["schema_version"], 1);
    result
}
fn setup() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fixture(&dir.path().join("a.pdf"), &[200, 300], None);
    fixture(&dir.path().join("b.pdf"), &[500], None);
    dir
}
#[test]
fn inspect_reports_real_geometry() {
    let dir = setup();
    let result = json(&run(dir.path(), &["inspect", "a.pdf", "--json"]), 0);
    assert_eq!(result["result"]["page_count"], 2);
    assert_eq!(result["result"]["pages"][0]["width_pt"], 200.0);
    assert_eq!(result["result"]["encrypted"], false);
}
#[test]
fn merge_preserves_input_order_and_never_changes_inputs() {
    let dir = setup();
    let original = fs::read(dir.path().join("a.pdf")).unwrap();
    let result = json(
        &run(
            dir.path(),
            &[
                "merge",
                "b.pdf",
                "a.pdf",
                "--output",
                "merged.pdf",
                "--json",
            ],
        ),
        0,
    );
    assert_eq!(result["stats"]["pages"], 3);
    let merged = fs::read(dir.path().join("merged.pdf")).unwrap();
    let info = pdfjiff_core::inspect(&merged).unwrap();
    assert_eq!(
        info.pages.iter().map(|p| p.width_pt).collect::<Vec<_>>(),
        vec![500., 200., 300.]
    );
    assert_eq!(fs::read(dir.path().join("a.pdf")).unwrap(), original);
}
#[test]
fn lossless_compression_has_safe_defaults_and_preserves_text() {
    let dir = setup();
    let original = fs::read(dir.path().join("a.pdf")).unwrap();
    let result = json(
        &run(dir.path(), &["compress", "a.pdf", "--lossless", "--json"]),
        0,
    );
    let bytes = fs::read(dir.path().join("a-compressed.pdf")).unwrap();
    assert!(bytes.len() <= original.len());
    assert_eq!(
        Document::load_mem(&bytes)
            .unwrap()
            .extract_text(&[1])
            .unwrap(),
        Document::load_mem(&original)
            .unwrap()
            .extract_text(&[1])
            .unwrap()
    );
    assert_eq!(result["stats"]["images_recompressed"], 0);
    assert_eq!(fs::read(dir.path().join("a.pdf")).unwrap(), original);
}
#[test]
fn target_failure_does_not_replace_existing_output_or_leave_temp_files() {
    let dir = setup();
    fs::write(dir.path().join("keep.pdf"), b"keep me").unwrap();
    let result = json(
        &run(
            dir.path(),
            &[
                "compress",
                "a.pdf",
                "--target",
                "1B",
                "--output",
                "keep.pdf",
                "--overwrite",
                "--json",
            ],
        ),
        6,
    );
    assert_eq!(result["error"]["code"], "TARGET_UNREACHABLE");
    assert_eq!(fs::read(dir.path().join("keep.pdf")).unwrap(), b"keep me");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
}
#[test]
fn target_units_and_dry_run_are_explicit() {
    let dir = setup();
    for (unit, expected) in [("2MB", 2_000_000), ("2MiB", 2_097_152), ("200KiB", 204_800)] {
        let result = json(
            &run(
                dir.path(),
                &["compress", "a.pdf", "--target", unit, "--dry-run", "--json"],
            ),
            0,
        );
        assert_eq!(result["status"], "planned");
        assert_eq!(result["result"]["target_bytes"], expected);
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[test]
fn existing_output_requires_explicit_permission() {
    let dir = setup();
    fs::write(dir.path().join("out.pdf"), b"existing").unwrap();
    let result = json(
        &run(
            dir.path(),
            &["compress", "a.pdf", "--output", "out.pdf", "--json"],
        ),
        2,
    );
    assert_eq!(result["error"]["code"], "OUTPUT_EXISTS");
    json(
        &run(
            dir.path(),
            &[
                "compress",
                "a.pdf",
                "--output",
                "out.pdf",
                "--overwrite",
                "--json",
            ],
        ),
        0,
    );
    assert!(Document::load(dir.path().join("out.pdf")).is_ok());
}
#[test]
fn input_and_hardlink_aliases_are_refused_even_with_overwrite() {
    let dir = setup();
    fs::hard_link(dir.path().join("a.pdf"), dir.path().join("alias.pdf")).unwrap();
    let before = fs::read(dir.path().join("a.pdf")).unwrap();
    for path in ["a.pdf", "alias.pdf"] {
        let result = json(
            &run(
                dir.path(),
                &[
                    "compress",
                    "a.pdf",
                    "--output",
                    path,
                    "--overwrite",
                    "--json",
                ],
            ),
            2,
        );
        assert_eq!(result["error"]["code"], "OUTPUT_IS_INPUT");
    }
    assert_eq!(fs::read(dir.path().join("a.pdf")).unwrap(), before);
}
#[cfg(unix)]
#[test]
fn output_symlinks_are_refused() {
    let dir = setup();
    std::os::unix::fs::symlink("a.pdf", dir.path().join("alias.pdf")).unwrap();
    let result = json(
        &run(
            dir.path(),
            &[
                "compress",
                "a.pdf",
                "--output",
                "alias.pdf",
                "--overwrite",
                "--json",
            ],
        ),
        2,
    );
    assert_eq!(result["error"]["code"], "OUTPUT_IS_INPUT");
}
#[test]
fn invalid_and_missing_input_have_stable_errors() {
    let dir = setup();
    fs::write(dir.path().join("bad.pdf"), b"not a PDF").unwrap();
    assert_eq!(
        json(&run(dir.path(), &["inspect", "bad.pdf", "--json"]), 3)["error"]["code"],
        "INVALID_PDF"
    );
    assert_eq!(
        json(&run(dir.path(), &["inspect", "missing.pdf", "--json"]), 3)["error"]["code"],
        "INPUT_UNAVAILABLE"
    );
}
#[test]
fn invalid_arguments_are_json_not_mixed_output() {
    let dir = setup();
    for args in [
        vec!["compress", "a.pdf", "--target", "0", "--json"],
        vec!["compress", "a.pdf", "--target", "2GB", "--json"],
        vec!["serve", "--json"],
    ] {
        let output = run(dir.path(), &args);
        assert_eq!(json(&output, 2)["error"]["code"], "INVALID_ARGUMENT");
        assert!(output.stderr.is_empty());
    }
}
#[test]
fn merge_requires_named_output() {
    let dir = setup();
    assert_eq!(
        json(&run(dir.path(), &["merge", "a.pdf", "b.pdf", "--json"]), 2)["error"]["code"],
        "OUTPUT_REQUIRED"
    );
}
#[test]
fn structures_are_rejected_or_explicitly_reported() {
    let dir = setup();
    fixture(&dir.path().join("form.pdf"), &[200], Some("forms"));
    let base = [
        "merge", "form.pdf", "b.pdf", "--output", "out.pdf", "--json",
    ];
    assert_eq!(
        json(&run(dir.path(), &base), 3)["error"]["code"],
        "UNSUPPORTED_PRESERVATION"
    );
    assert!(!dir.path().join("out.pdf").exists());
    let result = json(
        &run(
            dir.path(),
            &[
                "merge",
                "form.pdf",
                "b.pdf",
                "--output",
                "out.pdf",
                "--allow-structure-loss",
                "--json",
            ],
        ),
        0,
    );
    assert!(result["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().unwrap().contains("forms")));
}
#[test]
fn signed_document_requires_acknowledgement() {
    let dir = setup();
    fixture(&dir.path().join("signed.pdf"), &[200], Some("signed"));
    assert_eq!(
        json(&run(dir.path(), &["compress", "signed.pdf", "--json"]), 3)["error"]["code"],
        "SIGNED_PDF"
    );
    let result = json(
        &run(
            dir.path(),
            &[
                "compress",
                "signed.pdf",
                "--allow-signature-invalidation",
                "--json",
            ],
        ),
        0,
    );
    assert!(!result["warnings"].as_array().unwrap().is_empty());
}
#[test]
fn input_limit_is_enforced_before_pdf_parsing() {
    let dir = setup();
    fs::write(dir.path().join("large.pdf"), vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert_eq!(
        json(
            &run(
                dir.path(),
                &["inspect", "large.pdf", "--max-input-mib", "1", "--json"]
            ),
            6
        )["error"]["code"],
        "INPUT_LIMIT"
    );
}
#[test]
fn capabilities_are_truthful_and_offline() {
    let dir = setup();
    let result = json(&run(dir.path(), &["capabilities", "--json"]), 0);
    assert_eq!(result["result"]["network_required"], false);
    assert_eq!(result["result"]["operations"].as_array().unwrap().len(), 3);
    assert!(result["result"]["not_implemented"]
        .as_array()
        .unwrap()
        .contains(&Value::String("server".into())));
}
#[test]
fn paths_with_spaces_and_unicode_work() {
    let dir = setup();
    fs::rename(dir.path().join("a.pdf"), dir.path().join("café report.pdf")).unwrap();
    json(
        &run(
            dir.path(),
            &[
                "compress",
                "café report.pdf",
                "--output",
                "result one.pdf",
                "--json",
            ],
        ),
        0,
    );
    assert!(dir.path().join("result one.pdf").exists());
}

#[cfg(unix)]
#[test]
fn non_unicode_paths_return_structured_errors() {
    use std::os::unix::ffi::OsStringExt;
    let dir = setup();
    let name = std::ffi::OsString::from_vec(b"bad-\xff.pdf".to_vec());
    let input = dir.path().join(&name);
    let output = Command::new(env!("CARGO_BIN_EXE_pdfjiff"))
        .args(["inspect", "--json"])
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(json(&output, 2)["error"]["code"], "UNSUPPORTED_PATH");
}

#[test]
fn merge_requires_acknowledgement_for_page_annotations() {
    let dir = setup();
    let path = dir.path().join("a.pdf");
    let mut doc = Document::load(&path).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", Vec::<Object>::new());
    doc.save(&path).unwrap();
    let output = run(
        dir.path(),
        &["merge", "a.pdf", "b.pdf", "--output", "out.pdf", "--json"],
    );
    json(&output, 3);
    assert!(!dir.path().join("out.pdf").exists());
}
#[test]
fn completions_are_generated_for_common_shells() {
    let dir = setup();
    for (shell, marker) in [
        ("bash", "_pdfjiff()"),
        ("zsh", "#compdef pdfjiff"),
        ("fish", "complete -c pdfjiff"),
        ("powershell", "Register-ArgumentCompleter"),
    ] {
        let output = run(dir.path(), &["completions", shell]);
        assert_eq!(output.status.code(), Some(0), "{shell}");
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(script.contains(marker), "{shell} script lacks {marker}");
        assert!(
            script.contains("compress"),
            "{shell} script lacks subcommands"
        );
    }
    let output = run(dir.path(), &["completions", "tcsh"]);
    assert_eq!(output.status.code(), Some(2));
}
#[cfg(unix)]
#[test]
fn outputs_get_ordinary_permissions_and_replacements_keep_theirs() {
    use std::os::unix::fs::PermissionsExt;
    let dir = setup();
    let mode = |name: &str| {
        fs::metadata(dir.path().join(name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    // A file written the ordinary way shows what the current umask allows.
    fs::write(dir.path().join("reference.txt"), b"x").unwrap();
    json(
        &run(
            dir.path(),
            &["merge", "a.pdf", "b.pdf", "-o", "new.pdf", "--json"],
        ),
        0,
    );
    assert_eq!(mode("new.pdf"), mode("reference.txt"));

    let replaced = dir.path().join("replaced.pdf");
    fs::write(&replaced, b"old").unwrap();
    fs::set_permissions(&replaced, fs::Permissions::from_mode(0o640)).unwrap();
    let args = [
        "merge",
        "a.pdf",
        "b.pdf",
        "-o",
        "replaced.pdf",
        "--overwrite",
        "--json",
    ];
    json(&run(dir.path(), &args), 0);
    assert_eq!(mode("replaced.pdf"), 0o640);
}
