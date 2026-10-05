//! A CEK machine written in Elya (`examples/03_cek.elya`), run at depth by the
//! reference evaluator. The example's own `main` stays small because the
//! tree-walker cross-check (`tests/crosscheck.rs`) recurses on the host stack;
//! here the CEK evaluator alone runs it, so the object program can recurse
//! thousands deep (its continuation lives on the heap, in the machine's `Kont`).

use elya::eval::{run_module_value, Value};
use elya::parse::parse_module;
use elya::Session;

/// The machine and its programs, without the example's `main`.
fn machine() -> &'static str {
    let src = include_str!("../examples/03_cek.elya");
    &src[..src.find("pub fn main").expect("example has a main")]
}

fn run_main(main: &str) -> Value {
    let src = format!("{}{main}", machine());
    assert!(
        elya::check_source("cek.elya", &src).is_ok(),
        "{:?}",
        elya::check_source("cek.elya", &src)
    );
    let (m, pd) = parse_module(&Session::new(), &src);
    assert!(pd.is_empty(), "{pd:?}");
    run_module_value(&m).expect("runs").1
}

#[test]
fn the_elya_cek_machine_runs_its_three_programs() {
    assert_eq!(
        run_main("pub fn main() -> Int { run(add_one()) }\n"),
        Value::Int(42)
    );
    assert_eq!(
        run_main("pub fn main() -> Int { run(twice_add3()) }\n"),
        Value::Int(16)
    );
    assert_eq!(
        run_main("pub fn main() -> Int { run(sum_to(10)) }\n"),
        Value::Int(55)
    );
}

#[test]
fn the_elya_cek_machine_recurses_two_thousand_deep() {
    // sum 1..2000 through the Z combinator: 2,001,000. The object program's
    // recursion is NOT tail recursive -- each level waits on an `AddLeft`
    // frame in the machine's continuation -- so this pins that `Kont` carries
    // the depth, not the host.
    assert_eq!(
        run_main("pub fn main() -> Int { run(sum_to(2000)) }\n"),
        Value::Int(2_001_000)
    );
}
