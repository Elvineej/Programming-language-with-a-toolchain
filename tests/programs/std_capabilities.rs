//! The programs of `tests/std_capabilities.rs`, shared with
//! `crates/codegen/tests/native_std_capabilities.rs` (no tests here).

/// A dependency that writes a line and reads the clock: granted `Console` and
/// `WallClock`, never `IO`.
pub const DEP: &str = "import elya/console / {Console}
import elya/clock / {WallClock}
pub fn report(n: Int) -> Int {
  let t = wall_clock_ms()
  console_line(\"report \" <> int.to_string(n) <> \" at \" <> int.to_string(t))
  n * 2
}
";

pub const GIVEN_LESS: &str = "import elya/console / {Console}
import elya/clock / {WallClock}
import elya/host / {IO}
import dep / {Console, WallClock}
pub fn main() -> Int {
  host.console(fn() {
    console.prefixed(\"[dep] \", fn() {
      clock.fixed(1000, fn() { dep.report(1) + dep.report(2) })
    })
  })
}
";

pub const ATTENUATED: &str = "import elya/console / {Console}
import elya/clock / {WallClock}
import elya/host / {IO}
import dep / {Console, WallClock}
fn three() -> Int { dep.report(1) + dep.report(2) + dep.report(3) }
pub fn main() -> Int {
  let a = host.console(fn() { console.limited(2, fn() { clock.stepping(10, 5, fn() { three() }) }) })
  let b = host.console(fn() {
    match console.counted(fn() { clock.fixed(0, fn() { three() }) }) { ConsoleCounted(x, n) -> x * 100 + n }
  })
  let c = match console.captured(fn() { clock.fixed(7, fn() { dep.report(5) + dep.report(6) }) }) {
    ConsoleCaptured(x, text) -> { host.console(fn() { console_line(\"captured:\\n\" <> text) })  x }
  }
  let d = console.silenced(fn() { clock.fixed(7, fn() { dep.report(9) }) })
  let e = host.console(fn() {
    clock.fixed(50, fn() { clock.shifted(25, fn() { dep.report(4) }) })
  })
  a * 100000 + b * 100 + c + d + e
}
";
