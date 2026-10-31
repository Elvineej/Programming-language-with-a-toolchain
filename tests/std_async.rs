//! Async as a standard module (2026-10-31, spec
//! `docs/superpowers/specs/2026-10-31-elya-async-module-design.md`):
//! `elya/async` is one scheduler for every program, its tasks performing
//! `Async(a, e)` and whatever else `e` is, handled outside the scope.
//!
//! Negative controls (each reverted; `src/std/` compared byte for byte after):
//! - K1, `pending` reporting no task (the deadlock rule's answer): the deadlock
//!   test alone (the first draft's counter, off, looped until the time limit);
//! - K2, timers not kept in deadline order: the two order tests;
//! - K3, `async_host.timed` not sleeping: the host test alone;
//! - K4, `elya/host` importing `elya/async` again (the review's R2): the
//!   host-names test alone (E0209).
//!
//! The many-tasks test was red on the first draft's scheduler (over the
//! 60-second limit); it runs in about a second now.

/// A reader over `files`: module path -> (file name, text).
fn reader<'a>(
    files: &'a [(&'a str, &'a str)],
) -> impl FnMut(&str) -> Option<(String, String)> + 'a {
    move |path: &str| {
        files
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(p, t)| (format!("{p}.elya"), t.to_string()))
    }
}

/// What the program printed and main's value, on an 8 MiB thread that must
/// finish in 60 s (a deadlock that loops fails here, not the run).
fn run_with(
    root: &'static str,
    files: &'static [(&'static str, &'static str)],
) -> Result<(String, i64), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let mut read = reader(files);
            let o = elya::run_source_outcome_with("main.elya", root, &mut read);
            let r = match o.error {
                Some(e) => Err(e),
                None => Ok((o.output, o.value.expect("a value").expect("an Int"))),
            };
            let _ = tx.send(r);
        })
        .expect("a thread");
    rx.recv_timeout(std::time::Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("the program did not finish in 60 s:\n{root}"))
}

fn run(root: &'static str) -> Result<(String, i64), String> {
    run_with(root, &[])
}

/// D1, D2: a scope joins every task it started and answers the body's value.
#[test]
fn a_scope_joins_its_tasks_and_answers_their_value() {
    let src = "import elya/async\n\
               fn fib(n: Int) -> Int {\n\
               \x20 if n < 2 { n } else {\n\
               \x20   let a = async_spawn(fn() { fib(n - 1) })\n\
               \x20   let b = async_spawn(fn() { fib(n - 2) })\n\
               \x20   async_await(a) + async_await(b)\n\
               \x20 }\n\
               }\n\
               pub fn main() -> Int {\n\
               \x20 match async.scope(fn() { fib(10) }) { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 - 1 }\n\
               }\n";
    assert_eq!(run(src), Ok((String::new(), 55)));
}

/// D1: one scope's tasks share a result type, another scope's may differ; the
/// tasks' other effects (`Log`) are handled outside.
#[test]
fn a_scopes_tasks_share_a_result_type_and_their_other_effects_go_out() {
    let src = "import elya/async\n\
               effect Log { fn say(s: String) -> Unit }\n\
               fn done(e) { match e { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> \"deadlock\" } }\n\
               pub fn main() -> Int {\n\
               \x20 handle {\n\
               \x20   let w = done(async.scope(fn() { let x = async_spawn(fn() { say(\"in a task\")  \"spawned\" })  async_await(x) <> \" and awaited\" }))\n\
               \x20   say(w)\n\
               \x20   match async.scope(fn() { async_await(async_spawn(fn() { 41 })) + 1 }) { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 }\n\
               \x20 } with { Log.say(s) -> { io.println(s)  resume(Unit) } }\n\
               }\n";
    assert_eq!(
        run(src),
        Ok(("in a task\nspawned and awaited\n".to_string(), 42))
    );
}

/// D2: tasks that await each other end the scope with their ids, by name (06's
/// scheduler looped without end, measured).
#[test]
fn a_deadlock_is_answered_by_name() {
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
    // Tasks 0 (the body), 1 and 2 are stuck.
    assert_eq!(run(src), Ok((String::new(), 300)));
}

/// D3: sleeping tasks wake in deadline order on a scope's own virtual clock.
#[test]
fn sleeping_tasks_wake_in_deadline_order() {
    const SRC: &str = concat!(
        "import elya/async\n",
        "effect Log { fn say(s: String) -> Unit }\n",
        "fn fetch(name: String, ms: Int) -> Int { async_sleep(ms)  say(name)  ms }\n",
        "fn quiet(body) { handle { body() } with { Log.say(s) -> { io.println(s)  resume(Unit) } } }\n",
        "pub fn main() -> Int {\n",
        "  quiet(fn() { match async.scope(fn() {\n",
        "    let a = async_spawn(fn() { fetch(\"a\", 30) })\n",
        "    let b = async_spawn(fn() { fetch(\"b\", 10) })\n",
        "    let c = async_spawn(fn() { fetch(\"c\", 20) })\n",
        "    async_await(a) + async_await(b) + async_await(c)\n",
        "  }) { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 - 1 } })\n",
        "}\n"
    );
    assert_eq!(run(SRC), Ok(("b\nc\na\n".to_string(), 60)));
}

/// D3: `async.schedule` takes its clock as two functions: here a virtual one
/// that says when it waits. Two requests of 30 and 10 ms: it waits until 10,
/// then until 30.
#[test]
fn a_scheduler_takes_its_clock_as_functions() {
    const SRC: &str = concat!(
        "import elya/async\n",
        "effect Log { fn say(s: String) -> Unit }\n",
        "fn fetch(name: String, ms: Int) -> Int { async_sleep(ms)  say(name)  ms }\n",
        "fn quiet(body) { handle { body() } with { Log.say(s) -> { io.println(s)  resume(Unit) } } }\n",
        "pub fn main() -> Int {\n",
        "  quiet(fn() {\n",
        "    let ended = async.schedule(fn() {\n",
        "      let a = async_spawn(fn() { fetch(\"a\", 30) })\n",
        "      let b = async_spawn(fn() { fetch(\"b\", 10) })\n",
        "      async_await(a) + async_await(b)\n",
        "    }, fn(t: Int) { t }, fn(t: Int, d: Int) { say(\"wait until \" <> int.to_string(d))  d })\n",
        "    match ended { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 - 1 }\n",
        "  })\n",
        "}\n"
    );
    assert_eq!(
        run(SRC),
        Ok(("wait until 10\nb\nwait until 30\na\n".to_string(), 40))
    );
}

/// D3: the machine's clock is `elya/async_host`'s (`async_host.timed`), which
/// needs `IO` granted: a sleep really waits.
#[test]
fn the_host_scheduler_really_sleeps() {
    const SRC: &str = concat!(
        "import elya/async\n",
        "import elya/async_host / {IO}\n",
        "pub fn main() -> Int {\n",
        "  let t0 = time.now_ms()\n",
        "  let v = match async_host.timed(fn() { async_sleep(30)  1 }) { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 }\n",
        "  if time.now_ms() - t0 >= 30 { v } else { 0 }\n",
        "}\n"
    );
    assert_eq!(run(SRC), Ok((String::new(), 1)));
}

/// D5: `elya/async` itself is imported without a grant (a scope's row is its
/// tasks' other effects alone); a dependency that spawns tasks performs
/// `Async`, so spawning is a capability its importer grants.
#[test]
fn spawning_is_a_capability_a_dependency_is_granted() {
    const DEP: &str = concat!(
        "import elya/async\n",
        "pub fn both(f, g) { let a = async_spawn(f)  let b = async_spawn(g)  async_await(a) + async_await(b) }\n"
    );
    const ROOT: &str = concat!(
        "import elya/async\n",
        "import lib/dep / {Async}\n",
        "pub fn main() -> Int {\n",
        "  match async.scope(fn() { dep.both(fn() { 20 }, fn() { 22 }) }) { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 }\n",
        "}\n"
    );
    assert_eq!(run_with(ROOT, &[("lib/dep", DEP)]), Ok((String::new(), 42)));
    const UNGRANTED: &str = concat!(
        "import elya/async\n",
        "import lib/dep\n",
        "pub fn main() -> Int {\n",
        "  match async.scope(fn() { dep.both(fn() { 20 }, fn() { 22 }) }) { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 }\n",
        "}\n"
    );
    let e = run_with(UNGRANTED, &[("lib/dep", DEP)]).expect_err("refused");
    assert!(
        e.contains("module `lib/dep` can perform `Async`, which this import does not grant"),
        "{e}"
    );
}

/// The review's R2: `elya/host` does not bring `elya/async`'s names with it. A
/// program that declares its own `Async` (as the examples do) still imports the
/// host's handlers: the machine's scheduler is a module of its own.
#[test]
fn the_host_module_reserves_no_async_name() {
    const SRC: &str = concat!(
        "import elya/host / {IO}\n",
        "effect Async { fn spawn(f: fn() -> Unit) -> Unit }\n",
        "type AsyncPair { P }\n",
        "pub fn main() -> Int { host.console(fn() { 7 }) }\n"
    );
    assert_eq!(run(SRC), Ok((String::new(), 7)));
}

/// The review's R3: a task waiting in `async_await` is parked until the task it
/// waits for finishes, not polled every round (fib(15), about 2 000 tasks, took
/// 236 s in the evaluator; fib(14) was over the 60-second limit here).
#[test]
fn many_tasks_are_scheduled_without_polling_the_waiting() {
    const SRC: &str = concat!(
        "import elya/async\n",
        "fn fib(n: Int) -> Int {\n",
        "  if n < 2 { n } else {\n",
        "    let a = async_spawn(fn() { fib(n - 1) })\n",
        "    let b = async_spawn(fn() { fib(n - 2) })\n",
        "    async_await(a) + async_await(b)\n",
        "  }\n",
        "}\n",
        "pub fn main() -> Int { match async.scope(fn() { fib(14) }) { AsyncFinished(v) -> v  AsyncDeadlocked(ids) -> 0 - 1 } }\n"
    );
    assert_eq!(run(SRC), Ok((String::new(), 377)));
}
