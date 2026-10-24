//! Record and replay as an Elya library (2026-10-24, spec
//! `docs/superpowers/specs/2026-10-24-elya-std-replay-design.md`): `import
//! elya/replay` is a standard module written in Elya, shipped in the compiler.
//!
//! Negative control (reverted; `std/replay.elya` compared byte for byte after):
//! - K1, `replay` answering without its question check (`if True`): the game
//!   with an eight-sided die "replays" instead of diverging, and
//!   `the_library_records_and_replays_two_programs` fails.

/// The dice game and the shop of `examples/09_record_any.elya`, through the
/// library instead of its own recorder.
pub const GAME_AND_SHOP: &str = r#"import elya/replay
effect World {
  fn roll(sides: Int) -> Int
  fn now() -> Int
}

// Score a roll double if the round was quick.
fn game(rounds: Int, sides: Int, last: Int) / {World} -> Int {
  if rounds == 0 { 0 } else {
    let r = roll(sides)
    let t = now()
    let s = if t - last < 50 { 2 * r } else { r }
    s + game(rounds - 1, sides, t)
  }
}

// A seeded generator stands in for real dice and a real clock.
fn next(s: Int) -> Int { (s * 1103515245 + 12345) % 2147483648 }

fn live_world(body: fn() / {World} -> Recorded(Int), seed: Int) -> Recorded(Int) {
  let run = handle { body() } with {
    World.roll(sides) -> fn(s: Int) { resume(next(s) % sides + 1)(next(s)) }
    World.now() -> fn(s: Int) { resume(s % 100)(next(s)) }
    return(x) -> fn(s: Int) { x }
  }
  run(seed)
}

type Stock { Out, Left(Int) }

effect Store {
  fn stock(item: String) -> Stock
  fn sell(item: String, n: Int) -> Unit
}

// Sell what is asked for, as far as the stock goes; answer the units sold.
fn shop() / {Store} -> Int {
  let a = buy("tea", 3)
  let b = buy("cake", 2)
  a + b
}

fn buy(item: String, n: Int) / {Store} -> Int {
  match stock(item) {
    Out -> 0
    Left(k) -> {
      let m = if k < n { k } else { n }
      let _ = sell(item, m)
      m
    }
  }
}

fn live_store(body: fn() / {Store} -> Recorded(Int)) -> Recorded(Int) {
  handle { body() } with {
    Store.stock(item) -> resume(if item == "tea" { Left(10) } else { Left(1) })
    Store.sell(item, n) -> resume(Unit)
    return(x) -> x
  }
}


fn report(what: String, o: Replayed(Int)) {
  match o {
    Replayed(x) -> io.println(what <> ": replayed, " <> int.to_string(x))
    Diverged(asked, held) -> io.println(what <> ": diverged, asked " <> asked <> " where the recording holds " <> held)
    Unconsumed(x, rest) -> io.println(what <> ": finished with " <> int.to_string(replay.size(rest)) <> " entries left over")
  }
}

pub fn main() -> Int {
  let g = match live_world(fn() { replay.record(fn() { game(3, 6, 0) }) }, 42) {
    Recorded(score, log) -> {
      io.println(replay.text(log))
      report("the game", replay.replay(fn() { game(3, 6, 0) }, log))
      report("the game with an eight-sided die", replay.replay(fn() { game(3, 8, 0) }, log))
      report("two rounds of the game", replay.replay(fn() { game(2, 6, 0) }, log))
      report("four rounds of the game", replay.replay(fn() { game(4, 6, 0) }, log))
      score
    }
  }
  let s = match live_store(fn() { replay.record(fn() { shop() }) }) {
    Recorded(sold, log) -> {
      io.println(replay.text(log))
      report("the shop", replay.replay(fn() { shop() }, log))
      sold
    }
  }
  g * 100 + s
}
"#;

fn run(src: &str) -> (String, Result<i64, String>) {
    let o = elya::run_source_outcome("t.elya", src);
    if let Some(e) = o.error {
        return (o.output, Err(e));
    }
    (o.output, Ok(o.value.expect("a value").expect("an Int")))
}

fn errors(src: &str) -> String {
    match elya::check_source("t.elya", src) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

#[test]
fn the_library_records_and_replays_two_programs() {
    let (out, v) = run(GAME_AND_SHOP);
    assert_eq!(v, Ok(2404), "{out}");
    assert_eq!(
        out,
        "elya trace 1\n\
         World.roll(6) = 4\nWorld.now() = 27\nWorld.roll(6) = 6\n\
         World.now() = 53\nWorld.roll(6) = 2\nWorld.now() = 35\n\n\
         the game: replayed, 24\n\
         the game with an eight-sided die: diverged, asked World.roll(8) where the recording holds World.roll(6)\n\
         two rounds of the game: finished with 2 entries left over\n\
         four rounds of the game: diverged, asked World.roll(6) where the recording holds the end of the trace\n\
         elya trace 1\n\
         Store.stock(\"tea\") = Left(10)\nStore.sell(\"tea\", 3) = Unit\n\
         Store.stock(\"cake\") = Left(1)\nStore.sell(\"cake\", 1) = Unit\n\n\
         the shop: replayed, 4\n"
    );
}

#[test]
fn a_replayed_answer_keeps_its_type() {
    // A recorded `Bool` answer replayed into the same question: the program
    // gets `True` back, with no handler of `Ask` at all.
    let src = "import elya/replay\n\
               effect Ask { fn ok(n: Int) -> Bool }\n\
               fn prog() / {Ask} -> Int { if ok(1) { 10 } else { 20 } }\n\
               pub fn main() -> Int {\n\
                 let r = handle { replay.record(fn() { prog() }) } with { Ask.ok(n) -> resume(n == 1) }\n\
                 match r { Recorded(x, t) -> match replay.replay(fn() { prog() }, t) {\n\
                   Replayed(y) -> x + y * 100 + replay.size(t) * 10000\n\
                   Diverged(a, b) -> 0  Unconsumed(y, rest) -> 1 } }\n\
               }\n";
    assert_eq!(run(src).1, Ok(11010));
}

#[test]
fn the_library_needs_no_grant() {
    // Its interface performs nothing of its own: no list, and no `IO`.
    let src = "import elya/replay\npub fn main() -> Int { match replay.record(fn() { 7 }) { Recorded(x, t) -> x + replay.size(t) } }\n";
    assert_eq!(errors(src), "");
    assert_eq!(run(src).1, Ok(7));
    // A list on it is checked like any module's (it may grant more than used).
    let src = "import elya/replay / {IO}\npub fn main() -> Int { match replay.record(fn() { 7 }) { Recorded(x, t) -> x } }\n";
    assert_eq!(errors(src), "");
}

#[test]
fn its_names_are_program_unique() {
    let src = "import elya/replay\ntype Trace { Mine }\npub fn main() -> Int { 1 }\n";
    let e = errors(src);
    assert!(e.contains("E0209"), "{e}");
    // Without the import nothing clashes.
    assert_eq!(
        errors("type Trace { Mine }\npub fn main() -> Int { 1 }\n"),
        ""
    );
}
