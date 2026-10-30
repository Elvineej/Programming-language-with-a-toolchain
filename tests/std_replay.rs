//! Record and replay as an Elya library (2026-10-24, spec
//! `docs/superpowers/specs/2026-10-24-elya-std-replay-design.md`): `import
//! elya/replay` is a standard module written in Elya, shipped in the compiler.
//!
//! Negative control (reverted; `std/replay.elya` compared byte for byte after):
//! - K1, `replay` answering without its question check (`if True`): the game
//!   with an eight-sided die "replays" instead of diverging, and
//!   `the_library_records_and_replays_two_programs` fails.
//! - K2 (2026-10-30, spec 2026-10-24-elya-trace-text), `parse`'s separator
//!   ignoring quotes: `a_parsed_trace_is_the_trace_it_was_written_from` and
//!   `an_unreadable_trace_names_its_line` fail (a ` = ` inside a string splits).

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

/// A trace with string answers that hold ` = `, quotes, a newline and a
/// two-byte character, written by `text` and read back by `parse` (spec
/// 2026-10-24-elya-trace-text D2, D3).
pub const PARSE_ROUND_TRIP: &str = r#"import elya/replay
effect Ask {
  fn ask(q: String) -> String
  fn n(k: Int) -> Int
}

fn prog() / {Ask} -> Int {
  let a = ask("x = y")
  let b = ask("p")
  string.length(a) * 100 + string.length(b) * 10 + n(3)
}

fn live(body: fn() / {Ask} -> Recorded(Int)) -> Recorded(Int) {
  handle { body() } with {
    Ask.ask(q) -> resume(if q == "p" { "plain" } else { "p = \"q\"\né" })
    Ask.n(k) -> resume(k * 2)
    return(x) -> x
  }
}

pub fn main() -> Int {
  match live(fn() { replay.record(fn() { prog() }) }) {
    Recorded(x, t) -> {
      io.println(replay.text(t))
      match replay.parse(replay.text(t)) {
        TraceParsed(u) -> {
          let same = if u == t { 1 } else { 0 }
          match replay.replay(fn() { prog() }, u) {
            Replayed(y) -> x * 10 + same + y * 100000
            Diverged(a, b) -> 0 - 1
            Unconsumed(y, r) -> 0 - 2
          }
        }
        TraceUnreadable(line, why) -> {
          io.println(why)
          0 - 3
        }
      }
    }
  }
}
"#;

#[test]
fn a_parsed_trace_is_the_trace_it_was_written_from() {
    let (out, v) = run(PARSE_ROUND_TRIP);
    // a is 10 bytes (`p = "q"`, a newline, two for `é`), b is 5: 1056; the
    // parse equals the recording (1) and replays to the same answer.
    assert_eq!(v, Ok(1056 * 100000 + 10561), "{out}");
    assert_eq!(
        out,
        "elya trace 1\n\
         Ask.ask(\"x = y\") = \"p = \\\"q\\\"\\n\u{e9}\"\n\
         Ask.ask(\"p\") = \"plain\"\n\
         Ask.n(3) = 6\n\n"
    );
    // The game and the shop round trip too.
    let src = GAME_AND_SHOP.replace(
        "report(\"the game\", replay.replay(fn() { game(3, 6, 0) }, log))",
        "match replay.parse(replay.text(log)) {\n\
         \x20 TraceParsed(u) -> report(if u == log { \"the parsed game\" } else { \"another game\" }, replay.replay(fn() { game(3, 6, 0) }, u))\n\
         \x20 TraceUnreadable(n, why) -> io.println(why)\n\
         }",
    );
    let (out, v) = run(&src);
    assert_eq!(v, Ok(2404), "{out}");
    assert!(out.contains("the parsed game: replayed, 24\n"), "{out}");
}

/// `parse` on `text` the program holds, answering `TraceUnreadable(line, why)`.
fn parsed(text: &str) -> String {
    let lit = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    let src = format!(
        "import elya/replay\n\
         fn show(t: Trace) -> String {{ match t {{ TraceEnd -> \"\"  TraceEntry(q, a, r) -> \"[\" <> q <> \"|\" <> a <> \"]\" <> show(r) }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 match replay.parse(\"{lit}\") {{\n\
         \x20   TraceParsed(t) -> {{ io.println(show(t))  0 }}\n\
         \x20   TraceUnreadable(n, why) -> {{ io.println(why)  n }}\n\
         \x20 }}\n\
         }}\n"
    );
    let (out, v) = run(&src);
    format!(
        "{}:{}",
        v.unwrap_or_else(|e| panic!("{text:?}: {e}")),
        out.trim_end()
    )
}

#[test]
fn an_unreadable_trace_names_its_line() {
    assert_eq!(
        parsed("elya trace 2\nA.a() = 1\n"),
        "1:the first line must be `elya trace 1`"
    );
    assert_eq!(parsed(""), "1:the first line must be `elya trace 1`");
    assert_eq!(
        parsed("elya trace 1\nA.a() = 1\nA.b()\n"),
        "3:expected `question = answer`"
    );
    assert_eq!(
        parsed("elya trace 1\n\nA.a(\"x = 1) = 2\n"),
        "3:a string is not closed"
    );
    // A separator inside quotes is not one; the first one outside is.
    assert_eq!(
        parsed("elya trace 1\nA.a(\"k = v\", \"\\\"\") = \"= \\\\\" = \"\n"),
        "0:[A.a(\"k = v\", \"\\\"\")|\"= \\\\\" = \"]"
    );
}

#[test]
fn the_hosts_lenient_trace_parses_the_same() {
    let clean = parsed("elya trace 1\nA.a(1) = 2\nA.b() = Unit\n");
    assert_eq!(clean, "0:[A.a(1)|2][A.b()|Unit]");
    for lenient in [
        "\u{feff}elya trace 1\nA.a(1) = 2\nA.b() = Unit\n",
        "elya trace 1\r\nA.a(1) = 2\r\nA.b() = Unit\r\n",
        "  elya trace 1  \n\n   \nA.a(1)   =   2\n\tA.b() = Unit\t\n\n",
        "elya trace 1\nA.a(1) = 2\nA.b() = Unit",
        // `str::trim`'s whitespace, beyond ASCII (the host's rule, kept).
        "elya trace 1\u{a0}\n\u{3000}\nA.a(1) = 2\u{2003}\n\u{85}A.b() = Unit\n",
    ] {
        assert_eq!(parsed(lenient), clean, "{lenient:?}");
    }
}

/// The review's F1 (2026-10-30): `parse` and the host reader (`read_trace`) on
/// every short trace over a small alphabet: the same entries, or the same line
/// and the same words. The first version of `parse` split the TRIMMED line, so
/// a ` = ` touching a trimmed edge (`A.a() = ` then the end) read differently.
#[test]
fn parse_reads_every_short_trace_as_the_host_does() {
    // Every line of up to three symbols, under the header, two lines each.
    let alphabet = [" ", "=", "\"", "\\", "a", "\u{3000}"];
    let mut lines: Vec<String> = vec![String::new()];
    for _ in 0..3 {
        let longer: Vec<String> = lines
            .iter()
            .flat_map(|l| alphabet.iter().map(move |c| format!("{l}{c}")))
            .collect();
        lines.extend(longer);
    }
    lines.sort();
    lines.dedup();
    let mut texts: Vec<String> = Vec::new();
    for l in &lines {
        texts.push(format!("elya trace 1\n{l}\n"));
        texts.push(format!("elya trace 1\nA.a() = 1\n{l}"));
        texts.push(format!("elya trace 1\r\n{l} = {l}\r\n"));
    }
    texts.push(String::new());
    texts.push("elya trace 1".to_string());
    texts.push("\u{feff} elya trace 1 \n".to_string());
    // One program parses them all and prints one line per text.
    let lit = |t: &str| {
        t.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    };
    let mut calls = String::new();
    for t in &texts {
        calls.push_str(&format!("  io.println(read(\"{}\"))\n", lit(t)));
    }
    let src = format!(
        "import elya/replay\n\
         fn show(t: Trace) -> String {{ match t {{ TraceEnd -> \"\"  TraceEntry(q, a, r) -> \"[\" <> q <> \"|\" <> a <> \"]\" <> show(r) }} }}\n\
         fn read(text: String) -> String {{\n\
         \x20 match replay.parse(text) {{\n\
         \x20   TraceParsed(t) -> \"ok \" <> show(t)\n\
         \x20   TraceUnreadable(n, why) -> int.to_string(n) <> \" \" <> why\n\
         \x20 }}\n\
         }}\n\
         pub fn main() -> Int {{\n{calls}  0\n}}\n"
    );
    let (out, v) = run(&src);
    assert_eq!(v, Ok(0), "{out}");
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(got.len(), texts.len());
    let mut differ = Vec::new();
    for (t, g) in texts.iter().zip(got) {
        let want = match elya::read_trace(t) {
            Ok(entries) => {
                let shown: String = entries.iter().map(|(q, a)| format!("[{q}|{a}]")).collect();
                format!("ok {shown}")
            }
            Err((n, why)) => format!("{n} {why}"),
        };
        if want != g {
            differ.push(format!("{t:?}: host `{want}`, parse `{g}`"));
        }
    }
    assert!(
        differ.is_empty(),
        "{} differ:\n{}",
        differ.len(),
        differ.join("\n")
    );
}
