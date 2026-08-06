//! TCE is a measured guarantee: deep tail recursion runs in bounded Kont depth.
//! `K_MAX` was pinned from the first measurement (both loops peak at 3). A
//! per-iteration frame leak would drive the peak toward the iteration count;
//! a constant off-by-one would exceed the pinned ceiling. Do NOT raise `K_MAX`
//! to hide a regression — fix the machine.

use elya::parse::parse_module;
use elya::Session;

fn peak(src: &str) -> usize {
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    elya::eval::run_module(&m).unwrap().peak_kont_depth()
}

const K_MAX: usize = 3;

#[test]
fn self_tail_recursion_is_bounded() {
    let src = "fn down(n) { if n == 0 { 0 } else { down(n - 1) } }\n\
               pub fn main() { let _ = down(1000000)\n io.println(\"done\") }\n";
    let p = peak(src);
    assert!(
        p <= K_MAX,
        "self-tail-recursion peak={p} exceeds K_MAX={K_MAX}"
    );
}

#[test]
fn mutual_tail_recursion_is_bounded() {
    let src = "fn ev(n) { if n == 0 { True } else { od(n - 1) } }\n\
               fn od(n) { if n == 0 { False } else { ev(n - 1) } }\n\
               pub fn main() { let _ = ev(1000000)\n io.println(\"done\") }\n";
    let p = peak(src);
    assert!(
        p <= K_MAX,
        "mutual-tail-recursion peak={p} exceeds K_MAX={K_MAX}"
    );
}

#[test]
fn non_tail_recursion_grows_with_depth() {
    let prog = |n: i64| {
        format!(
            "fn sum(n) {{ if n == 0 {{ 0 }} else {{ n + sum(n - 1) }} }}\n\
             pub fn main() {{ let _ = sum({n})\n io.println(\"done\") }}\n"
        )
    };
    let shallow = peak(&prog(5));
    let deep = peak(&prog(50));
    assert!(
        deep > shallow,
        "non-tail must grow: shallow={shallow}, deep={deep}"
    );
    assert!(
        deep >= 45,
        "expected depth ~proportional to n=50, got {deep}"
    );
}
