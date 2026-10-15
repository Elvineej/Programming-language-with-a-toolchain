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
