//! Row parameters on declarations, natively (2026-10-30, spec
//! `docs/superpowers/specs/2026-10-30-elya-row-variables-design.md`, part 2,
//! D6): a native binary prints what the evaluator prints, for a type and an
//! effect that take a row.
//!
//! Negative control (reverted; the file compared byte for byte after): K3, the
//! instance builder without the row substitution (`mono::ctor_fields_at`): the
//! two programs whose types take a row are refused by name; the spawning
//! scheduler, whose types take none, still runs.

use std::process::Command;

fn no_files(_: &str) -> Option<(String, String)> {
    None
}

/// The evaluator's output, then main's value line.
fn evaluated(src: &str) -> String {
    let o = elya::run_source_outcome_with("main.elya", src, &mut no_files);
    assert!(o.error.is_none(), "{:?}", o.error);
    let line = o.line.expect("a value line").expect("a value");
    format!("{}{line}\n", o.output)
}

/// The native binary's stdout (its output, then main's value line).
fn native(src: &str, tag: &str) -> String {
    let (module, diags, _) = elya::front_end_with("main.elya", src, &mut no_files);
    assert!(diags.is_empty(), "{tag}: {diags:?}");
    let session = elya::Session::new();
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    assert!(diags.is_empty(), "{tag}: {diags:?}");
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let dir = std::env::temp_dir().join(format!("elya-rowp-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).unwrap_or_else(|e| panic!("{tag}: {e}"));
    elya_codegen::link(&obj, &exe).expect("link");
    let out = Command::new(&exe).output().expect("run the binary");
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        out.status.success(),
        "{tag}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("UTF-8 stdout")
}

const LOG: &str = "effect Log { fn say(s: String) -> Unit }\n\
                   fn quiet(body) { handle { body() } with { Log.say(s) -> { io.println(s)  resume(Unit) } } }\n";

/// `Task({Log})` and `Task({})` in one program: two instances (by convention),
/// each field read back at its own.
#[test]
fn a_type_at_two_rows_is_two_instances() {
    let src = format!(
        "{LOG}type Task(e) {{ Task(fn() / {{e}} -> Int) }}\n\
         type Queue(e) {{ Empty, Push(Task(e), Queue(e)) }}\n\
         fn run(t: Task(e)) / {{e}} -> Int {{ match t {{ Task(f) -> f() }} }}\n\
         fn all(q: Queue(e)) / {{e}} -> Int {{ match q {{ Empty -> 0  Push(t, rest) -> run(t) + all(rest) }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 let logged = quiet(fn() {{ all(Push(Task(fn() {{ say(\"x\")  2 }}), Push(Task(fn() {{ 3 }}), Empty))) }})\n\
         \x20 logged + all(Push(Task(fn() {{ 30 }}), Empty)) + run(Task(fn() {{ 7 }}))\n\
         }}\n"
    );
    let want = evaluated(&src);
    assert_eq!(want, "x\n42\n");
    assert_eq!(native(&src, "two"), want);
}

/// An effect's row parameter: tasks that perform `Log` and spawn tasks.
#[test]
fn an_effect_with_a_row_parameter_spawns_natively() {
    let src = format!(
        "effect Async(e) {{ fn spawn(f: fn() / {{Async | e}} -> Int) -> Int }}\n{LOG}\
         fn sched(body: fn() / {{Async(e) | e}} -> Int) / {{e}} -> Int {{\n\
         \x20 handle {{ body() }} with {{ Async.spawn(f) -> resume(sched(f)) }}\n\
         }}\n\
         fn w() {{ spawn(fn() {{ say(\"t\")  spawn(fn() {{ say(\"u\")  1 }}) + 5 }}) }}\n\
         pub fn main() -> Int {{ quiet(fn() {{ sched(fn() {{ w() }}) }}) }}\n"
    );
    let want = evaluated(&src);
    assert_eq!(want, "t\nu\n6\n");
    assert_eq!(native(&src, "spawn"), want);
}

/// The round robin of `examples/04_async.elya`, written once for any other
/// effects (`tests/row_parameters/round_robin.elya`).
#[test]
fn the_round_robin_runs_natively() {
    let src = include_str!("../../../tests/row_parameters/round_robin.elya");
    let want = evaluated(src);
    assert!(
        want.starts_with("main: start\n  ping\n") && want.ends_with("main: done\n3\n"),
        "{want}"
    );
    assert_eq!(native(src, "rr"), want);
}

/// A ground type holding row arguments in its fields and in a label's argument
/// (`J(fn() / {Async({Log}), Log} -> Int)`), handed to a scheduler: the part 2
/// review's F1 (it was refused, `{Log}` never matched).
#[test]
fn a_ground_type_with_row_arguments_runs_natively() {
    let src = format!(
        "{LOG}type Task(e) {{ Task(fn() / {{e}} -> Int) }}\n\
         type Holder {{ H(Task({{Log}}), Task({{}})) }}\n\
         effect Async(e) {{ fn spawn(f: fn() / {{Async | e}} -> Int) -> Int }}\n\
         type Job {{ J(fn() / {{Async({{Log}}), Log}} -> Int) }}\n\
         fn run(t: Task(e)) / {{e}} -> Int {{ match t {{ Task(f) -> f() }} }}\n\
         fn sched(body: fn() / {{Async(e) | e}} -> Int) / {{e}} -> Int {{\n\
         \x20 handle {{ body() }} with {{ Async.spawn(f) -> resume(sched(f)) }}\n\
         }}\n\
         pub fn main() -> Int {{\n\
         \x20 let h = H(Task(fn() {{ say(\"h\")  1 }}), Task(fn() {{ 2 }}))\n\
         \x20 let j = J(fn() {{ spawn(fn() {{ say(\"child\")  3 }}) }})\n\
         \x20 let a = match h {{ H(x, y) -> quiet(fn() {{ run(x) }}) + run(y) }}\n\
         \x20 let b = match j {{ J(f) -> quiet(fn() {{ sched(f) }}) }}\n\
         \x20 a + b\n\
         }}\n"
    );
    let want = evaluated(&src);
    assert_eq!(want, "h\nchild\n6\n");
    assert_eq!(native(&src, "ground"), want);
}
