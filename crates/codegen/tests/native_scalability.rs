//! The native passes scale with the program (2026-10-31, spec
//! `docs/superpowers/specs/2026-10-31-elya-scalability-design.md` D12): a name
//! bound above a long block is found without searching the block, a lambda
//! keeps only the bindings it captures, and rooting reads the pointers in
//! scope alone.
//!
//! Timing-free: `elya_codegen::scope::probes()`, the bindings name lookups and
//! rooting examine on this thread, measured around `emit_ir` at `n` and `2n`.
//! Each doubles when its program doubles (and the IR with it). The probes
//! count what `Scope` and `Env` examine: they pin those two and the passes'
//! use of them, not a pass that searches a list of its own (the review of
//! part 3a: one made to search again went unseen; its emit time grew).

fn no_files(_: &str) -> Option<(String, String)> {
    None
}

/// IR lines and the probes emitting them took.
fn emit(src: &str) -> (usize, u64) {
    let (module, diags, _) = elya::front_end_with("main.elya", src, &mut no_files);
    assert!(diags.is_empty(), "{diags:?}");
    let session = elya::Session::new();
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    assert!(diags.is_empty(), "{diags:?}");
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let before = elya_codegen::scope::probes();
    let ir = elya_codegen::emit_ir(&core).expect("emits");
    (ir.lines().count(), elya_codegen::scope::probes() - before)
}

fn assert_linear(name: &str, shape: impl Fn(usize) -> String, n: usize) {
    let (a, b) = (emit(&shape(n)), emit(&shape(2 * n)));
    for (what, x, y) in [("IR lines", a.0 as u64, b.0 as u64), ("probes", a.1, b.1)] {
        assert!(
            10 * y <= 21 * x,
            "{name}: {what}({n}) = {x}, {what}({}) = {y}, ratio {:.2} (linear is 2.00)",
            2 * n,
            y as f64 / x as f64
        );
    }
}

/// A parameter used down a block of `n` statements.
fn a_parameter_down_a_block(n: usize) -> String {
    let mut src = String::from("fn f(x: Int) -> Int {\n  let a0 = x\n");
    for i in 1..n {
        src.push_str(&format!("  let a{i} = (a{} + x) % 1000003\n", i - 1));
    }
    src.push_str(&format!(
        "  a{}\n}}\npub fn main() -> Int {{ f(7) }}\n",
        n - 1
    ));
    src
}

#[test]
fn a_parameter_used_down_a_long_block_compiles_linearly() {
    assert_linear("a parameter down a block", a_parameter_down_a_block, 500);
}

/// A top-level function called down a block: each call is a direct call, and
/// each roots the environment.
fn calls_down_a_block(n: usize) -> String {
    let mut src =
        String::from("fn g(x: Int) -> Int { x * 3 + 1 }\npub fn main() -> Int {\n  let a0 = 1\n");
    for i in 1..n {
        src.push_str(&format!("  let a{i} = g(a{}) % 1000003\n", i - 1));
    }
    src.push_str(&format!("  a{}\n}}\n", n - 1));
    src
}

#[test]
fn calls_down_a_long_block_compile_linearly() {
    assert_linear("calls down a block", calls_down_a_block, 500);
}

/// A closure made and called at every statement of a block.
fn closures_down_a_block(n: usize) -> String {
    let mut src = String::from(
        "fn apply(k: fn() -> Int) -> Int { k() }\npub fn main() -> Int {\n  let f = fn() { 1 }\n  let a0 = 0\n",
    );
    for i in 1..n {
        src.push_str(&format!(
            "  let a{i} = (a{} + apply(fn() {{ f() }})) % 1000003\n",
            i - 1
        ));
    }
    src.push_str(&format!("  a{}\n}}\n", n - 1));
    src
}

#[test]
fn closures_down_a_long_block_compile_linearly() {
    assert_linear("closures down a block", closures_down_a_block, 300);
}
