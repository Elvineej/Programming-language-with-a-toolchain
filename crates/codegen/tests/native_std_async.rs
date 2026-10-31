//! `elya/async`, natively (2026-10-31, spec
//! `docs/superpowers/specs/2026-10-31-elya-async-module-design.md`): a native
//! binary prints what the evaluator prints for the standard scheduler.

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
    let dir = std::env::temp_dir().join(format!("elya-async-{}-{tag}", std::process::id()));
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

fn agree(src: &str, tag: &str, want: &str) {
    let ev = evaluated(src);
    assert_eq!(ev, want, "{tag}: the evaluator");
    assert_eq!(native(src, tag), ev, "{tag}: native vs evaluator");
}

const LOG: &str = "effect Log { fn say(s: String) -> Unit }\n\
                   fn quiet(body) { handle { body() } with { Log.say(s) -> { io.println(s)  resume(Unit) } } }\n\
                   fn fetch(name: String, ms: Int) -> Int { async_sleep(ms)  say(name)  ms }\n";

/// D1, D2: tasks that spawn tasks, joined; tasks of another scope answer a
/// `String` and say things outside.
#[test]
fn scopes_join_their_tasks_natively() {
    let src = format!(
        "import elya/async\n{LOG}\
         fn fib(n: Int) -> Int {{\n\
         \x20 if n < 2 {{ n }} else {{\n\
         \x20   let a = async_spawn(fn() {{ fib(n - 1) }})\n\
         \x20   let b = async_spawn(fn() {{ fib(n - 2) }})\n\
         \x20   async_await(a) + async_await(b)\n\
         \x20 }}\n\
         }}\n\
         pub fn main() -> Int {{\n\
         \x20 quiet(fn() {{\n\
         \x20   let w = match async.scope(fn() {{ let x = async_spawn(fn() {{ say(\"in a task\")  \"spawned\" }})  async_await(x) }}) {{ AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> \"deadlock\" }}\n\
         \x20   say(w)\n\
         \x20   match async.scope(fn() {{ fib(10) }}) {{ AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 - 1 }}\n\
         \x20 }})\n\
         }}\n"
    );
    agree(&src, "join", "in a task\nspawned\n55\n");
}

/// D2: a deadlock is answered by name, natively too.
#[test]
fn a_deadlock_is_answered_natively() {
    let src = "import elya/async\n\
               fn sum(l) { match l { AsyncNil -> 0  AsyncCons(x, rest) -> x + sum(rest) } }\n\
               pub fn main() -> Int {\n\
               \x20 let ended = async.scope(fn() {\n\
               \x20   let a = async_spawn(fn() { async_await(2) })\n\
               \x20   let b = async_spawn(fn() { async_await(1) })\n\
               \x20   async_await(a) + async_await(b)\n\
               \x20 })\n\
               \x20 match ended { AsyncFinished(v) -> 0 - 1  AsyncDeadlocked(ids) -> sum(ids) * 100 }\n\
               }\n";
    agree(src, "dead", "300\n");
}

/// D3: sleepers wake in deadline order on the virtual clock, and on a clock
/// given as functions.
#[test]
fn sleepers_wake_in_order_natively() {
    let src = format!(
        "import elya/async\n{LOG}\
         fn three() -> Int {{\n\
         \x20 let a = async_spawn(fn() {{ fetch(\"a\", 30) }})\n\
         \x20 let b = async_spawn(fn() {{ fetch(\"b\", 10) }})\n\
         \x20 let c = async_spawn(fn() {{ fetch(\"c\", 20) }})\n\
         \x20 async_await(a) + async_await(b) + async_await(c)\n\
         }}\n\
         pub fn main() -> Int {{\n\
         \x20 quiet(fn() {{\n\
         \x20   let v = match async.scope(fn() {{ three() }}) {{ AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 - 1 }}\n\
         \x20   let w = match async.schedule(fn() {{ three() }}, fn(t: Int) {{ t }}, fn(t: Int, d: Int) {{ say(\"wait until \" <> int.to_string(d))  d }}) {{ AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 - 1 }}\n\
         \x20   v * 1000 + w\n\
         \x20 }})\n\
         }}\n"
    );
    agree(
        &src,
        "sleep",
        "b\nc\na\nwait until 10\nb\nwait until 20\nc\nwait until 30\na\n60060\n",
    );
}

/// The review's R1, an older native limit (HANDOFF's native-limits step): a
/// scope written inline in a task that also performs the outer `Async` is
/// refused by name (the evaluator answers 4). When the limit is lifted, this
/// test flips to `agree`.
#[test]
fn a_nested_scope_inline_is_refused_by_name_natively() {
    let src = "import elya/async\n\
               pub fn main() -> Int {\n\
               \x20 match async.scope(fn() {\n\
               \x20   let s = match async.scope(fn() { 4 }) { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 - 1 }\n\
               \x20   async_yield()\n\
               \x20   s\n\
               \x20 }) { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 - 2 }\n\
               }\n";
    assert_eq!(evaluated(src), "4\n");
    let (module, diags, _) = elya::front_end_with("main.elya", src, &mut no_files);
    assert!(diags.is_empty(), "{diags:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diags.is_empty(), "{diags:?}");
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let dir = std::env::temp_dir().join(format!("elya-async-{}-nested", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let err = elya_codegen::compile_module(&core, &dir.join("nested.o")).expect_err("refused");
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        err.to_string()
            .contains("calling convention disagrees with the callee"),
        "{err}"
    );
}
