use std::process::ExitCode;

/// Reads the program's imports from disk: module `a/b` is the file `a/b.elya` in
/// the program's directory (spec 2026-10-23-elya-modules-part-1 D1, D7).
fn files(program: &str) -> impl FnMut(&str) -> Option<(String, String)> {
    let dir = std::path::Path::new(program)
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    move |module: &str| {
        let file = dir.join(format!("{module}.elya"));
        std::fs::read_to_string(&file)
            .ok()
            .map(|text| (file.display().to_string(), text))
    }
}

/// The compiler's stack: 1 GiB, reserved rather than committed, as native
/// programs run on (`elya_start`). The front end and every Core pass recurse on
/// the host stack -- down an expression (bounded by E0102 at 1024 levels) and
/// down a block's chain of statements (not bounded): an 8 MiB main thread
/// overflowed a debug `elya build` at about 700 statements (spec
/// 2026-10-24-elya-diagnostics-at-the-edges D5).
const COMPILER_STACK: usize = 1 << 30;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let shared = std::sync::Arc::new(args);
    let for_thread = std::sync::Arc::clone(&shared);
    let worker = std::thread::Builder::new()
        .name("elya".to_string())
        .stack_size(COMPILER_STACK)
        .spawn(move || dispatch(&for_thread));
    match worker {
        Ok(handle) => match handle.join() {
            Ok(code) => code,
            // A compiler panic keeps its exit status (101): the panic message was
            // printed on the thread; re-raise it here.
            Err(panic) => std::panic::resume_unwind(panic),
        },
        // No room for the stack (a limit on address space): compile on the main
        // thread, as before D5, rather than refuse (the review's F3).
        Err(_) => dispatch(&shared),
    }
}

fn dispatch(args: &[String]) -> ExitCode {
    match args.get(1).map(String::as_str) {
        Some("run")
            if matches!(
                args.get(2).map(String::as_str),
                Some("--record" | "--replay")
            ) =>
        {
            trace_cmd(args)
        }
        Some("run") if args.get(2).map(String::as_str) == Some("--value") => value_cmd(args),
        Some("run") if args.len() > 3 => {
            eprintln!("usage: elya run <file.elya>  (flags go before the program: elya run --record <trace> <file.elya>)");
            ExitCode::from(2)
        }
        Some("run") => cmd(args, true),
        Some("check") => cmd(args, false),
        #[cfg(feature = "codegen")]
        Some("build") => build_cmd(args),
        _ => {
            eprintln!("usage: elya <run|check|build> <file.elya>");
            eprintln!("       elya run --value <file.elya>");
            eprintln!("       elya run --record <trace> <file.elya>");
            eprintln!("       elya run --replay <trace> <file.elya>");
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
        // D1: the output a failing run produced is printed before its error.
        let o = elya::run_source_outcome_with(path, &text, &mut files(path));
        print!("{}", o.output);
        match o.error {
            None => {
                surface_warnings(path, &text);
                ExitCode::SUCCESS
            }
            Some(diags) => {
                eprint!("{diags}");
                ExitCode::FAILURE
            }
        }
    } else {
        match elya::check_source_with(path, &text, &mut files(path)) {
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

/// `elya run --value <file.elya>` (spec 2026-10-16-elya-differential-sweep,
/// D2): `elya run`, then main's value as the native shim prints it, so an
/// evaluator run and a native run compare byte for byte.
fn value_cmd(args: &[String]) -> ExitCode {
    let (Some(path), None) = (args.get(3), args.get(4)) else {
        eprintln!("usage: elya run --value <file.elya>");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    let o = elya::run_source_outcome_with(path, &text, &mut files(path));
    print!("{}", o.output);
    if let Some(diags) = o.error {
        eprint!("{diags}");
        return ExitCode::FAILURE;
    }
    surface_warnings(path, &text);
    match o.value {
        Some(Ok(n)) => {
            println!("{n}");
            ExitCode::SUCCESS
        }
        Some(Err(why)) => {
            eprintln!("error: {why}");
            ExitCode::FAILURE
        }
        None => ExitCode::FAILURE,
    }
}

/// On a successful compile, render any non-fatal warnings (e.g. `E0426`,
/// `E0431`) to stderr so the lint still reaches the user.
fn surface_warnings(path: &str, text: &str) {
    if let Some(warns) = elya::warnings_with(path, text, &mut files(path)) {
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
    if let Err(diags) = elya::check_source_with(path, &text, &mut files(path)) {
        eprint!("{diags}");
        return ExitCode::FAILURE;
    }
    surface_warnings(path, &text);

    // The check has passed, so loading and inference cannot fail here; the
    // backend needs the merged module and the typed table, which the convenience
    // API does not return.
    let session = elya::Session::new();
    let (module, _, _) = elya::front_end_with(path, &text, &mut files(path));
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    // Warnings (E0426, E0431) are expected here: `check_source` passed them
    // through (the 2026-10-16 sweep found this asserting no diagnostics at all).
    debug_assert!(
        diags
            .iter()
            .all(|d| d.severity != elya::diag::Severity::Error),
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

/// `elya run --record <trace> <file.elya>` and `elya run --replay <trace>
/// <file.elya>` (spec 2026-10-16-elya-cli-record-replay): the trace is the
/// host's answers; a replay reproduces the run or stops by name.
fn trace_cmd(args: &[String]) -> ExitCode {
    // Exactly `run <flag> <trace> <file.elya>`: a flag after the program, an
    // extra argument, or a trace path that is a flag or the program itself
    // is a usage error (the record-replay review's findings).
    let (Some(flag), Some(trace), Some(path), None) =
        (args.get(2), args.get(3), args.get(4), args.get(5))
    else {
        eprintln!("usage: elya run --record|--replay <trace> <file.elya>");
        return ExitCode::from(2);
    };
    if trace.starts_with("--") || trace == path {
        eprintln!("error: `{trace}` is not a trace path (it is a flag or the program)");
        eprintln!("usage: elya run --record|--replay <trace> <file.elya>");
        return ExitCode::from(2);
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    // Spec 2026-10-23-elya-traced-run-output D1: the program's output on stdout,
    // failing or not, then the error on stderr, as `elya run` and a native binary.
    let o = if flag == "--record" {
        let (log, o) = elya::record_source_with(path, &text, &mut files(path));
        // Written even when the run fails: that run is the one to replay.
        if let Some(log) = log {
            if let Err(e) = std::fs::write(trace, log) {
                eprintln!("error: cannot write {trace}: {e}");
                return ExitCode::from(2);
            }
        }
        o
    } else {
        match std::fs::read_to_string(trace) {
            Ok(log) => elya::replay_source_with(path, &text, &log, &mut files(path)),
            Err(e) => {
                eprintln!("error: cannot read {trace}: {e}");
                return ExitCode::from(2);
            }
        }
    };
    print!("{}", o.output);
    match o.error {
        None => {
            surface_warnings(path, &text);
            ExitCode::SUCCESS
        }
        Some(diags) => {
            eprint!("{diags}");
            ExitCode::FAILURE
        }
    }
}
