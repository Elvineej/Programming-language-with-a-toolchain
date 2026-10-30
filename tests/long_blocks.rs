//! A block's length is not a stack bound (2026-10-30, spec
//! `docs/superpowers/specs/2026-10-30-elya-long-blocks-design.md` D2): a long
//! `main` is checked, lowered to Core, dropped and run on a small thread.
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K1a, `CoreExpr`'s `Drop` back to the derived one: the thread overflows
//!   dropping the lowered module (it did at 60 000 statements on 8 MiB);
//! - K1b, the evaluator's `Scope` `Drop` back to the derived one: the thread
//!   overflows dropping the environment chain after the run.

fn no_files(_: &str) -> Option<(String, String)> {
    None
}

/// `n` statements, each using the one before, so none of them is dead.
fn long_main(n: usize) -> String {
    let mut src = String::from("pub fn main() -> Int {\n  let a0 = 1\n");
    for i in 1..n {
        src.push_str(&format!("  let a{i} = (a{} * 3 + {i}) % 1000003\n", i - 1));
    }
    src.push_str(&format!("  a{}\n}}\n", n - 1));
    src
}

#[test]
fn a_long_block_is_lowered_dropped_and_run_on_a_small_thread() {
    let n = 100_000;
    let src = long_main(n);
    let mut want: i64 = 1;
    for i in 1..n as i64 {
        want = (want * 3 + i) % 1_000_003;
    }
    let got = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let (module, diags, _) = elya::front_end_with("t.elya", &src, &mut no_files);
            assert!(diags.is_empty(), "{diags:?}");
            let session = elya::Session::new();
            let (diags, table) = elya::types::infer_typed_table(&session, &module);
            assert!(diags.is_empty(), "{diags:?}");
            let core = elya::core::lower_module(&module, &table).expect("lowers");
            assert_eq!(core.fns.len(), 1);
            drop(core);
            let (_, r) = elya::eval::run_module_keeping(&module, elya::eval::World::Live);
            match r {
                Ok(elya::eval::Value::Int(v)) => v,
                other => panic!("main: {other:?}"),
            }
        })
        .expect("a thread")
        .join()
        .expect("no panic");
    assert_eq!(got, want);
}
