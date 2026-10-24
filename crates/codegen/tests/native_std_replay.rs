//! `import elya/replay` natively (2026-10-24, spec
//! `docs/superpowers/specs/2026-10-24-elya-std-replay-design.md`): the standard
//! module is merged, compiled, linked and RUN, and its output compared with the
//! evaluator's. The program is `tests/std_replay.rs`'s `GAME_AND_SHOP`.

use std::process::Command;

const GAME_AND_SHOP: &str = r#"import elya/replay
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

#[test]
fn the_library_runs_natively() {
    let mut read = |_: &str| None;
    let o = elya::run_source_outcome_with("main.elya", GAME_AND_SHOP, &mut read);
    assert!(o.error.is_none(), "{:?}", o.error);
    let want = format!("{}{}\n", o.output, o.value.unwrap().unwrap());
    let (module, diags, _) = elya::front_end_with("main.elya", GAME_AND_SHOP, &mut read);
    assert!(diags.is_empty(), "{diags:?}");
    let session = elya::Session::new();
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    assert!(diags.is_empty(), "{diags:?}");
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let dir = std::env::temp_dir().join(format!("elya-std-replay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let obj = dir.join("r.o");
    let exe = dir.join(format!("r{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).unwrap_or_else(|e| panic!("{e}"));
    elya_codegen::link(&obj, &exe).expect("link");
    let out = Command::new(&exe).output().expect("run the binary");
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), want);
    assert!(want.ends_with("the shop: replayed, 4\n2404\n"), "{want}");
}
