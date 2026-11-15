//! The checker's verdict as text (`elya::check_dump`, `elya check --dump`):
//! the form a checker written in Elya will be compared against byte for byte
//! (self-hosting, part 4's checker half). Pinned on a program with generic,
//! effectful, higher-order and array code, so a change to the form is seen.
//!
//! Negative control (reverted, compared byte for byte): K1, one `TyPrinter`
//! shared across lines instead of fresh names per line:
//! `the_verdict_on_a_program_is_pinned` fails.

use elya::check_dump::dump;

const PROGRAM: &str = r#"type Option(a) { None, Some(a) }
effect Ask { fn ask() -> Int }
fn id(x: a) -> a { x }
fn twice(f: fn(Int) -> Int, x: Int) -> Int { f(f(x)) }
fn get(o: Option(Int)) -> Int { match o { Some(n) -> n  None -> ask() } }
pub fn main() -> Int {
  let g = fn(n: Int) { n + 1 }
  let xs = array.push(array.new(), id(3))
  handle { twice(g, get(Some(array.get(xs, 0)))) } with { Ask.ask() -> resume(0) }
}
"#;

const VERDICT: &str = r#"scheme get fn(Option(Int)) / {Ask} -> Int
scheme id forall a. fn(a) -> a
scheme main fn() -> Int
scheme twice forall a. fn(fn(Int) / {a} -> Int, Int) / {a} -> Int
type 64 86 fn(a) -> a
type 70 71 a
type 83 84 a
type 87 141 fn(fn(Int) / {a} -> Int, Int) / {a} -> Int
type 96 97 fn(Int) / {a} -> Int
type 115 116 Int
type 132 133 fn(Int) / {a} -> Int
type 132 139 Int
type 134 135 fn(Int) / {a} -> Int
type 134 138 Int
type 136 137 Int
type 142 215 fn(Option(Int)) / {Ask} -> Int
type 149 150 Option(Int)
type 174 213 Int
type 180 181 Option(Int)
type 195 196 Int
type 206 209 fn() / {Ask} -> Int
type 206 211 Int
type 220 396 fn() -> Int
type 249 269 fn(Int) -> Int
type 252 253 Int
type 262 263 Int
type 262 267 Int
type 266 267 Int
type 281 291 fn(Array(Int), Int) -> Array(Int)
type 281 311 Array(Int)
type 292 301 fn() -> Array(Int)
type 292 303 Array(Int)
type 305 307 fn(Int) -> Int
type 305 310 Int
type 308 309 Int
type 314 394 Int
type 321 362 Int
type 323 328 fn(fn(Int) / {Ask} -> Int, Int) / {Ask} -> Int
type 323 360 Int
type 329 330 fn(Int) / {Ask} -> Int
type 332 335 fn(Option(Int)) / {Ask} -> Int
type 332 359 Int
type 336 358 Option(Int)
type 341 350 fn(Array(Int), Int) -> Int
type 341 357 Int
type 351 353 Array(Int)
type 355 356 Int
type 383 392 Int
type 390 391 Int
"#;

fn module(src: &str) -> elya::ast::Module {
    let (m, diags, _) = elya::front_end_with("main.elya", src, &mut |_| None);
    assert!(diags.iter().all(|d| d.code.starts_with("E04")), "{diags:?}");
    m
}

#[test]
fn the_verdict_on_a_program_is_pinned() {
    assert_eq!(dump(&module(PROGRAM)), VERDICT);
}

#[test]
fn a_type_error_is_in_the_verdict() {
    let v = dump(&module("pub fn main() -> Int { \"a\" + 1 }\n"));
    assert_eq!(
        v,
        "scheme main fn() -> Int\ntype 4 32 fn() -> Int\ntype 23 26 String\ntype 23 30 Int\ntype 29 30 Int\nerror E0400 23 30 \"type mismatch\"\n"
    );
}
