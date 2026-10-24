//! Operation values as values natively (2026-10-23, spec
//! `docs/superpowers/specs/2026-10-23-elya-operation-values-natively-design.md`).
//! A `with any` clause's operation `o` given to helpers, stored, captured,
//! returned and compared. Every program is compiled, linked and RUN, and its
//! output compared with the evaluator's (the `native_any.rs` harness).
//!
//! Negative controls (each reverted; `git diff --quiet` after):
//! - K1, clones keyed by answer type again (spec D2): five tests fail by
//!   `match failed (no arm matched)`, a helper clone shared by two operations
//!   that answer `Int`; the comparison test (one operation per answer) passes.
//! - K2, `o` built with its fields reversed (D1): four tests panic in inkwell
//!   (a field of the wrong width); the replayer and the one-argument test pass.
//! - K3, no rewrite in clones (D3): five tests are refused at build,
//!   `unsupported builtin (op.show)` and the like; the storing test, whose
//!   helpers never look inside, passes.

use elya::core::lower_module;
use elya::parse::parse_module;
use elya::Session;
use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-opv-{}-{tag}", std::process::id()));
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

/// A live handler for the effects below, and a recorder written with helpers.
const W: &str = "type Shape { Dot, Box(Int, Int) }\n\
    effect W {\n\
      fn roll(s: Int) -> Int\n\
      fn now() -> Int\n\
      fn word(w: String) -> String\n\
      fn mix(a: Int, b: String, c: Shape) -> Int\n\
      fn yes() -> Bool\n\
    }\n\
    fn live(body) {\n\
      handle { body() } with {\n\
        W.roll(s) -> resume(s + 1)\n\
        W.now() -> resume(7)\n\
        W.word(w) -> resume(w <> \"b\")\n\
        W.mix(a, b, c) -> resume(a * 10)\n\
        W.yes() -> resume(True)\n\
      }\n\
    }\n\
    fn prog() / {W} -> Int {\n\
      let w = word(\"a\")\n\
      let m = mix(4, \"q\\\"z\", Box(1, 2))\n\
      if yes() { roll(6) + now() + m } else { 0 }\n\
    }\n";

#[test]
fn helpers_take_the_operation() {
    // p1: a helper that names and shows the operation.
    let src = format!(
        "{W}fn describe(o) {{ op.name(o) <> \" asked \" <> op.show(o) }}\n\
         fn rec(body) {{ handle {{ body() }} with any {{ op(o) -> {{ io.println(describe(o))  resume(op.perform(o)) }} }} }}\n\
         pub fn main() -> Int {{ live(fn() {{ rec(fn() {{ prog() }}) }}) }}\n"
    );
    let got = agree(&src, "describe");
    assert_eq!(
        got.text,
        "W.word asked W.word(\"a\")\nW.mix asked W.mix(4, \"q\\\"z\", Box(1, 2))\nW.yes asked W.yes()\n\
         W.roll asked W.roll(6)\nW.now asked W.now()\n"
    );
    assert_eq!(got.end, Ok("54".to_string()));
    // p2: a helper that performs, and one that encodes an answer.
    let src = format!(
        "{W}fn forward(o) {{ op.perform(o) }}\n\
         fn logged(o, v) {{ op.show(o) <> \" = \" <> op.encode(o, v) }}\n\
         fn rec(body) {{ handle {{ body() }} with any {{ op(o) -> {{ let v = forward(o)  io.println(logged(o, v))  resume(v) }} }} }}\n\
         pub fn main() -> Int {{ live(fn() {{ rec(fn() {{ prog() }}) }}) }}\n"
    );
    let got = agree(&src, "forward");
    assert_eq!(
        got.text,
        "W.word(\"a\") = \"ab\"\nW.mix(4, \"q\\\"z\", Box(1, 2)) = 40\nW.yes() = True\nW.roll(6) = 7\nW.now() = 7\n"
    );
}

#[test]
fn operations_are_stored_captured_and_returned() {
    // p3: in a list twice, in a lambda, and back out through a generic `first`.
    let src = format!(
        "{W}type List(a) {{ Nil, Cons(a, List(a)) }}\n\
         type Opt(a) {{ No, Yes(a) }}\n\
         fn first(xs) {{ match xs {{ Nil -> No  Cons(x, r) -> Yes(x) }} }}\n\
         fn rec(body) {{ handle {{ body() }} with any {{ op(o) -> {{\n\
           let ops = Cons(o, Cons(o, Nil))\n\
           let k = fn() {{ op.show(o) }}\n\
           io.println(k())\n\
           match first(ops) {{ Yes(p) -> {{ io.println(op.name(p))  resume(op.perform(p)) }}  No -> resume(op.perform(o)) }}\n\
         }} }} }}\n\
         pub fn main() -> Int {{ live(fn() {{ rec(fn() {{ prog() }}) }}) }}\n"
    );
    let got = agree(&src, "stored");
    assert_eq!(got.end, Ok("54".to_string()));
    // p5: a closure that returns the operation, and a declared box around it.
    let src = format!(
        "{W}type Bx(a) {{ B(a, Int) }}\n\
         fn keep(o) {{ fn() {{ o }} }}\n\
         fn boxed(o) {{ B(o, 3) }}\n\
         fn rec(body) {{ handle {{ body() }} with any {{ op(o) -> {{\n\
           let g = keep(o)\n\
           match boxed(g()) {{ B(p, n) -> {{ io.println(op.show(p) <> \" \" <> int.to_string(n))  resume(op.perform(p)) }} }}\n\
         }} }} }}\n\
         pub fn main() -> Int {{ live(fn() {{ rec(fn() {{ prog() }}) }}) }}\n"
    );
    let got = agree(&src, "returned");
    assert_eq!(got.end, Ok("54".to_string()));
}

#[test]
fn two_operations_with_one_answer_type_get_their_own_helpers() {
    // D2: `roll` and `now` both answer `Int`; one helper clone each.
    let src = "effect W { fn roll(s: Int) -> Int  fn now() -> Int }\n\
        fn describe(o) { op.show(o) }\n\
        fn twice(o) { describe(o) <> \"/\" <> describe(o) }\n\
        fn rec(body) { handle { body() } with any { op(o) -> { io.println(twice(o))  resume(op.perform(o)) } } }\n\
        pub fn main() -> Int { handle { rec(fn() { roll(6) + now() }) } with { W.roll(s) -> resume(s)  W.now() -> resume(7) } }\n";
    let got = agree(src, "one-answer");
    assert_eq!(got.text, "W.roll(6)/W.roll(6)\nW.now()/W.now()\n");
    assert_eq!(got.end, Ok("13".to_string()));
}

#[test]
fn operations_compare_through_helpers() {
    // p4: a helper choosing between two operations, another comparing them.
    let src = "effect W { fn roll(s: Int) -> Int  fn put(n: Int, w: String) -> Unit }\n\
        fn pick(o, p) { if op.name(o) == \"W.roll\" { o } else { p } }\n\
        fn same(o, p) { o == p }\n\
        fn differ(o, p) { o != p }\n\
        fn rec(body) { handle { body() } with any { op(o) -> {\n\
          let q = pick(o, o)\n\
          io.println(if same(o, q) { \"same\" } else { \"diff\" })\n\
          io.println(if differ(q, o) { \"diff\" } else { \"same\" })\n\
          resume(op.perform(q))\n\
        } } }\n\
        pub fn main() -> Int { handle { rec(fn() { put(1, \"x\")  roll(6) }) } with { W.roll(s) -> resume(s)  W.put(n, w) -> resume(Unit) } }\n";
    let got = agree(src, "compare");
    assert_eq!(got.text, "same\nsame\nsame\nsame\n");
    // p7: an argument holding a function stops the comparison by name.
    let src = "type Bx { B(fn(Int) -> Int) }\n\
        effect K { fn k(f: Bx) -> Int }\n\
        fn same(o, p) { o == p }\n\
        fn rec(body) { handle { body() } with any { op(o) -> { io.println(\"before\")  io.println(if same(o, o) { \"same\" } else { \"diff\" })  resume(op.perform(o)) } } }\n\
        pub fn main() -> Int { handle { rec(fn() { k(B(fn(x) { x })) }) } with { K.k(f) -> resume(match f { B(g) -> g(1) }) } }\n";
    let got = agree(src, "compare-fn");
    assert_eq!(got.text, "before\n");
    assert!(got.end.is_err(), "{got:?}");
}

#[test]
fn a_replayer_built_from_helpers() {
    // A state-passing replayer whose clause reads the log through helpers, over
    // two unrelated effects; `op.decode` runs in a helper.
    let src = "type Log { End, Ent(String, String, Log) }\n\
        type Shape { Dot, Box(Int, Int) }\n\
        effect Dice { fn roll(s: Int) -> Int }\n\
        effect Shop { fn price(item: String) -> Int  fn shape() -> Shape }\n\
        fn question(o) { op.show(o) }\n\
        fn answer(o, a) { op.decode(o, a) }\n\
        fn matches(o, q) { q == question(o) }\n\
        fn replay(body, log) {\n\
          let run = handle { body() } with any {\n\
            op(o) -> fn(l) {\n\
              match l {\n\
                Ent(q, a, rest) -> if matches(o, q) { resume(answer(o, a))(rest) } else { 0 - 1 }\n\
                End -> 0 - 2\n\
              }\n\
            }\n\
            return(x) -> fn(l) { x }\n\
          }\n\
          run(log)\n\
        }\n\
        pub fn main() -> Int {\n\
          let log = Ent(\"Dice.roll(6)\", \"4\", Ent(\"Shop.price(\\\"tea\\\")\", \"30\", Ent(\"Shop.shape()\", \"Box(2, 5)\", End)))\n\
          replay(fn() { let d = roll(6)  let p = price(\"tea\")  match shape() { Box(a, b) -> d + p + a * b  Dot -> 0 } }, log)\n\
        }\n";
    let got = agree(src, "replayer");
    assert_eq!(got.end, Ok("44".to_string()));
}

#[test]
fn nested_handlers_pass_both_operations() {
    // An inner `with any` inside an outer clause: a helper takes both operations.
    let src = format!(
        "{W}fn via(a, b) {{ op.show(a) <> \" via \" <> op.name(b) }}\n\
         fn outer(body) {{\n\
           handle {{ body() }} with any {{\n\
             op(a) -> {{\n\
               let v = handle {{ op.perform(a) }} with any {{\n\
                 op(b) -> {{ io.println(via(a, b))  resume(op.perform(b)) }}\n\
               }}\n\
               resume(v)\n\
             }}\n\
           }}\n\
         }}\n\
         pub fn main() -> Int {{ live(fn() {{ outer(fn() {{ prog() }}) }}) }}\n"
    );
    let got = agree(&src, "nested");
    assert_eq!(got.end, Ok("54".to_string()));
}

// ---- The review (2026-10-23) -------------------------------------------------

#[test]
fn data_holding_an_operation_compares_by_the_evaluators_walk() {
    // F1: `==` on data around `o` walked the operation's fields without the
    // evaluator's stop at a function (it answered `eq`).
    for (cmp, tag) in [
        ("Yes(o) == Yes(o)", "eq-opt"),
        ("Cons(o, Nil) != Cons(o, Nil)", "ne-list"),
    ] {
        let src = format!(
            "type List(a) {{ Nil, Cons(a, List(a)) }}\n\
             type Opt(a) {{ No, Yes(a) }}\n\
             type Bx {{ B(fn(Int) -> Int) }}\n\
             effect K {{ fn k(f: Bx) -> Int }}\n\
             fn rec(body) {{ handle {{ body() }} with any {{ op(o) -> {{\n\
               io.println(\"before\")\n\
               io.println(if {cmp} {{ \"x\" }} else {{ \"y\" }})\n\
               resume(op.perform(o)) }} }} }}\n\
             pub fn main() -> Int {{ handle {{ rec(fn() {{ k(B(fn(x) {{ x }})) }}) }} with {{ K.k(f) -> resume(match f {{ B(g) -> g(1) }}) }} }}\n"
        );
        let got = agree(&src, tag);
        assert_eq!(got.text, "before\n", "{tag}");
        assert!(got.end.is_err(), "{tag}: {got:?}");
    }
    // Data that differs before the walk reaches the operation is unequal, with
    // no failure, on both sides.
    let src = "type List(a) { Nil, Cons(a, List(a)) }\n\
        effect W { fn roll(s: Int) -> Int }\n\
        fn rec(body) { handle { body() } with any { op(o) -> {\n\
          io.println(if Cons(o, Nil) == Nil { \"x\" } else { \"y\" })\n\
          io.println(if Cons(o, Nil) == Cons(o, Nil) { \"x\" } else { \"y\" })\n\
          resume(op.perform(o)) } } }\n\
        pub fn main() -> Int { handle { rec(fn() { roll(6) }) } with { W.roll(s) -> resume(s) } }\n";
    let got = agree(src, "data-eq");
    assert_eq!(got.text, "y\nx\n");
}

#[test]
fn helpers_that_never_see_the_operation_are_shared_per_answer_type() {
    // F2: thirty operations answering `Int` through a chain of nine helpers of
    // the answer only: one clone per answer type (as before this slice), not
    // 270, which passed the clone budget.
    let ops: Vec<String> = (0..30).map(|i| format!("fn o{i}() -> Int")).collect();
    let helpers: String = (0..8)
        .map(|i| format!("fn h{i}(x) {{ h{}(x) }}\n", i + 1))
        .collect();
    let sum: Vec<String> = (0..30).map(|i| format!("o{i}()")).collect();
    let clauses: Vec<String> = (0..30)
        .map(|i| format!("W.o{i}() -> resume({i})"))
        .collect();
    let src = format!(
        "effect W {{ {} }}\n{helpers}fn h8(x) {{ x }}\n\
         fn rec(body) {{ handle {{ body() }} with any {{ op(o) -> resume(h0(op.perform(o))) }} }}\n\
         pub fn main() -> Int {{ handle {{ rec(fn() {{ {} }}) }} with {{ {} }} }}\n",
        ops.join("  "),
        sum.join(" + "),
        clauses.join("  ")
    );
    assert_eq!(agree(&src, "budget").end, Ok("435".to_string()));
}

#[test]
fn an_inner_recorder_over_an_effect_at_the_outer_answer_type() {
    // F3: `spy` handles `Hold(t)` with `t` the outer clause's answer type; its
    // operation type mentioned that rigid when `spy` was expanded and was
    // refused, though the program ran natively before this slice.
    let src = "effect Hold(s) { fn peek() -> s }\n\
        effect W { fn roll(s: Int) -> Int  fn now() -> Int }\n\
        fn spy(body) { handle { body() } with any { op(b) -> { io.println(\"spy \" <> op.name(b) <> \" \" <> op.show(b))  resume(op.perform(b)) } } }\n\
        fn rec(body) { handle { body() } with any { op(o) -> {\n\
          let v = handle { spy(fn() { peek() }) } with { Hold.peek() -> resume(op.perform(o)) }\n\
          resume(v) } } }\n\
        pub fn main() -> Int { handle { rec(fn() { roll(6) + now() }) } with { W.roll(s) -> resume(s)  W.now() -> resume(7) } }\n";
    let got = agree(src, "hold");
    assert_eq!(
        got.text,
        "spy Hold.peek Hold.peek()\nspy Hold.peek Hold.peek()\n"
    );
    assert_eq!(got.end, Ok("13".to_string()));
}
