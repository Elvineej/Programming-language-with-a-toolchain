use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => cmd(&args, true),
        Some("check") => cmd(&args, false),
        _ => {
            eprintln!("usage: lyra <run|check> <file.lyra>");
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
        match lyra::run_source(path, &text) {
            Ok(out) => {
                print!("{out}");
                ExitCode::SUCCESS
            }
            Err(diags) => {
                eprint!("{diags}");
                ExitCode::FAILURE
            }
        }
    } else {
        match lyra::check_source(path, &text) {
            Ok(()) => {
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
