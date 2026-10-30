//! Row parameters on declarations (2026-10-30, spec
//! `docs/superpowers/specs/2026-10-30-elya-row-variables-design.md`, part 2): a
//! type or an effect can take a row, so a task type names "whatever else the
//! program performs" and a scheduler is written once.
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K1, kinds from row tails only (no propagation through another
//!   declaration's row parameter): the round robin fails, `Queue`'s parameter
//!   read as a type (three E0400);
//! - K2, an effect's row parameter not instantiated per perform and handle (the
//!   declaration's variable shared): failed nothing until
//!   `an_effects_row_parameter_is_instantiated_per_use` (E0420 under K2);
//! - K4, written label arguments dropped again: the label test (`[]`), and both
//!   schedulers and the per-use test (E0424: `{Async(e) | e}` in `sched`'s
//!   annotation loses its argument, so the row cannot be unified whole);
//! - K5, `settle_duplicates` off: `a_label_met_twice_along_a_row_is_unified`
//!   checks clean again.

use std::collections::HashMap;

fn codes(src: &str) -> Vec<String> {
    let (_, diags, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    diags.iter().map(|d| d.code.clone()).collect()
}

fn messages(src: &str) -> Vec<String> {
    let (_, diags, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    diags.iter().map(|d| d.message.clone()).collect()
}

/// main's value and output, or the rendered error.
fn run(src: &str) -> Result<(i64, String), String> {
    let o = elya::run_source_outcome("t.elya", src);
    match o.error {
        Some(e) => Err(e),
        None => Ok((o.value.expect("a value").expect("an Int"), o.output)),
    }
}

fn schemes(src: &str) -> HashMap<String, String> {
    let (m, pd) = elya::parse::parse_module(&elya::Session::new(), src);
    assert!(pd.is_empty(), "{pd:?}");
    let (s, diags) = elya::types::infer_schemes(&elya::Session::new(), &m);
    assert!(diags.is_empty(), "{diags:?}");
    s.into_iter().collect()
}

const LOG: &str = "effect Log { fn say(s: String) -> Unit }\n\
                   fn quiet(body) { handle { body() } with { Log.say(s) -> resume(Unit) } }\n";

/// D3, D4: `e` is a row parameter (it is a row's tail in `Task`); a task that
/// performs `Log` and a pure one are both `Task`s, and `run` performs what its
/// task does.
#[test]
fn a_type_with_a_row_parameter_holds_a_task() {
    let src = format!(
        "{LOG}type Task(e) {{ Task(fn() / {{e}} -> Int) }}\n\
         fn run(t: Task(e)) / {{e}} -> Int {{ match t {{ Task(f) -> f() }} }}\n\
         pub fn main() -> Int {{ quiet(fn() {{ run(Task(fn() {{ say(\"x\")  41 }})) }}) + run(Task(fn() {{ 1 }})) }}\n"
    );
    assert_eq!(codes(&src), Vec::<String>::new(), "{src}");
    assert_eq!(run(&src).map(|r| r.0), Ok(42));
}

/// D5: a use writes a row argument as a row, `Task({Log})` or `Task({})`, or a
/// row variable alone; a `Log` task is not a `Task({})`.
#[test]
fn a_row_argument_is_written_as_a_row() {
    let decl = "type Task(e) { Task(fn() / {e} -> Int) }\n\
                fn run(t: Task(e)) / {e} -> Int { match t { Task(f) -> f() } }\n";
    let logged = format!(
        "{LOG}{decl}pub fn main() -> Int {{ quiet(fn() {{ let t: Task({{Log}}) = Task(fn() {{ say(\"x\")  2 }})  run(t) }}) }}\n"
    );
    assert_eq!(run(&logged).map(|r| r.0), Ok(2), "{logged}");
    let pure = format!(
        "{LOG}{decl}pub fn main() -> Int {{ let t: Task({{}}) = Task(fn() {{ 3 }})  run(t) }}\n"
    );
    assert_eq!(run(&pure).map(|r| r.0), Ok(3), "{pure}");
    let wrong = format!(
        "{LOG}{decl}pub fn main() -> Int {{ quiet(fn() {{ let t: Task({{}}) = Task(fn() {{ say(\"x\")  2 }})  run(t) }}) }}\n"
    );
    assert_eq!(
        codes(&wrong),
        ["E0423"],
        "a `Log` task is not a `Task({{}})`:\n{wrong}"
    );
}

/// D3: `Queue`'s parameter is a row because it is `Task`'s argument there, found
/// through the declarations. The round-robin scheduler of `examples/04_async.elya`
/// written once for any other effects: the same output.
#[test]
fn a_row_parameter_is_found_through_another_declaration() {
    let src = include_str!("row_parameters/round_robin.elya");
    assert_eq!(codes(src), Vec::<String>::new());
    let want = "main: start\n  ping\nmain\n    pong\n  ping\nmain\n    pong\n  ping\nmain: done\n";
    assert_eq!(run(src), Ok((3, want.to_string())));
}

/// D3, D4: an effect's row parameter. A task performs `Async` and the program's
/// other effects (`{Async | e}`, `Async` being `Async(e)` in its own declaration),
/// so a task can spawn: with a type parameter for the task this was E0401, the
/// task's type inside its own effect's argument.
#[test]
fn an_effect_with_a_row_parameter_spawns_tasks_that_spawn() {
    let src = format!(
        "effect Async(e) {{ fn spawn(f: fn() / {{Async | e}} -> Int) -> Int }}\n{LOG}\
         fn sched(body: fn() / {{Async(e) | e}} -> Int) / {{e}} -> Int {{\n\
         \x20 handle {{ body() }} with {{ Async.spawn(f) -> resume(sched(f)) }}\n\
         }}\n\
         fn w() {{ spawn(fn() {{ say(\"t\")  spawn(fn() {{ 1 }}) + 5 }}) }}\n\
         pub fn main() -> Int {{ quiet(fn() {{ sched(fn() {{ w() }}) }}) }}\n"
    );
    assert_eq!(codes(&src), Vec::<String>::new(), "{src}");
    assert_eq!(run(&src).map(|r| r.0), Ok(6));
    // Unannotated, the scheduler's types are inferred the same way.
    let bare = src.replace(
        "fn sched(body: fn() / {Async(e) | e} -> Int) / {e} -> Int {",
        "fn sched(body) {",
    );
    assert_eq!(run(&bare).map(|r| r.0), Ok(6), "{bare}");
}

/// D4: each perform and each handle instantiates an effect's row parameter
/// afresh (control K2 found no test that told: one shared variable put `Log`, the
/// first scheduler's, into the second's, and `main` then "performed" it).
#[test]
fn an_effects_row_parameter_is_instantiated_per_use() {
    let src = format!(
        "effect Async(e) {{ fn spawn(f: fn() / {{Async | e}} -> Int) -> Int }}\n{LOG}\
         fn sched(body: fn() / {{Async(e) | e}} -> Int) / {{e}} -> Int {{\n\
         \x20 handle {{ body() }} with {{ Async.spawn(f) -> resume(sched(f)) }}\n\
         }}\n\
         pub fn main() -> Int {{\n\
         \x20 let a = quiet(fn() {{ sched(fn() {{ spawn(fn() {{ say(\"x\")  1 }}) }}) }})\n\
         \x20 a + sched(fn() {{ spawn(fn() {{ 2 }}) }})\n\
         }}\n"
    );
    assert_eq!(codes(&src), Vec::<String>::new(), "{src}");
    assert_eq!(run(&src).map(|r| r.0), Ok(3));
}

/// D3, D5's errors, each by name.
#[test]
fn a_parameter_is_a_type_or_a_row_by_name() {
    let task = "type Task(e) { Task(fn() / {e} -> Int) }\n";
    for (src, want) in [
        (
            "type T(e) { A(fn() / {e} -> Int), B(e) }\npub fn main() -> Int { 0 }\n".to_string(),
            "`e` is a row parameter of `T`: a type goes here",
        ),
        (
            format!("{task}fn f(t: Task(Int)) -> Int {{ 0 }}\npub fn main() -> Int {{ 0 }}\n"),
            "`Task`'s parameter `e` is a row: write one, `{Log}`",
        ),
        (
            "type Box(t) { Box(t) }\nfn f(b: Box({})) -> Int { 0 }\npub fn main() -> Int { 0 }\n"
                .to_string(),
            "`Box`'s parameter `t` is a type, not a row",
        ),
        (
            "effect State(s) { fn get() -> s }\ntype T { T(fn() / {State} -> Int) }\npub fn main() -> Int { 0 }\n"
                .to_string(),
            "effect `State` takes type arguments: write them, `State(..)`",
        ),
        (
            "type T(x) { T(fn() / {e} -> Int) }\npub fn main() -> Int { 0 }\n".to_string(),
            "a declaration's row cannot name a row variable (`e`) it does not declare",
        ),
    ] {
        assert_eq!(messages(&src), [want], "{src}");
    }
}

/// D5: a label's written arguments are checked (they were parsed and dropped):
/// `State(String)` is not the `State(Int)` the handler answers.
#[test]
fn a_labels_written_arguments_are_checked() {
    let decl = "effect State(s) { fn get() -> s }\n";
    let ok = format!(
        "{decl}fn f() / {{State(Int)}} -> Int {{ get() }}\npub fn main() -> Int {{ handle {{ f() }} with {{ State.get() -> resume(3) }} }}\n"
    );
    assert_eq!(run(&ok).map(|r| r.0), Ok(3), "{ok}");
    let wrong = ok.replace("{State(Int)}", "{State(String)}");
    assert_eq!(codes(&wrong), ["E0423"], "{wrong}");
}

/// D4: the printer writes a row argument as the row, not as its phantom.
#[test]
fn the_printer_writes_a_row_argument_as_a_row() {
    let s = schemes(
        "effect Log { fn say(s: String) -> Unit }\n\
         type Task(e) { Task(fn() / {e} -> Int) }\n\
         fn keep(t: Task({Log})) -> Task({Log}) { t }\n\
         fn run(t: Task(e)) / {e} -> Int { match t { Task(f) -> f() } }\n",
    );
    assert_eq!(s["keep"], "fn(Task({Log})) -> Task({Log})");
    assert_eq!(s["run"], "forall a. fn(Task({a})) / {a} -> Int");
}

const ASYNC: &str = "effect Async(e) { fn spawn(f: fn() / {Async | e} -> Int) -> Int }\n";

/// The review's F1: unifying a shared label's arguments can bind the other
/// row's tail (`Async`'s phantom binds `e`), and the rows were split as they
/// were before. A label was lost: `St(String)` beside `St(Int)`, a program that
/// checked clean and failed at run time (natively, a crash). And `{Log}` was
/// never matched, so a correct program was refused.
#[test]
fn a_rows_labels_are_read_again_after_its_arguments_unify() {
    let lost = format!(
        "{ASYNC}effect St(s) {{ fn get() -> s }}\n\
         fn take(body: fn() / {{Async(e) | e}} -> Int) / {{e}} -> Int {{\n\
         \x20 handle {{ body() }} with {{ Async.spawn(f) -> resume(0) }}\n\
         }}\n\
         fn lam() {{\n\
         \x20 let s = get()\n\
         \x20 let n = spawn(fn() {{ get() + 1 }})\n\
         \x20 if s == \"a\" {{ 1 }} else {{ 2 }}\n\
         }}\n\
         pub fn main() -> Int {{ handle {{ take(lam) }} with {{ St.get() -> resume(5) }} }}\n"
    );
    assert_eq!(
        codes(&lost),
        ["E0423"],
        "`St` at String and at Int:\n{lost}"
    );
    let k = format!(
        "{ASYNC}{LOG}fn take(body: fn() / {{Async(e) | e}} -> Int) -> Int {{ 7 }}\n\
         pub fn main() -> Int {{ let k: fn() / {{Async({{Log}}), Log}} -> Int = fn() {{ 0 }}  take(k) }}\n"
    );
    assert_eq!(run(&k).map(|r| r.0), Ok(7), "{k}");
    let job = format!(
        "{ASYNC}{LOG}type Job {{ J(fn() / {{Async({{Log}}), Log}} -> Int) }}\n\
         fn sched(body: fn() / {{Async(e) | e}} -> Int) / {{e}} -> Int {{\n\
         \x20 handle {{ body() }} with {{ Async.spawn(f) -> resume(sched(f)) }}\n\
         }}\n\
         pub fn main() -> Int {{\n\
         \x20 let j = J(fn() {{ spawn(fn() {{ say(\"child\")  3 }}) }})\n\
         \x20 match j {{ J(f) -> quiet(fn() {{ sched(f) }}) }}\n\
         }}\n"
    );
    assert_eq!(run(&job).map(|r| r.0), Ok(3), "{job}");
}

/// The review's F2: a written label argument is unified structurally. These ran
/// before label arguments were checked; the first draft refused each, reading
/// two types not yet equal (`Box(a)`, `Box(Int)`) as a conflict.
#[test]
fn a_written_label_argument_unifies_structurally() {
    let st = "effect St(s) { fn get() -> s }\n";
    for (src, want) in [
        (
            format!("{st}type Box(a) {{ Box(a) }}\nfn f() / {{St(Box(Int))}} -> Int {{ match get() {{ Box(n) -> 9 }} }}\npub fn main() -> Int {{ handle {{ f() }} with {{ St.get() -> resume(Box(4)) }} }}\n"),
            9,
        ),
        (
            format!("{st}type Pair(a, b) {{ P(a, b) }}\nfn f() / {{St(Pair(Int, String))}} -> Int {{ match get() {{ P(n, s) -> n }} }}\npub fn main() -> Int {{ handle {{ f() }} with {{ St.get() -> resume(P(5, \"x\")) }} }}\n"),
            5,
        ),
        (
            format!("{st}fn f() / {{St(fn(Int) -> Int)}} -> Int {{ let g = get()  g(1) }}\npub fn main() -> Int {{ handle {{ f() }} with {{ St.get() -> resume(fn(x) {{ x + 1 }}) }} }}\n"),
            2,
        ),
    ] {
        assert_eq!(run(&src).map(|r| r.0), Ok(want), "{src}");
    }
}

/// The review's F3: a scheduler that runs a spawned task in its clause, annotated
/// with the type inference gives it unannotated. The handle's residual (`e`)
/// was equated with an ambient that held `Async(e)` from the clause: E0424.
#[test]
fn a_scheduler_that_runs_a_task_in_its_clause_checks() {
    let src = format!(
        "{ASYNC}fn grab(body: fn() / {{Async(e) | e}} -> Int) / {{Async(e) | e}} -> Int {{\n\
         \x20 handle {{ body() }} with {{ Async.spawn(f) -> f() + 100 }}\n\
         }}\n\
         pub fn main() -> Int {{ handle {{ grab(fn() {{ spawn(fn() {{ 6 }}) }}) }} with {{ Async.spawn(f) -> resume(0) }} }}\n"
    );
    assert_eq!(codes(&src), Vec::<String>::new(), "{src}");
    assert_eq!(run(&src).map(|r| r.0), Ok(106));
}

/// The review's F4: what a mistake with a task's row is called. A function that
/// runs a task but declares the task's other effects alone performs `Async`
/// unhandled: named, not "cyclic effect row"; and a declaration's bare
/// row-parameterized effect is told its argument is a row.
#[test]
fn a_task_rows_mistakes_are_named() {
    let src = format!(
        "{ASYNC}fn bad(t: fn() / {{Async(e) | e}} -> Int) / {{e}} -> Int {{ t() }}\npub fn main() -> Int {{ 0 }}\n"
    );
    let (_, diags, _) = elya::front_end_with("t.elya", &src, &mut |_| None);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, "E0423");
    assert!(
        diags[0].helps.iter().any(|h| h.contains("Async")),
        "{diags:?}"
    );
    let bare = format!(
        "{ASYNC}type T(e) {{ T(fn() / {{Async | e}} -> Int) }}\npub fn main() -> Int {{ 0 }}\n"
    );
    let (_, diags, _) = elya::front_end_with("t.elya", &bare, &mut |_| None);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].helps.iter().any(|h| h.contains("`Async({Log})`")),
        "{diags:?}"
    );
}

/// Found checking the review's F3 fix: a label kept out of a row variable its
/// arguments mention can reach that variable later by another path, and
/// `resolve_row` kept the first of the two. `g`'s row holds `E(type of k)`; a `k`
/// that performs `E(Int)` put `E(Int)` beside it, dropped: the program checked
/// clean. Now their arguments are unified where rows meet: E0423.
#[test]
fn a_label_met_twice_along_a_row_is_unified() {
    let g = "effect E(t) { fn op(x: t) -> Unit }\nfn g(k) { op(k)  k() }\n";
    let pure = format!(
        "{g}pub fn main() -> Int {{ handle {{ g(fn() {{ 4 }}) }} with {{ E.op(x) -> resume(Unit) }} }}\n"
    );
    assert_eq!(run(&pure).map(|r| r.0), Ok(4), "{pure}");
    let twice = format!(
        "{g}fn self_op() {{ op(1)  2 }}\n\
         pub fn main() -> Int {{ handle {{ g(self_op) }} with {{ E.op(x) -> resume(Unit) }} }}\n"
    );
    assert_eq!(codes(&twice), ["E0423"], "{twice}");
}
