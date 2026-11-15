//! The parser's output as text (self-hosting part 4, spec
//! `docs/superpowers/specs/2026-11-15-elya-parser-in-elya-design.md` D2):
//! the form the parser in Elya must print byte for byte. Pinned on a program
//! that uses every kind of node, so a change to the form is seen here first.
//!
//! Negative control (reverted, compared byte for byte): K1, `Param` printing
//! no `borrow` flag: `the_dump_of_every_node_is_pinned` fails.

use elya::ast_dump::{dump, dump_diagnostics};
use elya::parse::parse_module;
use elya::Session;

const EVERY: &str = r#"import elya/map
import lib/dep as d / {Console, State(Int)}

linear type File(a) { File(Int, a), Closed }
effect multi Choice(t) { fn pick(xs: List(t), n: Int) -> t }

pub fn total(borrow xs: Array(Int), i: Int) / {Choice(Int) | e} -> Int {
  let f: fn(Int) / {e} -> Int = fn(x: Int) { x + 1 }
  let s = "a\tb\"c\\"
  if i >= array.length(xs) && !False { 1.5e3 <. 2.0 } else { -i * (3 - 4) % 5 }
  match d.f(i) {
    Some(File(n, _)) -> n
    "x" -> 0
    _ -> {
      handle { pick(Nil, 1) } with multi {
        Choice.pick(a, b) -> resume(b)
        return(v) -> v
      }
    }
  }
}
"#;

const EVERY_DUMP: &str = r#"0 Module - -
1 Import 0 15 elya/map - -1
1 Import 16 53 lib/dep d 2
2 Allow 39 46 Console
2 Allow 48 53 State
1 Type 61 105 linear File a
2 Variant 83 95 File
3 Ty 88 91 Int
3 Ty 93 94 a
2 Variant 97 103 Closed
1 Effect 106 166 multi Choice t
2 Op 131 164 pick 2
3 Param 139 141 - xs
3 Param 152 153 - n
3 Ty 143 147 List
4 Ty 148 149 t
3 Ty 155 158 Int
3 Ty 163 164 t
1 Fn 172 587 pub total
2 Param 188 190 borrow xs
3 Ty 192 197 Array
4 Ty 198 201 Int
2 Param 204 205 - i
3 Ty 207 210 Int
2 Row - - e
3 Label 215 221 Choice 1
4 Ty 222 225 Int
2 Ty 235 238 Int
2 Block 239 587
3 Let 243 293 f
4 Ty 250 270 fn
5 Ty 253 256 Int
5 Ty 267 270 Int
5 Row - - e
4 Lambda 273 293 1
5 Param 276 277 - x
6 Ty 279 282 Int
5 Block 284 293
6 Tail - -
7 Binary 286 291 +
8 Var 286 287 x
8 Int 290 291 1
3 Let 296 315 s
4 NoType - -
4 Str 304 315 "a\tb\"c\\"
3 Do 318 395
4 If 318 395
5 Binary 321 352 &&
6 Binary 321 342 >=
7 Var 321 322 i
7 Call 326 342 1
8 Qualified 326 338 array length
8 Var 339 341 xs
6 Unary 346 352 !
7 Bool 347 352 False
5 Block 353 369
6 Tail - -
7 Binary 355 367 <.
8 Float 355 360 1.5e3
8 Float 364 367 2.0
5 Block 375 395
6 Tail - -
7 Binary 377 393 %
8 Binary 377 388 *
9 Unary 377 379 -
10 Var 378 379 i
9 Binary 383 388 -
10 Int 383 384 3
10 Int 387 388 4
8 Int 392 393 5
3 Tail - -
4 Match 398 585 3
5 Call 404 410 1
6 Qualified 404 407 d f
6 Var 408 409 i
5 Arm 417 438
6 PCtor 417 433 Some 1
7 PCtor 422 432 File 2
8 PVar 427 428 n
8 PWild 430 431
6 Var 437 438 n
5 Arm 443 451
6 PStr 443 446 "x"
6 Int 450 451 0
5 Arm 456 581
6 PWild 456 457
6 Block 461 581
7 Tail - -
8 Handle 469 575 multi -
9 Block 476 492
10 Tail - -
11 Call 478 490 2
12 Var 478 482 pick
12 Var 483 486 Nil
12 Int 488 489 1
9 Clause 514 544 Choice pick
10 Param 526 527 - a
10 Param 529 530 - b
10 Resume 535 544
11 Var 542 543 b
9 Return - - v
10 Var 566 567 v
"#;

#[test]
fn the_dump_of_every_node_is_pinned() {
    let (m, diags) = parse_module(&Session::new(), EVERY);
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(dump(&m, EVERY), EVERY_DUMP);
}

#[test]
fn a_parse_error_is_dumped_with_its_code_span_and_words() {
    let src = "pub fn main() -> Int { let x = 1 +\n}\nfn f( { 1 }\n";
    let (m, diags) = parse_module(&Session::new(), src);
    assert_eq!(
        dump_diagnostics(&diags),
        "error E0100 35 36 \"expected an expression\"\nerror E0100 43 44 \"expected parameter name\"\n"
    );
    // What recovery kept is dumped too: the binding's value is an error node.
    assert!(
        dump(&m, src).contains("4 Error 23 34\n"),
        "{}",
        dump(&m, src)
    );
}
