//! Exact replay and handler-based testing (ROADMAP priority 3; spec
//! `docs/superpowers/specs/2026-10-10-elya-replay-design.md`). Every test runs the
//! declarations of `examples/05_replay.elya` -- the program, `record`, `replay`,
//! `live` and `fixed` -- under a `main` of its own: the handlers are the API.
//!
//! Negative controls (spec §3), each reverted: R1 -- `replay` answers a roll with
//! the recorded value without checking the die's sides -> the divergence test
//! fails (the changed game replays to a score); R2 -- `replay`'s return clause
//! accepts a log with entries left over -> the leftover test fails; R3 -- `replay`
//! answers a clock reading from a `Rolled` entry -> the order test fails.

use elya::parse::parse_module;
use elya::Session;

const EXAMPLE: &str = include_str!("../examples/05_replay.elya");

/// The example without its `main`, plus `main_src`; `main`'s Int value.
fn run(main_src: &str) -> i64 {
    let decls = &EXAMPLE[..EXAMPLE.find("pub fn main").expect("the example has a main")];
    let src = format!("{decls}{main_src}");
    elya::check_source("replay.elya", &src).unwrap_or_else(|e| panic!("check: {e}"));
    let (m, d) = parse_module(&Session::new(), &src);
    assert!(d.is_empty(), "parse: {d:?}");
    match elya::eval::run_module_value(&m) {
        Ok((_, elya::eval::Value::Int(n))) => n,
        Ok((_, v)) => panic!("main returned {v:?}"),
        Err(e) => panic!("{:?}", e.diag),
    }
}

/// Record a live run with `seed`, replay it with no world at all: the same
/// score, and every recorded answer used.
#[test]
fn a_recording_replays_exactly_for_every_seed() {
    for seed in 1..=20 {
        let v = run(&format!(
            "pub fn main() -> Int {{\n\
             \x20 match live(fn() {{ record(fn() {{ game(6, 6, 0) }}) }}, {seed}) {{\n\
             \x20   Rec(score, log) -> match replay(fn() {{ game(6, 6, 0) }}, log) {{\n\
             \x20     Replayed(x) -> if x == score {{ score }} else {{ 0 - 1 }}\n\
             \x20     Diverged(w, l) -> 0 - 2\n\
             \x20     Unconsumed(x, l) -> 0 - 3\n\
             \x20   }}\n\
             \x20 }}\n\
             }}\n"
        ));
        // Six rounds of a six-sided die, each scoring 1..12.
        assert!((6..=72).contains(&v), "seed {seed}: {v}");
    }
}

/// The changed program asks for an 8-sided die in its third round: replay
/// stops there, names the operation and its argument, and has consumed the
/// first two rounds' answers (a roll and two clock readings each).
#[test]
fn replay_names_the_first_divergence() {
    let v = run("pub fn main() -> Int {\n\
         \x20 match live(fn() { record(fn() { game(5, 6, 0) }) }, 7) {\n\
         \x20   Rec(score, log) -> match replay(fn() { game_changed(5, 0) }, log) {\n\
         \x20     Replayed(x) -> 0 - 1\n\
         \x20     Unconsumed(x, l) -> 0 - 3\n\
         \x20     Diverged(w, l) -> match w {\n\
         \x20       WantRoll(sides) -> sides * 100 + length(log) - length(l)\n\
         \x20       WantNow -> 0 - 2\n\
         \x20     }\n\
         \x20   }\n\
         \x20 }\n\
         }\n");
    assert_eq!(v, 806);
}

/// A recording of five rounds replayed into a four-round game: the program
/// finishes, but the run was not the same one.
#[test]
fn a_replay_with_answers_left_over_is_not_the_same_run() {
    let v = run("pub fn main() -> Int {\n\
         \x20 match live(fn() { record(fn() { game(5, 6, 0) }) }, 7) {\n\
         \x20   Rec(score, log) -> match replay(fn() { game(4, 6, 0) }, log) {\n\
         \x20     Replayed(x) -> 0 - 1\n\
         \x20     Diverged(w, l) -> 0 - 2\n\
         \x20     Unconsumed(x, l) -> length(l)\n\
         \x20   }\n\
         \x20 }\n\
         }\n");
    // Every round takes three answers (its roll, its clock reading, and the
    // reading `game` takes before the next round): round five's three are left.
    assert_eq!(v, 3);
}

/// The test double is just another handler: no mocks, no injection. All
/// sixes and no time passing make every round score 12.
#[test]
fn swapping_the_handler_is_the_test_double() {
    assert_eq!(
        run("pub fn main() -> Int { fixed(fn() { game(7, 6, 0) }) }\n"),
        84
    );
    assert_eq!(
        run("pub fn main() -> Int { fixed(fn() { game(3, 20, 0) }) }\n"),
        120
    );
}

/// A changed program that asks for the time BEFORE rolling: the recording's
/// first answer is a roll, so replay stops at once, naming `now`, with nothing
/// consumed. (Every other test asks in the recorded order, so without this one
/// a replay that answered from the wrong kind of entry would pass.)
#[test]
fn replay_answers_each_question_only_from_its_own_kind() {
    let v = run("fn round_swapped(sides: Int, last: Int) -> Int {\n\
         \x20 let t = now()\n\
         \x20 let r = roll(sides)\n\
         \x20 if t - last < 50 { 2 * r } else { r }\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 match live(fn() { record(fn() { game(5, 6, 0) }) }, 7) {\n\
         \x20   Rec(score, log) -> match replay(fn() { round_swapped(6, 0) }, log) {\n\
         \x20     Replayed(x) -> 0 - 1\n\
         \x20     Unconsumed(x, l) -> 0 - 3\n\
         \x20     Diverged(w, l) -> match w {\n\
         \x20       WantNow -> 1000 + length(log) - length(l)\n\
         \x20       WantRoll(sides) -> 0 - 2\n\
         \x20     }\n\
         \x20   }\n\
         \x20 }\n\
         }\n");
    assert_eq!(v, 1000);
}
