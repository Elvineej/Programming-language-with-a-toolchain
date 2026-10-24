//! Upcasts through data, natively (2026-10-23, spec
//! `docs/superpowers/specs/2026-10-23-elya-row-tails-in-data-design.md`). A value
//! holding a pure closure used where data of an effectful one is expected
//! (sub-effecting under a constructor). Every program is compiled, linked and RUN,
//! and its outcome compared with the evaluator's.
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K1, D1 off (`anf::bind_tails` not called): `wrapped_results_inside_data_agree`
//!   fails on r1, a pointer natively where the evaluator prints 15.
//! - K2, D2's data case off (`coerce` and `coercible` without `Con`):
//!   `call_results_upcast_through_data_agree` is refused by name at let_lam
//!   ("direct function used where an effectful one is expected"): D3's guard.
//! - K3, D2 and D3 off: the same test fails at let_lam with "match failed (no
//!   arm matched)" natively where the evaluator prints 16.
//! - K4, the parameter pass of spec 2026-10-23-elya-wrapper-parameter-rows D1
//!   off: `wrapped_results_sharing_a_parameter_row_agree` fails on f2, a pointer.

use elya::core::lower_module;
use elya::parse::parse_module;
use elya::Session;
use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-upcast-data-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn lower(src: &str) -> elya::core::CoreModule {
    if let Err(e) = elya::check_source("t.elya", src) {
        panic!("the front end rejected the program: {e}\n{src}");
    }
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&session, &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    lower_module(&m, &table).unwrap_or_else(|e| panic!("core lowering failed: {e:?}"))
}

/// What a run printed, and how it ended: main's value, or the failure's message.
#[derive(Debug, PartialEq)]
struct Outcome {
    text: String,
    end: Result<String, String>,
}

fn evaluate(src: &str) -> Outcome {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (interp, r) = elya::eval::run_module_keeping(&m, elya::eval::World::Live);
    let end = match r {
        Ok(elya::eval::Value::Int(n)) => Ok(n.to_string()),
        Ok(other) => panic!("main must be an Int, got {other:?}"),
        Err(e) => Err(e.diag.message),
    };
    Outcome {
        text: interp.output().to_string(),
        end,
    }
}

fn run_native(src: &str, tag: &str) -> Outcome {
    let core = lower(src);
    let dir = temp_dir(tag);
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    if let Err(e) = elya_codegen::compile_module(&core, &obj) {
        panic!("{tag}: {e}");
    }
    elya_codegen::link(&obj, &exe).expect("link");
    let out = Command::new(&exe).output().expect("run the binary");
    std::fs::remove_dir_all(&dir).ok();
    let stdout = String::from_utf8(out.stdout).expect("UTF-8 stdout");
    let stderr = String::from_utf8(out.stderr).expect("UTF-8 stderr");
    if out.status.success() {
        assert!(stderr.is_empty(), "{tag}: stderr: {stderr}");
        let body = stdout
            .strip_suffix('\n')
            .unwrap_or_else(|| panic!("{tag}: no value line: {stdout:?}"));
        let (text, value) = match body.rsplit_once('\n') {
            Some((t, v)) => (format!("{t}\n"), v.to_string()),
            None => (String::new(), body.to_string()),
        };
        Outcome {
            text,
            end: Ok(value),
        }
    } else {
        assert_eq!(out.status.code(), Some(1), "{tag}: {stderr}");
        let msg = stderr
            .strip_prefix("elya: ")
            .and_then(|s| s.strip_suffix('\n'))
            .unwrap_or_else(|| panic!("{tag}: not a named failure: {stderr:?}"));
        Outcome {
            text: stdout,
            end: Err(msg.to_string()),
        }
    }
}

/// The program natively and in the evaluator: the same outcome, returned.
fn agree(src: &str, tag: &str) -> Outcome {
    let want = evaluate(src);
    let got = run_native(src, tag);
    assert_eq!(got, want, "{tag}: native vs evaluator\n{src}");
    got
}

const P: &str = "effect E { fn e() -> Int }\ntype Option(a) { None, Some(a) }\n";

/// D1: a top-level function whose own result has a row variable inside data,
/// used as a value at an effectful type (the 2026-10-23 review's F2).
#[test]
fn wrapped_results_inside_data_agree() {
    let r1 = format!(
        "{P}fn mko() -> fn() -> Option(fn() -> Int) {{ fn() {{ Some(fn() {{ 15 }}) }} }}\n\
         fn use(g: fn() -> fn() / {{E}} -> Option(fn() / {{E}} -> Int)) / {{E}} -> Int {{\n\
         \x20 let h = g()\n\
         \x20 match h() {{ Some(k) -> k()  None -> 0 }}\n\
         }}\n\
         pub fn main() -> Int {{ handle {{ use(mko) }} with {{ E.e() -> resume(1) }} }}\n"
    );
    assert_eq!(agree(&r1, "r1").end, Ok("15".into()));
    let top_val = format!(
        "{P}fn mk() -> Option(fn() -> Int) {{ Some(fn() {{ 15 }}) }}\n\
         fn mk2() -> fn() -> Option(fn() -> Int) {{ mk }}\n\
         fn use(g: fn() -> fn() / {{E}} -> Option(fn() / {{E}} -> Int)) / {{E}} -> Int {{\n\
         \x20 let h = g()\n\
         \x20 match h() {{ Some(k) -> k() + e()  None -> 0 }}\n\
         }}\n\
         pub fn main() -> Int {{ handle {{ use(mk2) }} with {{ E.e() -> resume(1) }} }}\n"
    );
    assert_eq!(agree(&top_val, "top_val").end, Ok("16".into()));
}

/// D2: a call's result upcast through a data constructor, flat, recursive and
/// nested.
#[test]
fn call_results_upcast_through_data_agree() {
    let let_lam = format!(
        "{P}fn run(o: Option(fn() / {{E}} -> Int)) / {{E}} -> Int {{ match o {{ Some(k) -> k() + e()  None -> 0 }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let mk = fn() {{ Some(fn() {{ 15 }}) }}\n\
         \x20 handle {{ run(mk()) }} with {{ E.e() -> resume(1) }}\n\
         }}\n"
    );
    assert_eq!(agree(&let_lam, "let_lam").end, Ok("16".into()));
    let list = format!(
        "{P}type L(a) {{ Nil, Cons(a, L(a)) }}\n\
         fn sum(xs: L(fn() / {{E}} -> Int)) / {{E}} -> Int {{ match xs {{ Nil -> e()  Cons(k, r) -> k() + 10 * sum(r) }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let mk = fn() {{ Cons(fn() {{ 1 }}, Cons(fn() {{ 20 }}, Nil)) }}\n\
         \x20 handle {{ sum(mk()) }} with {{ E.e() -> resume(3) }}\n\
         }}\n"
    );
    assert_eq!(agree(&list, "list").end, Ok("501".into()));
    let nest = format!(
        "{P}type Two(a) {{ Two(Int, a) }}\n\
         fn run(t: Two(Option(fn() / {{E}} -> Int))) / {{E}} -> Int {{ match t {{ Two(n, Some(k)) -> n + k() + e()  Two(n, None) -> n }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let mk = fn() {{ Two(4000, Some(fn() {{ 15 }})) }}\n\
         \x20 handle {{ run(mk()) }} with {{ E.e() -> resume(100) }}\n\
         }}\n"
    );
    assert_eq!(agree(&nest, "nest").end, Ok("4115".into()));
    // Data holding data holding a function, under a declared (non-generic) type
    // whose field is generic data: the inner `Option` is coerced by its own
    // function, and printing before the upcast keeps its place.
    let deep = format!(
        "{P}type Box(a) {{ Box(Option(a), Option(a)) }}\n\
         fn run(b: Box(fn() / {{E}} -> Int)) / {{E}} -> Int {{\n\
         \x20 match b {{ Box(Some(k), None) -> k() * e()  Box(_, _) -> 0 }}\n\
         }}\n\
         pub fn main() -> Int {{\n\
         \x20 let mk = fn() {{ io.println(\"made\")  Box(Some(fn() {{ 7 }}), None) }}\n\
         \x20 handle {{ run(mk()) }} with {{ E.e() -> resume(6) }}\n\
         }}\n"
    );
    let out = agree(&deep, "deep");
    assert_eq!((out.text.as_str(), out.end), ("made\n", Ok("42".into())));
}

/// D2's cost: a long list upcast once is copied once, natively on its own stack.
#[test]
fn a_long_list_is_upcast_once() {
    let src = format!(
        "{P}type L(a) {{ Nil, Cons(a, L(a)) }}\n\
         fn sum(xs: L(fn() / {{E}} -> Int), acc: Int) / {{E}} -> Int {{ match xs {{ Nil -> acc + e()  Cons(k, r) -> sum(r, acc + k()) }} }}\n\
         fn rep(n: Int, x: a, acc: L(a)) -> L(a) {{ if n == 0 {{ acc }} else {{ rep(n - 1, x, Cons(x, acc)) }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let mk = fn() {{ rep(300000, fn() {{ 2 }}, Nil) }}\n\
         \x20 handle {{ sum(mk(), 0) }} with {{ E.e() -> resume(1) }}\n\
         }}\n"
    );
    assert_eq!(agree(&src, "long").end, Ok("600001".into()));
}

/// The value path: the call's result bound first (annotated, and not), then
/// passed and captured.
#[test]
fn bound_values_upcast_through_data_agree() {
    let ann = format!(
        "{P}fn run(o: Option(fn() / {{E}} -> Int)) / {{E}} -> Int {{ match o {{ Some(k) -> k() + e()  None -> 0 }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let mk = fn() {{ Some(fn() {{ 15 }}) }}\n\
         \x20 let v: Option(fn() / {{E}} -> Int) = mk()\n\
         \x20 handle {{ run(v) }} with {{ E.e() -> resume(1) }}\n\
         }}\n"
    );
    assert_eq!(agree(&ann, "ann").end, Ok("16".into()));
    let cap = format!(
        "{P}fn run(o: Option(fn() / {{E}} -> Int)) / {{E}} -> Int {{ match o {{ Some(k) -> k() + e()  None -> 0 }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let mk = fn() {{ Some(fn() {{ 15 }}) }}\n\
         \x20 let v = mk()\n\
         \x20 let later = fn() {{ run(v) }}\n\
         \x20 handle {{ later() + run(v) }} with {{ E.e() -> resume(1) }}\n\
         }}\n"
    );
    assert_eq!(agree(&cap, "cap").end, Ok("32".into()));
}

/// The review's F1: an upcast through a NON-REGULAR type (a field at a bigger
/// instance of the type itself) would need infinitely many rebuild functions;
/// it is refused by name, where the first version overflowed the compiler's
/// stack. The same type at a regular depth still runs.
#[test]
fn a_non_regular_type_is_refused_by_name() {
    let src = format!(
        "{P}type N(a) {{ Z(a), S(N(Option(a))) }}\n\
         fn run(o: N(fn() / {{E}} -> Int)) / {{E}} -> Int {{ match o {{ Z(k) -> k() + e()  S(_) -> 7 }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let mk = fn() {{ Z(fn() {{ 15 }}) }}\n\
         \x20 handle {{ run(mk()) }} with {{ E.e() -> resume(1) }}\n\
         }}\n"
    );
    let dir = temp_dir("nonregular");
    let err = elya_codegen::compile_module(&lower(&src), &dir.join("n.o"))
        .expect_err("a non-regular upcast is refused");
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        err.to_string()
            .contains("upcast through a non-regular data type"),
        "{err}"
    );
    // Nested Options, eight deep: regular, rebuilt by eight functions.
    let deep = format!(
        "{P}fn run(o: Option(Option(Option(Option(Option(Option(Option(Option(fn() / {{E}} -> Int))))))))) / {{E}} -> Int {{\n\
         \x20 match o {{ Some(Some(Some(Some(Some(Some(Some(Some(k)))))))) -> k() + e()  _ -> 0 }}\n\
         }}\n\
         pub fn main() -> Int {{\n\
         \x20 let mk = fn() {{ Some(Some(Some(Some(Some(Some(Some(Some(fn() {{ 15 }}))))))))  }}\n\
         \x20 handle {{ run(mk()) }} with {{ E.e() -> resume(1) }}\n\
         }}\n"
    );
    assert_eq!(agree(&deep, "deep8").end, Ok("16".into()));
}

/// Spec 2026-10-23-elya-wrapper-parameter-rows (the upcasts-through-data
/// review's F2): the wrapped function's own result shares a row variable with a
/// parameter; it is bound from the use's parameter.
#[test]
fn wrapped_results_sharing_a_parameter_row_agree() {
    let programs: [(&str, &str); 10] = [
        ("res_param", "effect E { fn e() -> Int }\nfn mk(k: fn() -> Int) -> fn(fn() -> Int) -> fn() -> Int { fn(j) { k } }\nfn use(g: fn(fn() / {E} -> Int) -> fn(fn() -> Int) / {E} -> fn() / {E} -> Int) / {E} -> Int { g(fn() { e() + 15 })(fn() { 3 })() }\npub fn main() -> Int { handle { use(mk) } with { E.e() -> resume(1) } }\n"),
        ("res_param2", "effect E { fn e() -> Int }\nfn mk(k: fn() -> Int) -> fn() -> fn(fn() -> Int) -> fn() -> Int { fn() { fn(j) { k } } }\nfn use(g: fn(fn() / {E} -> Int) -> fn() / {E} -> fn(fn() -> Int) / {E} -> fn() / {E} -> Int) / {E} -> Int { g(fn() { e() + 15 })()(fn() { 3 })() }\npub fn main() -> Int { handle { use(mk) } with { E.e() -> resume(1) } }\n"),
        ("f2", "effect E { fn e() -> Int }\nfn mko(k: fn() -> Int) -> fn() -> fn() -> Int { fn() { k } }\nfn use(g: fn(fn() / {E} -> Int) -> fn() / {E} -> fn() / {E} -> Int) / {E} -> Int { g(fn() { 15 })()() + e() }\npub fn main() -> Int { handle { use(mko) } with { E.e() -> resume(1) } }\n"),
        ("effarg", "effect E { fn e() -> Int }\nfn mko(k: fn() -> Int) -> fn() -> fn() -> Int { fn() { k } }\nfn use(g: fn(fn() / {E} -> Int) -> fn() / {E} -> fn() / {E} -> Int) / {E} -> Int { g(fn() { e() + 15 })()() }\npub fn main() -> Int { handle { use(mko) } with { E.e() -> resume(1) } }\n"),
        ("s8", "effect E { fn e() -> Int }\nfn mko(k: fn() -> Int) -> fn() -> fn() -> Int { fn() { k } }\nfn use(g: fn(fn() / {E} -> Int) -> fn() -> fn() / {E} -> Int) / {E} -> Int { g(fn() { 15 })()() + e() }\npub fn main() -> Int { handle { use(mko) } with { E.e() -> resume(1) } }\n"),
        ("r8", "effect E { fn e() -> Int }\ntype Option(a) { None, Some(a) }\nfn mko(k: fn() -> Int) -> fn() -> Option(fn() -> Int) { fn() { Some(k) } }\nfn use(g: fn(fn() / {E} -> Int) -> fn() / {E} -> Option(fn() / {E} -> Int)) / {E} -> Int { match g(fn() { 15 })() { Some(k) -> k() + e()  None -> 0 } }\npub fn main() -> Int { handle { use(mko) } with { E.e() -> resume(1) } }\n"),
        ("s3", "effect E { fn e() -> Int }\ntype Option(a) { None, Some(a) }\nfn mko(k: fn() -> Int) -> fn() -> Option(fn() -> Int) { fn() { Some(k) } }\nfn use(g: fn(fn() / {E} -> Int) -> fn() -> Option(fn() / {E} -> Int)) / {E} -> Int { match g(fn() { 15 })() { Some(k) -> k() + e()  None -> 0 } }\npub fn main() -> Int { handle { use(mko) } with { E.e() -> resume(1) } }\n"),
        ("s4", "effect E { fn e() -> Int }\ntype Option(a) { None, Some(a) }\nfn mko(k: fn() -> Int) -> fn() -> Option(fn() -> Int) { fn() { Some(k) } }\npub fn main() -> Int { let g: fn(fn() / {E} -> Int) -> fn() / {E} -> Option(fn() / {E} -> Int) = mko\n handle { match g(fn() { 15 })() { Some(k) -> k() + e()  None -> 0 } } with { E.e() -> resume(1) } }\n"),
        ("one", "effect E { fn e() -> Int }\nfn mko(k: fn() -> Int) -> fn() -> Int { k }\nfn use(g: fn(fn() / {E} -> Int) -> fn() / {E} -> Int) / {E} -> Int { g(fn() { 15 })() + e() }\npub fn main() -> Int { handle { use(mko) } with { E.e() -> resume(1) } }\n"),
        ("unused", "effect E { fn e() -> Int }\nfn mko(k: fn() -> Int) -> fn() -> fn() -> Int { fn() { fn() { 15 } } }\nfn use(g: fn(fn() / {E} -> Int) -> fn() / {E} -> fn() / {E} -> Int) / {E} -> Int { g(fn() { 15 })()() + e() }\npub fn main() -> Int { handle { use(mko) } with { E.e() -> resume(1) } }\n"),
    ];
    for (tag, src) in programs {
        assert_eq!(agree(src, tag).end, Ok("16".into()), "{tag}");
    }
}
