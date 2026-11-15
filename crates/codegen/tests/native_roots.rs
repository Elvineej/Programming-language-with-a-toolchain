//! What the emitter roots (2026-11-15): a value is put on the shadow stack
//! only where something may collect before its last use. Rooting around reads
//! that allocate nothing (`string.length`, `string.byte_at`, `array.get`, ...)
//! and around arguments lowered before others that cannot allocate was half of
//! `elya/map`'s native time (callgrind: `elya_gc_push` and `elya_gc_pop` 46%);
//! without it, a million entries took 3.1 s instead of 4.5 s.
//!
//! A call to a function that never allocates (`noalloc.rs`, a fixpoint over
//! the module) roots nothing the caller holds, nor does a comparison whose
//! right operand cannot allocate.
//!
//! A string literal is a constant object in read-only data, not a heap copy
//! made each time it is evaluated: the parser in Elya natively took 66 ms on
//! its own source, then 30 ms.
//!
//! Negative controls (reverted, compared byte for byte): K1, `none_allocate`
//! answering `false` (every argument rooted again):
//! `reading_a_string_by_byte_roots_nothing` fails. K2, `never_allocating`
//! answering no function: `a_call_to_a_function_that_never_allocates_roots_nothing`
//! fails. K3, a string literal lowered to a call of `elya_str_lit` again:
//! `a_string_literal_is_a_constant` fails. The collector's safety
//! under the change is the stress suites' (`native_gc_stress.rs`,
//! `native_array.rs`, `native_std_map.rs`, a collection at every
//! allocation).

/// The optimised IR of `src`.
fn ir(src: &str) -> String {
    let (m, pd, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    assert!(pd.is_empty(), "{pd:?}");
    let (d, table) = elya::types::infer_typed_table(&elya::Session::new(), &m);
    assert!(d.is_empty(), "{d:?}");
    let core = elya::core::lower_module(&m, &table).expect("lowers");
    elya_codegen::emit_ir_for(
        &core,
        elya_codegen::Optimise::Speed,
        &elya_codegen::Machine::host(),
    )
    .expect("IR")
}

/// The body of the function named `name` in `ir`.
fn body(ir: &str, name: &str) -> String {
    let start = ir
        .find(&format!("@elya.{name}("))
        .unwrap_or_else(|| panic!("no {name} in {ir}"));
    let rest = &ir[start..];
    rest[..rest.find("\n}\n").expect("the function ends")].to_string()
}

#[test]
fn reading_a_string_by_byte_roots_nothing() {
    let src = "fn hash(key: String, i: Int, h: Int) -> Int {\n  if i == string.length(key) { h } else {\n    let byte = string.byte_at(key, i)\n    hash(key, i + 1, int.wrapping_mul(int.xor(h, byte), 1099511628211))\n  }\n}\n\
               pub fn main() -> Int { hash(\"hello\", 0, 7) }\n";
    let f = body(&ir(src), "hash");
    assert!(!f.contains("elya_gc_push"), "{f}");
    // And a read of an array, likewise.
    let src = "fn sum(borrow xs: Array(Int), i: Int, acc: Int) -> Int {\n  if i == array.length(xs) { acc } else { sum(xs, i + 1, acc + array.get(xs, i)) }\n}\n\
               pub fn main() -> Int { sum(array.push(array.new(), 3), 0, 0) }\n";
    let f = body(&ir(src), "sum");
    assert!(!f.contains("elya_gc_push"), "{f}");
}

#[test]
fn an_argument_is_still_rooted_while_a_later_one_allocates() {
    // `s` must survive the allocation of `t` (a string built by `<>`).
    let src = "fn both(s: String, t: String) -> Int { string.length(s) + string.length(t) }\n\
               fn go(s: String, n: Int) -> Int { both(s, s <> int.to_string(n)) }\n\
               pub fn main() -> Int { go(\"ab\", 12) }\n";
    let f = body(&ir(src), "go");
    assert!(f.contains("elya_gc_push"), "{f}");
}

#[test]
fn a_call_to_a_function_that_never_allocates_roots_nothing() {
    // `at` and `is_digit` allocate nothing, and neither do the functions
    // they call; `count` holds a string across calls to them.
    let src = "fn at(s: String, i: Int) -> Int { if i < string.length(s) { string.byte_at(s, i) } else { 0 - 1 } }\n\
               fn is_digit(c: Int) -> Bool { c >= 48 && c <= 57 }\n\
               fn count(s: String, i: Int, n: Int) -> Int {\n  if i == string.length(s) { n } else {\n    let d = if is_digit(at(s, i)) { 1 } else { 0 }\n    count(s, i + 1, n + d)\n  }\n}\n\
               pub fn main() -> Int { count(\"a1b22\", 0, 0) }\n";
    let f = body(&ir(src), "count");
    assert!(!f.contains("elya_gc_push"), "{f}");
    // A helper that allocates (it builds a string) is rooted around as before.
    let src = "fn label(i: Int) -> String { \"#\" <> int.to_string(i) }\n\
               fn go(s: String, i: Int) -> Int { string.length(label(i)) + string.length(s) }\n\
               pub fn main() -> Int { go(\"ab\", 3) }\n";
    let f = body(&ir(src), "go");
    assert!(f.contains("elya_gc_push"), "{f}");
}

#[test]
fn a_string_literal_is_a_constant() {
    let src = "fn name(k: Int) -> String { if k == 0 { \"zero\" } else { \"many\" } }\n\
               pub fn main() -> Int { string.length(name(1)) }\n";
    let ir = ir(src);
    let f = body(&ir, "name");
    assert!(!f.contains("elya_str_lit"), "{f}");
    assert!(!f.contains("elya_alloc"), "{f}");
    assert!(
        ir.contains("c\"many\\00"),
        "the literal's bytes are a constant: {ir}"
    );
}

/// `a && b` is an `if` in Core (spec 2026-11-15-elya-short-circuit D1): a
/// branch whose parts allocate nothing allocates nothing, so a string held
/// across an argument that is a `&&` is not rooted, as it was not when `&&`
/// was a strict `Prim`. The reviewer's finding; control K8 in that plan.
#[test]
fn a_short_circuit_argument_roots_nothing() {
    let src = "fn g(s: String, b: Bool) -> Int { if b { string.length(s) } else { 0 } }\n\
               fn h(s: String, i: Int) -> Int { 1 + g(s, i < string.length(s) && i > 0) }\n\
               pub fn main() -> Int { h(\"abc\", 1) }\n";
    let ir = ir(src);
    let f = body(&ir, "h");
    assert!(!f.contains("elya_gc_push"), "{f}");
}
