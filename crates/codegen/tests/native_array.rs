//! Linear arrays natively (2026-11-14, spec
//! `docs/superpowers/specs/2026-11-14-elya-arrays-and-borrowing-design.md`):
//! the evaluator's output, in place, the elements that are heap values kept
//! by the collector with a collection at every allocation.
//!
//! Negative controls: see the plan,
//! `docs/superpowers/plans/2026-11-14-elya-arrays-and-borrowing.md`.

use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)] // each suite uses some of the programs
#[path = "../../../tests/programs/array.rs"]
mod programs;
use programs::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-narray-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn build(src: &str, dir: &Path, tag: &str) -> PathBuf {
    let (m, pd, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    assert!(pd.is_empty(), "front end: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let core = elya::core::lower_module(&m, &table).expect("lowers");
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).expect("compile_module");
    elya_codegen::link(&obj, &exe).expect("link");
    exe
}

#[test]
fn arrays_natively_are_the_evaluators_under_a_collection_at_every_allocation() {
    let dir = temp_dir("same");
    let exe = build(ARRAYS, &dir, "arrays");
    let want = format!("{ARRAYS_OUT}{ARRAYS_VALUE}\n");
    for stress in [None, Some("1")] {
        let mut c = Command::new(&exe);
        if let Some(s) = stress {
            c.env("ELYA_GC_STRESS", s);
        }
        let out = c.output().expect("run");
        assert!(out.status.success(), "{stress:?}: {out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{stress:?}");
    }
    let o = elya::run_source_outcome_with("t.elya", ARRAYS, &mut |_| None);
    assert_eq!(format!("{}{}\n", o.output, o.line.unwrap().unwrap()), want);
    std::fs::remove_dir_all(&dir).ok();
}

/// What an array of heap values holds is kept while the program allocates
/// (strings made after the array, a collection at every allocation), through
/// growths and a `set`; what it no longer holds is not.
#[test]
fn the_collector_follows_an_arrays_elements() {
    let src = r#"fn fill(xs: Array(String), i: Int, n: Int) -> Array(String) {
  if i == n { xs } else { fill(array.push(xs, "s" <> int.to_string(i)), i + 1, n) }
}
fn churn(i: Int, acc: Int) -> Int {
  if i == 0 { acc } else { churn(i - 1, acc + string.length("junk" <> int.to_string(i))) }
}
fn joined(borrow xs: Array(String), i: Int, acc: Int) -> Int {
  if i == array.length(xs) { acc } else { joined(xs, i + 1, acc + string.length(array.get(xs, i) <> "!")) }
}
pub fn main() -> Int {
  let xs = fill(array.new(), 0, 300)
  let a = churn(500, 0)
  let xs = array.set(xs, 0, "replaced" <> int.to_string(a))
  let b = churn(500, 0)
  io.println(array.get(xs, 0) <> " " <> array.get(xs, 299))
  joined(xs, 0, 0) + b
}
"#;
    let dir = temp_dir("traced");
    let exe = build(src, &dir, "traced");
    let o = elya::run_source_outcome_with("t.elya", src, &mut |_| None);
    let want = format!("{}{}\n", o.output, o.line.unwrap().unwrap());
    assert!(want.starts_with("replaced3392 s299\n"), "{want}");
    for stress in ["0", "1"] {
        let out = Command::new(&exe)
            .env("ELYA_GC_STRESS", stress)
            .output()
            .expect("run");
        assert!(out.status.success(), "{stress}: {out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{stress}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_native_index_out_of_range_has_the_evaluators_words() {
    let dir = temp_dir("range");
    for (i, (src, message)) in OUT_OF_RANGE.iter().enumerate() {
        let out = Command::new(build(src, &dir, &format!("r{i}")))
            .output()
            .expect("run");
        assert_eq!(out.status.code(), Some(1), "{src}: {out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr).trim_end(),
            format!("elya: {message}"),
            "{src}"
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim_end(),
            "before",
            "{src}"
        );
        let e = elya::run_source_outcome_with("t.elya", src, &mut |_| None)
            .error
            .expect("fails");
        assert!(e.contains(message), "{src}: the evaluator: {e}");
    }
    // As main's value an array stops by name, the evaluator's words.
    let src = "pub fn main() -> Array(Int) { array.push(array.new(), 1) }\n";
    let out = Command::new(build(src, &dir, "main"))
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim_end(),
        "elya: main's value is an array, which is not data"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn reading_an_array_by_index_is_linear_natively() {
    let dir = temp_dir("linear");
    let time = |n: u32| {
        let exe = build(&reads(n), &dir, &format!("a{n}"));
        // The faster of three runs: a first run's page faults on a fresh
        // large heap can cost more than the reads (Codex measured the ratio
        // past ten on a loaded machine, PR #28).
        (0..3)
            .map(|_| {
                let t = std::time::Instant::now();
                let out = Command::new(&exe).output().expect("run");
                let took = t.elapsed().as_secs_f64();
                assert_eq!(
                    String::from_utf8_lossy(&out.stdout).trim_end(),
                    reads_value(n.into())
                );
                took
            })
            .fold(f64::INFINITY, f64::min)
    };
    // A list's `nth` for every index is quadratic: 40,000 elements took
    // 3.3 s (the plan's measurement), the array 6 ms. A read is a load.
    let large = time(4_000_000);
    let small = time(1_000_000).max(1e-3);
    assert!(large < 2.0, "4,000,000 elements took {large:.3} s");
    assert!(large / small < 10.0, "{large:.3} s against {small:.3} s");
    std::fs::remove_dir_all(&dir).ok();
}
