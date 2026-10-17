//! `with any` natively (2026-10-17, spec
//! `docs/superpowers/specs/2026-10-17-elya-native-any-handlers-design.md`). Every
//! program here is compiled, linked and RUN, and its output compared with the
//! evaluator's: the same text and value, or the same failure in the same words
//! (`elya: ` and the evaluator's message, exit status 1).
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K1, the clause copies keep the rigid answer type (`any.rs` skips
//!   `replace_con`): the round-trip and 100 000-cell tests fail by name
//!   (`internal: op: a value of a type with no instance`), the forwarding and
//!   nested tests panic in inkwell (a value of the wrong width).
//! - K2, one clause per handled effect instead of one per operation: four tests
//!   fail, the later operations reaching the next handler out unrecorded (and
//!   `names...` refused: `calling convention disagrees with the callee`).
//! - K3, decode's trailing-input check off: `decode_failures...` fails on
//!   `Cons(1, Nil) x`, which then decodes.
//! - K4, decode's build phase roots nothing: the 100 000-cell binary never
//!   finishes (a collection during the build frees cells still being linked);
//!   killed after minutes, the other nine pass.
//! - K5, `instantiate_types` demands no decode closure (spec D5): the non-regular
//!   program compiles, and `Opt(Int)` (only ever decoded) fails by name,
//!   `internal: op.decode: a type with no instance`.

use elya::core::lower_module;
use elya::parse::parse_module;
use elya::Session;
use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-any-{}-{tag}", std::process::id()));
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

fn refusal(src: &str) -> String {
    let core = lower(src);
    let dir = temp_dir("refusal");
    let r = elya_codegen::compile_module(&core, &dir.join("r.o"));
    std::fs::remove_dir_all(&dir).ok();
    match r {
        Ok(()) => panic!("compiled, expected a refusal:\n{src}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn the_record_any_example_runs_natively() {
    let src = include_str!("../../../examples/09_record_any.elya");
    let got = agree(src, "example09");
    assert!(
        got.text
            .contains("the game with an eight-sided die: diverged, asked World.roll(8)"),
        "{}",
        got.text
    );
}

const ASK: &str = "type Shape { Dot, Box(Int, Int) }\n\
effect Ask {\n\
  fn num(n: Int) -> Int\n\
  fn yes() -> Bool\n\
  fn word(w: String) -> String\n\
  fn tick() -> Unit\n\
  fn shape(k: Int) -> Shape\n\
}\n\
fn live(body) {\n\
  handle { body() } with {\n\
    Ask.num(n) -> resume(n + 1)\n\
    Ask.yes() -> resume(True)\n\
    Ask.word(w) -> resume(\"ok\" <> w)\n\
    Ask.tick() -> resume(Unit)\n\
    Ask.shape(k) -> resume(if k > 0 { Box(k, 0 - 4) } else { Dot })\n\
  }\n\
}\n\
fn prog() / {Ask} -> Int {\n\
  let a = num(41)\n\
  let b = if yes() { 1 } else { 0 }\n\
  let w = word(\"he said \\\"hi\\\"\\n\\\\ \\t\\r\")\n\
  let _ = tick()\n\
  let s = match shape(3) { Dot -> 0  Box(x, y) -> x * y }\n\
  let d = match shape(0) { Dot -> 1000  Box(x, y) -> 0 }\n\
  a + b + s + d + (if w == \"ok\" { 100 } else { 7 })\n\
}\n";

#[test]
fn forwarding_resumes_at_every_answer_type() {
    let src = format!(
        "{ASK}fn rec(body) {{\n\
           handle {{ body() }} with any {{\n\
             op(o) -> {{\n\
               let v = op.perform(o)\n\
               io.println(op.name(o) <> \" \" <> op.show(o) <> \" = \" <> op.encode(o, v))\n\
               resume(v)\n\
             }}\n\
           }}\n\
         }}\n\
         pub fn main() -> Int {{ live(fn() {{ rec(fn() {{ prog() }}) }}) }}\n"
    );
    let got = agree(&src, "forward");
    assert!(
        got.text.contains("Ask.shape Ask.shape(3) = Box(3, -4)"),
        "{}",
        got.text
    );
}

#[test]
fn names_and_questions_are_the_evaluators() {
    let src = "type Shape { Dot, Box(Int, Int) }\n\
        type Bag(a) { Empty, Has(a, Bag(a)) }\n\
        effect Q {\n\
          fn mix(a: Int, b: String, c: Shape) -> Int\n\
          fn bag(b: Bag(Bool), u: Unit) -> Int\n\
          fn none() -> Int\n\
        }\n\
        fn replay(body) { handle { body() } with any {\n\
          op(o) -> { io.println(op.name(o))  io.println(op.show(o))  resume(op.decode(o, \"7\")) }\n\
        } }\n\
        pub fn main() -> Int {\n\
          replay(fn() {\n\
            mix(0 - 12, \"a\\\"b\\\\c\\nd\", Box(1, 0 - 1)) + bag(Has(True, Has(False, Empty)), Unit) + none()\n\
          })\n\
        }\n";
    let got = agree(src, "names");
    assert!(
        got.text
            .contains("Q.mix(-12, \"a\\\"b\\\\c\\nd\", Box(1, -1))"),
        "{}",
        got.text
    );
    assert!(got.text.contains("Q.none()"), "{}", got.text);
}

const ROUND_TYPES: &str = "type List(a) { Nil, Cons(a, List(a)) }\n\
type Opt(a) { No, Yes(a) }\n\
type Pair(a, b) { P(a, b) }\n\
type Tree { Leaf, Node(Tree, Int, Tree) }\n\
effect R {\n\
  fn words() -> List(Opt(String))\n\
  fn pair(n: Int) -> Pair(Bool, Unit)\n\
  fn tree() -> Tree\n\
  fn count() -> Opt(Int)\n\
}\n\
fn rt(body) {\n\
  handle { body() } with any {\n\
    op(o) -> {\n\
      let v = op.perform(o)\n\
      let e = op.encode(o, v)\n\
      io.println(op.show(o) <> \" -> \" <> e)\n\
      resume(op.decode(o, e))\n\
    }\n\
  }\n\
}\n\
fn len(xs) { match xs { Nil -> 0  Cons(x, r) -> 1 + len(r) } }\n\
fn sum(t) { match t { Leaf -> 0  Node(l, n, r) -> sum(l) + n + sum(r) } }\n";

#[test]
fn answers_round_trip_through_declared_types() {
    // Decoded through the recorder, then used: the decoded values are real values.
    let src = format!(
        "{ROUND_TYPES}pub fn main() -> Int {{\n\
           handle {{ rt(fn() {{\n\
             let ws = words()\n\
             let p = match pair(1) {{ P(b, u) -> if b {{ 10 }} else {{ 20 }} }}\n\
             let t = sum(tree())\n\
             len(ws) * 1000 + p + t\n\
           }}) }} with {{\n\
             R.words() -> resume(Cons(Yes(\"a, \\\"b\\\"\"), Cons(No, Cons(Yes(\"\"), Nil))))\n\
             R.pair(n) -> resume(P(n == 1, Unit))\n\
             R.tree() -> resume(Node(Node(Leaf, 0 - 3, Leaf), 5, Node(Leaf, 100, Leaf)))\n\
             R.count() -> resume(No)\n\
           }}\n\
         }}\n"
    );
    let got = agree(&src, "round");
    assert_eq!(got.end, Ok("3112".to_string()));
    // A decode at a type no constructor or pattern of the program mentions:
    // `Opt(Int)` exists only as `count`'s answer (spec D5), and texts with spaces.
    let src = format!(
        "{ROUND_TYPES}fn replay(body) {{ handle {{ body() }} with any {{\n\
           op(o) -> {{ let v = op.decode(o, \" Yes( -42 ) \")  io.println(op.encode(o, v))  resume(v) }}\n\
         }} }}\n\
         pub fn main() -> Int {{ let _ = replay(fn() {{ count() }})  1 }}\n"
    );
    let got = agree(&src, "unbuilt");
    assert_eq!(got.text, "Yes(-42)\n");
}

#[test]
fn a_hundred_thousand_cell_list_round_trips() {
    let src = "type L { Nil, Cons(Int, L) }\n\
        effect Q { fn big(n: Int) -> L }\n\
        fn build(n: Int, acc: L) -> L { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
        fn len(l: L, a: Int) -> Int { match l { Nil -> a  Cons(x, r) -> len(r, a + 1) } }\n\
        fn rt(body) {\n\
          handle { body() } with any {\n\
            op(o) -> { let v = op.perform(o)  resume(op.decode(o, op.encode(o, v))) }\n\
          }\n\
        }\n\
        pub fn main() -> Int {\n\
          handle { rt(fn() { len(big(100000), 0) }) } with { Q.big(n) -> resume(build(n, Nil)) }\n\
        }\n";
    let got = agree(src, "big");
    assert_eq!(got.end, Ok("100000".to_string()));
}

#[test]
fn decode_failures_have_the_evaluators_words() {
    let cases: &[&str] = &[
        "",
        "   ",
        "Cons(1, Nil",
        "Cons(1 Nil)",
        "Cons(1, Nil) x",
        "Cons(\\\"a\\\", Nil)",
        "Cons(1.5, Nil)",
        "Cons(99999999999999999999, Nil)",
        "Cons(-, Nil)",
        "Cons(1.2.3, Nil)",
        "\\\"abc",
        "\\\"a\\\\qb\\\"",
        "Nope",
        "Cons(1)",
        "Cons(1, Yes(2))",
        "#",
        "inf",
        "nan",
        "True",
        "Unit",
        "Nil()",
        "Cons(1, Nil)",
    ];
    for (i, text) in cases.iter().enumerate() {
        let src = format!(
            "type L {{ Nil, Cons(Int, L) }}\n\
             type O {{ Yes(Int) }}\n\
             effect E {{ fn e() -> L }}\n\
             fn replay(body) {{ handle {{ body() }} with any {{ op(o) -> resume(op.decode(o, \"{text}\")) }} }}\n\
             pub fn main() -> Int {{ match replay(fn() {{ e() }}) {{ Nil -> 0  Cons(x, r) -> x }} }}\n"
        );
        agree(&src, &format!("decode{i}"));
    }
    // At a base type, and at an effect's own parameter (the evaluator's S2).
    for (i, (ty, text)) in [
        ("Int", "-7"),
        ("Int", "True"),
        ("Bool", "False"),
        ("Bool", "0"),
        ("String", "\\\"x\\\""),
        ("Unit", "Unit"),
        ("Unit", "()"),
    ]
    .iter()
    .enumerate()
    {
        let src = format!(
            "effect E {{ fn e() -> {ty} }}\n\
             fn replay(body) {{ handle {{ body() }} with any {{ op(o) -> resume(op.decode(o, \"{text}\")) }} }}\n\
             pub fn main() -> Int {{ let _ = replay(fn() {{ e() }})  5 }}\n"
        );
        agree(&src, &format!("base{i}"));
    }
    let src = "effect St(s) { fn get() -> s }\n\
        fn replay(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"\\\"hello\\\"\")) } }\n\
        fn prog() / {St(Int)} -> Int { get() + 1 }\n\
        pub fn main() -> Int { io.println(\"before\")  replay(fn() { prog() }) }\n";
    let got = agree(src, "param");
    assert!(
        got.end
            .as_ref()
            .is_err_and(|m| m.contains("mentions the effect's parameter `s`")),
        "{got:?}"
    );
}

#[test]
fn showing_a_function_fails_by_name() {
    // A forwarding recorder's tail is checked at run time (the earlier spec's D5).
    let show = "effect K { fn k(f: fn(Int) -> Int) -> Int }\n\
        fn rec(body) { handle { body() } with any { op(o) -> { io.println(op.name(o))  io.println(op.show(o))  resume(op.perform(o)) } } }\n\
        pub fn main() -> Int { handle { rec(fn() { k(fn(x) { x + 1 }) }) } with { K.k(f) -> resume(f(1)) } }\n";
    let got = agree(show, "showfn");
    assert!(got.end.is_err(), "{got:?}");
    let encode = "type Box { B(fn(Int) -> Int) }\n\
        effect K { fn mk() -> Box }\n\
        fn rec(body) { handle { body() } with any { op(o) -> { let v = op.perform(o)  io.println(op.encode(o, v))  resume(v) } } }\n\
        pub fn main() -> Int { handle { rec(fn() { match mk() { B(f) -> f(2) } }) } with { K.mk() -> resume(B(fn(x) { x * 10 })) } }\n";
    let got = agree(encode, "encodefn");
    assert!(got.end.is_err(), "{got:?}");
}

#[test]
fn operations_compare_by_what_they_are() {
    let src = "type Shape { Dot, Box(Int, Int) }\n\
        effect W { fn roll(s: Int) -> Int  fn put(s: Shape, w: String) -> Unit  fn none() -> Bool }\n\
        fn f(body) { handle { body() } with any {\n\
          op(o) -> {\n\
            io.println(if o == o { \"same\" } else { \"different\" })\n\
            io.println(if o != o { \"different\" } else { \"same\" })\n\
            resume(op.perform(o))\n\
          }\n\
        } }\n\
        pub fn main() -> Int {\n\
          handle { f(fn() { let _ = put(Box(1, 2), \"x\")  let b = none()  roll(6) + (if b { 1 } else { 0 }) }) } with {\n\
            W.roll(s) -> resume(s * 2)\n\
            W.put(s, w) -> resume(Unit)\n\
            W.none() -> resume(True)\n\
          }\n\
        }\n";
    let got = agree(src, "equal");
    assert_eq!(got.end, Ok("13".to_string()));
    let src = "effect World { fn roll(s: Int) -> Int }\n\
        fn f(body) { handle { body() } with any { op(o) -> if o == o { resume(op.decode(o, \"2\")) } else { 0 } } }\n\
        pub fn main() -> Int { f(fn() { roll(6) + 1 }) }\n";
    agree(src, "equal0419");
}

#[test]
fn nested_and_local_any_handlers() {
    // A recorder inside a recorder: the inner forwards to the outer, which
    // forwards to the live handler.
    let src = format!(
        "{ASK}fn rec(tag, body) {{\n\
           handle {{ body() }} with any {{\n\
             op(o) -> {{ let v = op.perform(o)  io.println(tag <> \": \" <> op.show(o) <> \" = \" <> op.encode(o, v))  resume(v) }}\n\
           }}\n\
         }}\n\
         pub fn main() -> Int {{ live(fn() {{ rec(\"outer\", fn() {{ rec(\"inner\", fn() {{ prog() }}) }}) }}) }}\n"
    );
    agree(&src, "nested");
    // A handler in a local `let` lambda, used at two effects.
    let src = "effect World { fn roll(s: Int) -> Int }\n\
        effect Ask { fn ask() -> Int }\n\
        fn f() -> Int {\n\
          let r = fn(b) { handle { b() } with any { op(o) -> resume(op.perform(o)) } }\n\
          let x = handle { r(fn() { roll(6) }) } with { World.roll(s) -> resume(4) }\n\
          let y = handle { r(fn() { ask() }) } with { Ask.ask() -> resume(5) }\n\
          x + y\n\
        }\n\
        pub fn main() -> Int { io.println(int.to_string(f()))  f() }\n";
    agree(src, "local");
    // An operation clause of an `any` handler inside the clause of another: the
    // inner handles what the outer's clause performs again.
    let src = format!(
        "{ASK}fn outer(body) {{\n\
           handle {{ body() }} with any {{\n\
             op(a) -> {{\n\
               let v = handle {{ op.perform(a) }} with any {{\n\
                 op(b) -> {{ io.println(op.show(a) <> \" via \" <> op.show(b))  resume(op.perform(b)) }}\n\
               }}\n\
               resume(v)\n\
             }}\n\
           }}\n\
         }}\n\
         pub fn main() -> Int {{ live(fn() {{ outer(fn() {{ prog() }}) }}) }}\n"
    );
    agree(&src, "inner-clause");
}

#[test]
fn what_is_not_compiled_is_refused_by_name() {
    // An operation value given to a function (spec D2).
    let src = "effect W { fn roll(s: Int) -> Int }\n\
        fn describe(o) { op.show(o) }\n\
        fn rec(body) { handle { body() } with any { op(o) -> { io.println(describe(o))  resume(op.perform(o)) } } }\n\
        pub fn main() -> Int { handle { rec(fn() { roll(6) }) } with { W.roll(s) -> resume(s) } }\n";
    let e = refusal(src);
    assert!(
        e.contains("operation value used other than by op.* or =="),
        "{e}"
    );
    // A decode at a non-regular type has no finite set of instances (spec D5).
    let src = "type Opt(a) { No, Yes(a) }\n\
        type Nest(a) { Z(a), S(Nest(Opt(a))) }\n\
        effect E { fn e() -> Nest(Int) }\n\
        fn replay(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"S(Z(Yes(1)))\")) } }\n\
        pub fn main() -> Int { match replay(fn() { e() }) { Z(n) -> 1  S(m) -> 2 } }\n";
    let e = refusal(src);
    assert!(e.contains("too many type instances"), "{e}");
}

// ---- The review (2026-10-17) -------------------------------------------------

#[test]
fn generic_helpers_at_the_answer_type() {
    // R1: a generic function called at the clause's answer type is cloned per
    // operation, as the clause is (it was cloned once, at the rigid type).
    let src = "effect W { fn roll(s: Int) -> Int }\n\
        fn id(x) { x }\n\
        fn rec(body) { handle { body() } with any { op(o) -> resume(id(op.perform(o))) } }\n\
        pub fn main() -> Int { handle { rec(fn() { roll(3) }) } with { W.roll(s) -> resume(s) } }\n";
    assert_eq!(agree(src, "helper-id").end, Ok("3".to_string()));
    let src = "type Box(a) { B(a) }\n\
        effect W { fn word() -> String  fn other(n: Int) -> String  fn num() -> Int  fn yes() -> Bool }\n\
        fn wrap(x) { B(x) }\n\
        fn twice(x) { unwrap(wrap(unwrap(wrap(x)))) }\n\
        fn unwrap(b) { match b { B(x) -> x } }\n\
        fn rec(body) { handle { body() } with any { op(o) -> {\n\
          let b = wrap(op.perform(o))\n\
          match b { B(x) -> { io.println(op.encode(o, twice(x)))  resume(twice(x)) } }\n\
        } } }\n\
        pub fn main() -> Int {\n\
          let s = handle { rec(fn() { let w = word() <> other(2)  if yes() { num() + 1 } else { 0 } }) } with {\n\
            W.word() -> resume(\"w\")  W.other(n) -> resume(\"o\")  W.num() -> resume(41)  W.yes() -> resume(True)\n\
          }\n\
          s\n\
        }\n";
    assert_eq!(agree(src, "helper-box").end, Ok("42".to_string()));
}

#[test]
fn comparing_an_operation_reaches_its_arguments() {
    // R2: an argument holding a function stops `o == o` as it stops the
    // evaluator's `==`, at run time, when that operation is performed.
    let src = "type Bx { B(fn(Int) -> Int) }\n\
        effect K { fn k(f: Bx) -> Int }\n\
        fn rec(body) { handle { body() } with any { op(o) -> { io.println(if o == o { \"same\" } else { \"diff\" })  resume(op.perform(o)) } } }\n\
        pub fn main() -> Int { handle { rec(fn() { k(B(fn(x) { x })) }) } with { K.k(f) -> resume(match f { B(g) -> g(1) }) } }\n";
    let got = agree(src, "eq-fn-data");
    assert!(got.end.is_err(), "{got:?}");
    let src = "effect K { fn k(f: fn(Int) -> Int) -> Int  fn n() -> Int }\n\
        fn rec(body) { handle { body() } with any { op(o) -> { io.println(if o != o { \"diff\" } else { \"same\" })  resume(op.perform(o)) } } }\n\
        pub fn main() -> Int {\n\
          let a = handle { rec(fn() { n() }) } with { K.k(f) -> resume(f(1))  K.n() -> resume(5) }\n\
          let b = handle { rec(fn() { k(fn(x) { x + a }) }) } with { K.k(f) -> resume(f(1))  K.n() -> resume(5) }\n\
          a + b\n\
        }\n";
    let got = agree(src, "eq-fn-arg");
    assert_eq!(got.text, "same\n", "{got:?}");
    assert!(got.end.is_err(), "{got:?}");
}
