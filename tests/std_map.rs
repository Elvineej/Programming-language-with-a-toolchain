//! `elya/map` (2026-11-15, spec
//! `docs/superpowers/specs/2026-11-14-elya-arrays-and-borrowing-design.md`
//! D7): a hash map from strings, written in Elya over the builtin arrays. The
//! map is one `Array(MapSlot(a))` (a borrowed value cannot be taken apart, so
//! no type wraps it): slot 0 holds the count, the rest a table probed
//! linearly, doubled past three quarters full. ChatGPT wrote the module and
//! these programs to Claude's brief; checked here.
//!
//! Negative controls (`src/std/map.elya`, each reverted and compared byte for
//! byte):
//! - K1, the probe not wrapping at the table's end (`next_index` answering
//!   `index + 1`): `collisions_wrap_and_a_replace_keeps_the_count` fails.
//! - K2, growth never happening: `the_seventh_entry_grows_the_table` fails.
//! - K3, a replace counted as an insert:
//!   `collisions_wrap_and_a_replace_keeps_the_count` fails.

#[allow(dead_code)] // each suite uses some of the programs
#[path = "programs/map.rs"]
mod programs;
use programs::*;

fn run(src: &str) -> elya::RunOutcome {
    elya::run_source_outcome_with("main.elya", src, &mut |_| None)
}

fn errors(src: &str) -> String {
    match elya::check_source_with("main.elya", src, &mut |_| None) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

fn value(src: &str) -> String {
    let o = run(src);
    assert!(o.error.is_none(), "{:?}\n{src}", o.error);
    o.line.expect("a line").expect("data")
}

#[test]
fn the_whole_map_works() {
    let o = run(MAP_PROGRAM);
    assert!(o.error.is_none(), "{:?}", o.error);
    assert_eq!(o.output, MAP_OUTPUT);
    assert_eq!(o.line, Some(Ok(MAP_VALUE.to_string())));
}

#[test]
fn collisions_wrap_and_a_replace_keeps_the_count() {
    // `h` and `p` both start at the last slot of eight: `p` wraps to the first.
    assert_eq!(
        value("import elya/map\npub fn main() -> Int {\n  let m = map.put(map.put(map.new(), \"h\", 1), \"p\", 2)\n  map.get_or(m, \"h\", 0) * 10 + map.get_or(m, \"p\", 0)\n}\n"),
        "12"
    );
    assert_eq!(
        value("import elya/map\npub fn main() -> Int {\n  let m = map.put(map.put(map.new(), \"same\", 1), \"same\", 2)\n  map.size(m) * 100 + map.get_or(m, \"same\", 0)\n}\n"),
        "102"
    );
}

#[test]
fn the_seventh_entry_grows_the_table() {
    // Seven entries pass three quarters of eight: 16 slots and the count's.
    assert_eq!(
        value("import elya/map\nfn add(m: Array(MapSlot(Int)), i: Int) -> Array(MapSlot(Int)) {\n  if i == 7 { m } else { add(map.put(m, int.to_string(i), i), i + 1) }\n}\npub fn main() -> Int {\n  let m: Array(MapSlot(Int)) = add(map.new(), 0)\n  array.length(m) * 100 + map.size(m)\n}\n"),
        "1707"
    );
}

#[test]
fn a_map_holds_no_affine_value() {
    let e = errors("import elya/map\npub fn main() -> Int {\n  let m = map.put(map.new(), \"bad\", buffer.new())\n  map.size(m)\n}\n");
    assert!(e.contains("E0437"), "{e}");
    assert!(e.contains("Buffer"), "{e}");
}

#[test]
fn inserts_and_lookups_are_linear_in_the_evaluator() {
    // Eight times the entries: linear is eight times the time.
    let time = |n: u32| {
        let t = std::time::Instant::now();
        assert_eq!(value(&inserts_and_lookups(n)), sum_below(n.into()));
        t.elapsed().as_secs_f64()
    };
    let small = time(2_500).max(1e-3);
    let large = time(20_000);
    assert!(
        large / small < 14.0,
        "20,000 took {large:.3} s, 2,500 took {small:.3} s"
    );
}
