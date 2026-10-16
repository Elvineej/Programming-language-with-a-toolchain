//! Time as a builtin source (2026-10-15, spec `2026-10-15-elya-async-step2-timers-design.md`
//! D3). `time.now_ms() -> Int` is monotonic milliseconds since an arbitrary start, and
//! `time.sleep_ms(ms: Int) -> Unit` blocks for at least `ms`. Both perform `IO`. A
//! scheduler reaches them only through a `Clock` handler (`examples/08_timers.elya`),
//! so its tests can run on a virtual clock instead.

use elya::{check_source, run_source};

#[test]
fn time_builtins_have_their_types_and_perform_io() {
    let ok = "fn elapsed(f) { let a = time.now_ms()  f()  time.now_ms() - a }\n\
              pub fn main() { io.println(if elapsed(fn() { time.sleep_ms(1) }) >= 1 { \"ok\" } else { \"short\" }) }\n";
    assert!(
        check_source("t.elya", ok).is_ok(),
        "{:?}",
        check_source("t.elya", ok)
    );
    // They perform IO: a function declared pure may not call them.
    for bad in [
        "fn f() / {} -> Int { time.now_ms() }\npub fn main() { io.println(\"x\") }\n",
        "fn f() / {} -> Unit { time.sleep_ms(1) }\npub fn main() { io.println(\"x\") }\n",
    ] {
        let err = check_source("t.elya", bad).expect_err("time is IO");
        assert!(format!("{err:?}").contains("E0421"), "{err:?}");
    }
    let wrong = "pub fn main() { time.sleep_ms(\"1\") }\n";
    let err = check_source("t.elya", wrong).expect_err("an Int, not a String");
    assert!(format!("{err:?}").contains("E0400"), "{err:?}");
}

#[test]
fn the_clock_advances_by_at_least_the_sleep() {
    let src = "pub fn main() {\n\
                 let a = time.now_ms()\n\
                 time.sleep_ms(20)\n\
                 let b = time.now_ms()\n\
                 time.sleep_ms(0 - 5)\n\
                 let c = time.now_ms()\n\
                 io.println(if a >= 0 && b - a >= 20 && c >= b { \"ok\" } else { \"wrong\" })\n\
               }\n";
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
    assert_eq!(run_source("t.elya", src).expect("runs"), "ok\n");
}

#[test]
fn time_builtins_used_as_values_are_refused() {
    for src in [
        "pub fn main() { let f = time.now_ms  io.println(\"x\") }\n",
        "fn apply(g, x) { g(x) }\npub fn main() { apply(time.sleep_ms, 3) }\n",
    ] {
        let err = check_source("t.elya", src).expect_err("a builtin used as a value");
        assert!(format!("{err:?}").contains("E0208"), "{err:?}");
    }
}

/// `examples/08_timers.elya`'s scheduler with a test `main` in place of the example's
/// (the review of 2026-10-16). Everything before `pub fn main` is the example, unchanged.
fn timers_example_with(main: &str) -> String {
    let src = include_str!("../examples/08_timers.elya");
    let cut = src.find("pub fn main").expect("the example has a main");
    format!("{}{main}", &src[..cut])
}

/// A timer whose deadline has passed fires even while other tasks can still run: the
/// scheduler polls its timers every turn, not only when every task is stuck. Under a
/// `ticking` clock (each reading is one ms later) a 5 ms sleeper must wake long before
/// a task that yields 100 times. Before the fix it woke only after the spinner was done.
#[test]
fn an_expired_timer_fires_while_other_tasks_run() {
    let src = timers_example_with(
        "fn ticking(body: fn() / {Clock, Log} -> Int) / {Log} -> Int {\n\
         \x20 let go = handle { body() } with {\n\
         \x20   Clock.now() -> fn(t: Int) { resume(t)(t + 1) }\n\
         \x20   Clock.wait_until(d) -> fn(t: Int) { if d > t { resume(Unit)(d) } else { resume(Unit)(t) } }\n\
         \x20   return(x) -> fn(t: Int) { x }\n\
         \x20 }\n\
         \x20 go(0)\n\
         }\n\
         fn spin(n: Int) / {Async, Log} -> Int {\n\
         \x20 if n == 0 { let _ = say(\"spinner done\")  0 } else { let _ = yld()  spin(n - 1) }\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle {\n\
         \x20   ticking(fn() { scope(fn() {\n\
         \x20     let a = spawn(fn() { let _ = sleep(5)  let _ = say(\"sleeper woke\")  1 })\n\
         \x20     let b = spawn(fn() { spin(100) })\n\
         \x20     await(a) + await(b)\n\
         \x20   }) })\n\
         \x20 } with {\n\
         \x20   Log.say(line) -> { io.println(line)  resume(Unit) }\n\
         \x20   return(v) -> v\n\
         \x20 }\n\
         }\n",
    );
    assert!(
        check_source("t.elya", &src).is_ok(),
        "{:?}",
        check_source("t.elya", &src)
    );
    assert_eq!(
        run_source("t.elya", &src).expect("runs"),
        "sleeper woke\nspinner done\n"
    );
}
