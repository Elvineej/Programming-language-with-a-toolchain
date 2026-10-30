//! The occurs checks look inside effect rows (2026-10-30, spec
//! `docs/superpowers/specs/2026-10-30-elya-occurs-in-rows-design.md`). A type
//! that would contain itself through an effect's argument is a named error, not
//! a checker that recurses until the stack runs out.
//!
//! Negative controls (each reverted; the file compared byte for byte after),
//! each program also checked with the CLI:
//! - K1, `occurs` without the row's labels (D1): the scheduler overflows again;
//!   the two row cycles stay E0424;
//! - K2, `bind_row` without the labels' arguments (D2): the two row cycles
//!   overflow again; the scheduler stays E0401;
//! - K3, `flush_inclusions`' member loop without its poisoned-row skip (D3):
//!   the respawning tasks loop until the 60-second limit. (The first loop's
//!   skip, the same rule, is reached by no test here: off alone, all pass.)

/// The front end's diagnostic codes for `src`, on an 8 MiB thread: a checker
/// that recursed without end aborts the test binary here, and one that loops
/// without end fails the test after 60 seconds (the review's F1: a poisoned row
/// kept the inclusion fixpoint from settling).
fn codes(src: &'static str) -> Vec<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let (_, diags, _) = elya::front_end_with("t.elya", src, &mut |_| None);
            let _ = tx.send(diags.iter().map(|d| d.code.clone()).collect());
        })
        .expect("a thread");
    rx.recv_timeout(std::time::Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("the checker did not finish in 60 s:\n{src}"))
}

#[test]
fn a_type_inside_its_own_row_is_a_named_error() {
    // A row variable inside its own label's argument: `f`'s row is `{E(type of f)}`.
    let row = "effect E(t) { fn op(x: t) -> Unit }\nfn f() { op(f) }\npub fn main() -> Int { 0 }\n";
    assert_eq!(codes(row), ["E0424"], "{row}");
    // A parameter performed and called. It was E0424: a call of a parameter
    // equated `k`'s row with `g`'s, which then held `E(type of k)`. Since row
    // parameters (spec 2026-10-30-elya-row-variables, the part 2 review's F3) a
    // label whose arguments mention the relayed row stays the caller's own:
    // `g : fn(fn() / {a} -> b) / {E(fn() / {a} -> b) | a} -> b`, a finite type.
    // A `k` that performs `E` itself is refused at the call
    // (`tests/row_parameters.rs`, `a_label_met_twice_along_a_row_is_unified`).
    let ty =
        "effect E(t) { fn op(x: t) -> Unit }\nfn g(k) { op(k)  k() }\npub fn main() -> Int { 0 }\n";
    assert_eq!(codes(ty), Vec::<String>::new(), "{ty}");
    // A type variable inside a row of its own type, directly (D1):
    // `k: fn(fn() / {E(type of k)} -> ..) -> ..`.
    let direct = "effect E(t) { fn op(x: t) -> Unit }\nfn g(k) { k(fn() { op(k) }) }\npub fn main() -> Int { 0 }\n";
    assert_eq!(codes(direct), ["E0401"], "{direct}");
    // The async scheduler whose task type is its own effect's argument.
    let sched = "effect Async(t) { fn spawn(f: t) -> Int }\n\
                 fn sched(body) { handle { body() } with { Async.spawn(f) -> resume(sched(f)) } }\n\
                 fn w() { spawn(fn() { spawn(fn() { 1 }) + 5 }) }\n\
                 pub fn main() -> Int { sched(fn() { w() }) }\n";
    assert_eq!(codes(sched), ["E0401"], "{sched}");
}

/// A task that respawns itself (the review's F1): the cycle is E0424, once, and
/// the checker finishes. Before the fix it overflowed; with the occurs check
/// alone, the poisoned row kept `flush_inclusions` looping.
#[test]
fn a_cycle_found_in_the_inclusion_fixpoint_ends_it() {
    for src in [
        "effect Async(t) { fn spawn(f: t) -> Int }\nfn worker() { spawn(fn() { worker() }) }\npub fn main() -> Int { 0 }\n",
        "effect E(t) { fn op(x: t) -> Unit }\nfn f() { op(fn() { f() }) }\npub fn main() -> Int { 0 }\n",
        "effect E(t) { fn op(x: t) -> Unit }\nfn f(x) { op(fn() { f(x) })  f(x) }\npub fn main() -> Int { 0 }\n",
        "effect E(t) { fn op(x: t) -> Unit }\nfn f() { let h = fn() { op(fn() { f() }) }  h() }\npub fn main() -> Int { 0 }\n",
    ] {
        let got = codes(src);
        assert!(!got.is_empty() && got.iter().all(|c| c == "E0424" || c == "E0401"), "{got:?}\n{src}");
    }
}

/// The same effects without a cycle: a task-typed effect parameter is fine when
/// the task does not perform the effect it is the argument of.
#[test]
fn a_task_typed_effect_without_a_cycle_runs() {
    let src = "effect Async(t) { fn spawn(f: t) -> Int }\n\
               effect Log { fn say(s: String) -> Unit }\n\
               fn w() { spawn(fn() { say(\"t\")  5 }) }\n\
               pub fn main() -> Int { handle { handle { w() } with { Async.spawn(f) -> resume(f()) } } with { Log.say(s) -> resume(Unit) } }\n";
    assert!(codes(src).is_empty(), "{:?}", codes(src));
    // An open row relayed into a label's argument, without a cycle: `w`'s task
    // calls `k`, which `w` does not perform.
    let relay = "effect Async(t) { fn spawn(f: t) -> Int }\nfn w(k) { spawn(fn() { k() }) }\npub fn main() -> Int { 0 }\n";
    assert!(codes(relay).is_empty(), "{:?}", codes(relay));
    let o = elya::run_source_outcome("t.elya", src);
    assert_eq!(o.error, None);
    assert_eq!(o.value, Some(Ok(5)));
}
