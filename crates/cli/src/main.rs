use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => cmd(&args, true),
        Some("check") => cmd(&args, false),
        #[cfg(feature = "codegen")]
        Some("build") => build_cmd(&args),
        _ => {
            eprintln!("usage: elya <run|check|build> <file.elya>");
            ExitCode::from(2)
        }
    }
}

fn cmd(args: &[String], run: bool) -> ExitCode {
    let Some(path) = args.get(2) else {
        eprintln!("error: missing file path");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    if run {
        match elya::run_source(path, &text) {
            Ok(out) => {
                surface_warnings(path, &text);
                print!("{out}");
                ExitCode::SUCCESS
            }
            Err(diags) => {
                eprint!("{diags}");
                ExitCode::FAILURE
            }
        }
    } else {
        match elya::check_source(path, &text) {
            Ok(()) => {
                surface_warnings(path, &text);
                println!("ok");
                ExitCode::SUCCESS
            }
            Err(diags) => {
                eprint!("{diags}");
                ExitCode::FAILURE
            }
        }
    }
}

/// On a successful compile, render any non-fatal warnings (e.g. `E0426`,
/// `E0431`) to stderr so the lint still reaches the user.
fn surface_warnings(path: &str, text: &str) {
    if let Some(warns) = elya::warnings(path, text) {
        eprint!("{warns}");
    }
}

/// `elya build <file.elya> [-o <out>]` (Slice 5b-1 §7): the front end runs
/// exactly as `elya check` — same diagnostics, same warning surfacing — and only
/// then does the program go to Core, to an object, and through clang. Default
/// output is the input stem plus the platform exe suffix.
#[cfg(feature = "codegen")]
fn build_cmd(args: &[String]) -> ExitCode {
    let Some(path) = args.get(2) else {
        eprintln!("error: missing file path");
        return ExitCode::from(2);
    };
    let out_path = match (args.get(3).map(String::as_str), args.get(4)) {
        (Some("-o"), Some(o)) => Some(std::path::PathBuf::from(o)),
        (None, _) => None,
        _ => {
            eprintln!("usage: elya build <file.elya> [-o <out>]");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };

    // Front end first: a program that does not `check` never reaches codegen.
    if let Err(diags) = elya::check_source(path, &text) {
        eprint!("{diags}");
        return ExitCode::FAILURE;
    }
    surface_warnings(path, &text);

    // `check_source` has already passed, so parse and inference cannot fail here;
    // the backend needs the typed table, which the convenience API does not return.
    let session = elya::Session::new();
    let (module, pd) = elya::parse::parse_module(&session, &text);
    debug_assert!(
        pd.is_empty(),
        "check_source passed but parse failed: {pd:?}"
    );
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    debug_assert!(
        diags.is_empty(),
        "check_source passed but inference errored: {diags:?}"
    );
    let core = match elya::core::lower_module(&module, &table) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: core lowering failed: {e:?}");
            return ExitCode::FAILURE;
        }
    };

    let exe = out_path.unwrap_or_else(|| {
        std::path::PathBuf::from(path).with_extension(std::env::consts::EXE_EXTENSION)
    });
    let obj = std::env::temp_dir().join(format!(
        "elya-build-{}-{}.o",
        std::process::id(),
        exe.file_stem().and_then(|s| s.to_str()).unwrap_or("out")
    ));
    if let Err(e) = elya_codegen::compile_module(&core, &obj) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    let linked = elya_codegen::link(&obj, &exe);
    let _ = std::fs::remove_file(&obj);
    if let Err(e) = linked {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    println!("built {}", exe.display());
    ExitCode::SUCCESS
}
