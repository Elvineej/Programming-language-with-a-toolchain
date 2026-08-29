//! The accessor that makes the native back end's differential check possible
//! (5b-2 §5): `elya run` observes only `io.println` output, so nothing public
//! could say what `main` evaluated to. `run_module_value` says it — and
//! `run_module` must keep behaving exactly as before.

use elya::eval::{run_module, run_module_value, Value};
use elya::parse::parse_module;
use elya::Session;

fn module_of(src: &str) -> elya::ast::Module {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    m
}

#[test]
fn run_module_value_reports_what_main_evaluated_to() {
    let m = module_of("pub fn main() { (2 + 3) * 4 - 5 }\n");
    let (_, v) = run_module_value(&m).expect("evaluates");
    assert_eq!(v, Value::Int(15));
}

#[test]
fn run_module_value_reports_non_int_results_too() {
    let m = module_of("pub fn main() { 1 < 2 }\n");
    let (_, v) = run_module_value(&m).expect("evaluates");
    assert_eq!(v, Value::Bool(true));
}

#[test]
fn run_module_still_observes_only_println_output() {
    // The whole point of the accessor is that it is additive: `elya run` sees
    // exactly what it saw before, so the differential check stays non-circular.
    let m = module_of("pub fn main() {\n  io.println(\"hi\")\n  1 + 2\n}\n");
    let interp = run_module(&m).expect("evaluates");
    assert_eq!(interp.output(), "hi\n");

    let (interp2, v) = run_module_value(&m).expect("evaluates");
    assert_eq!(
        interp2.output(),
        interp.output(),
        "same machine, same output"
    );
    assert_eq!(v, Value::Int(3), "and the value the output never showed");
}

#[test]
fn run_module_value_surfaces_runtime_errors_unchanged() {
    let m = module_of("pub fn notmain() { 1 }\n");
    assert!(run_module(&m).is_err());
    assert!(run_module_value(&m).is_err(), "same failure, same door");
}
