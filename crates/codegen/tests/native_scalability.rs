//! The native passes scale with the program (2026-10-31, spec
//! `docs/superpowers/specs/2026-10-31-elya-scalability-design.md` D12, D13): a
//! name bound above a long block is found without searching the block, a lambda
//! keeps only the bindings it captures, rooting reads the pointers in scope
//! alone, and of a block's own bindings only those still to be read.
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

/// A block of strings, each made from the one before (D13): every `<>`
/// allocates, and rooting pushed every string so far.
fn strings_down_a_block(n: usize) -> String {
    let mut src = String::from("pub fn main() -> Int {\n  let s0 = \"a\"\n");
    for i in 1..n {
        src.push_str(&format!("  let s{i} = s{} <> \"b\"\n", i - 1));
    }
    src.push_str(&format!("  string.length(s{})\n}}\n", n - 1));
    src
}

#[test]
fn a_block_of_strings_compiles_linearly() {
    assert_linear("strings down a block", strings_down_a_block, 300);
}

/// The same under one name: each binding shadows, and parks, the one before.
fn strings_under_one_name(n: usize) -> String {
    let mut src = String::from("pub fn main() -> Int {\n  let s = \"a\"\n");
    for _ in 1..n {
        src.push_str("  let s = s <> \"b\"\n");
    }
    src.push_str("  string.length(s)\n}\n");
    src
}

#[test]
fn a_block_of_strings_under_one_name_compiles_linearly() {
    assert_linear("strings under one name", strings_under_one_name, 300);
}

/// Strings no one reads, each shadowing the last: each binding is retired at
/// its own statement and then parked, retired, by the next.
fn unread_strings_under_one_name(n: usize) -> String {
    let mut src = String::from("pub fn main() -> Int {\n");
    for i in 0..n {
        src.push_str(&format!("  let s = int.to_string({i})\n"));
    }
    src.push_str("  string.length(s)\n}\n");
    src
}

#[test]
fn unread_strings_under_one_name_compile_linearly() {
    assert_linear(
        "unread strings under one name",
        unread_strings_under_one_name,
        300,
    );
}

/// The strings block in an effectful function, a site at its end: the CPS
/// emitter lowers the block and roots what its bindings hold.
fn strings_in_an_effectful_function(n: usize) -> String {
    let mut src = String::from(
        "effect E { fn op(x: Int) -> Int }\nfn f() / {E} -> Int {\n  let s0 = \"a\"\n",
    );
    for i in 1..n {
        src.push_str(&format!("  let s{i} = s{} <> \"b\"\n", i - 1));
    }
    src.push_str(&format!(
        "  op(string.length(s{}))\n}}\npub fn main() -> Int {{ handle {{ f() }} with {{ E.op(x) -> resume(x + 1) }} }}\n",
        n - 1
    ));
    src
}

#[test]
fn a_block_of_strings_in_an_effectful_function_compiles_linearly() {
    assert_linear(
        "strings in an effectful function",
        strings_in_an_effectful_function,
        300,
    );
}
