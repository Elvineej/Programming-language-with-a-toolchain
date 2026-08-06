//! Depth-instrumentation control. Bounded-depth (TCE) assertions arrive in
//! Slice 2 with the CEK machine; here we prove the measurement grows with
//! recursion, so the harness is real when the guarantee lands.

use elya::ast::Module;
use elya::eval::run_module;
use elya::parse::parse_module;
use elya::Session;

fn max_depth_for(src: &str) -> usize {
    let (m, d): (Module, _) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse diags: {d:?}");
    run_module(&m).unwrap().max_depth()
}

#[test]
fn recursion_depth_grows_with_input() {
    // A self-recursive countdown; deeper input => deeper host recursion (Slice 1).
    let prog = |n: i64| {
        format!(
            "fn go(n) {{ if n == 0 {{ 0 }} else {{ go(n - 1) }} }}\n\
             pub fn main() {{ let _ = go({n})\n io.println(\"done\") }}\n"
        )
    };
    let shallow = max_depth_for(&prog(5));
    let deep = max_depth_for(&prog(50));
    assert!(
        deep > shallow,
        "expected deeper recursion to grow max_depth: shallow={shallow}, deep={deep}"
    );
}
