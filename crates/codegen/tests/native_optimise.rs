//! Optimised code (spec 2026-11-13-elya-optimised-code): `compile_module` runs
//! LLVM's `default<O2>`; `Optimise::None` is the IR as written. Both write what
//! the program means, plainly and under `ELYA_GC_STRESS=1`; the optimised
//! object is the smaller.

use elya_codegen::Optimise;
use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-opt-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn lower(src: &str) -> elya::core::CoreModule {
    let (module, diags, _) = elya::front_end_with("opt.elya", src, &mut |_| None);
    assert!(diags.is_empty(), "{diags:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diags.is_empty(), "{diags:?}");
    elya::core::lower_module(&module, &table).expect("lowers")
}

/// (the object's bytes, then (status, stdout, stderr) plainly and under stress)
fn build_and_run(tag: &str, src: &str, optimise: Optimise) -> (u64, [(i32, String, String); 2]) {
    let dir = temp_dir(&format!("{tag}-{optimise:?}"));
    let obj = dir.join("program.o");
    elya_codegen::compile_module_with(&lower(src), &obj, optimise).expect("compile");
    let size = std::fs::metadata(&obj).expect("object").len();
    let exe = dir.join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    elya_codegen::link(&obj, &exe).expect("link");
    let run = |stress: bool| {
        let mut cmd = Command::new(&exe);
        if stress {
            cmd.env("ELYA_GC_STRESS", "1");
        }
        let out = cmd.output().expect("run");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let runs = [run(false), run(true)];
    std::fs::remove_dir_all(&dir).ok();
    (size, runs)
}

/// ChatGPT's eight benchmarks (written to Claude's brief, each checked against
/// its C twin's result at full size), here at sizes a test can afford: naive
/// recursion, a counting loop, n-queens on lists, trees, a merge sort, trial
/// division, a list of closures, a state handler.
const PROGRAMS: &[(&str, &str, &str)] = &[
    (
        "fib",
        "fn fib(n: Int) -> Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\n\
         pub fn main() -> Int { fib(22) }\n",
        "17711\n",
    ),
    (
        "sum",
        "fn remainder(x: Int, d: Int) -> Int { x - (x / d) * d }\n\
         fn go(i: Int, acc: Int) -> Int { if i == 300000 { acc } else { go(i + 1, acc + remainder(i * i, 7)) } }\n\
         pub fn main() -> Int { go(0, 0) }\n",
        "599998\n",
    ),
    (
        "queens",
        "type L { Nil, Cons(Int, L) }\n\
         fn safe(col: Int, xs: L, dist: Int) -> Bool { match xs { Nil -> True  Cons(h, t) -> if col == h || col - h == dist || h - col == dist { False } else { safe(col, t, dist + 1) } } }\n\
         fn positions(row: Int, col: Int, xs: L, acc: Int) -> Int { if col == 7 { acc } else { let a = if safe(col, xs, 1) { solve(row + 1, Cons(col, xs)) } else { 0 }  positions(row, col + 1, xs, acc + a) } }\n\
         fn solve(row: Int, xs: L) -> Int { if row == 7 { 1 } else { positions(row, 0, xs, 0) } }\n\
         pub fn main() -> Int { solve(0, Nil) }\n",
        "40\n",
    ),
    (
        "trees",
        "type Tree { Leaf, Branch(Tree, Tree) }\n\
         fn build(depth: Int) -> Tree { if depth == 0 { Leaf } else { Branch(build(depth - 1), build(depth - 1)) } }\n\
         fn count(t: Tree) -> Int { match t { Leaf -> 1  Branch(a, b) -> 1 + count(a) + count(b) } }\n\
         fn rounds(n: Int, acc: Int) -> Int { if n == 0 { acc } else { let t = build(9)  rounds(n - 1, acc + count(t)) } }\n\
         pub fn main() -> Int { rounds(3, 0) }\n",
        "3069\n",
    ),
    (
        "sort",
        "type L { Nil, Cons(Int, L) }\n\
         fn remainder(x: Int, d: Int) -> Int { x - (x / d) * d }\n\
         fn build(i: Int, seed: Int, acc: L) -> L { if i == 2000 { acc } else { let next = remainder(seed * 48271, 2147483647)  build(i + 1, next, Cons(next, acc)) } }\n\
         fn reverse(xs: L, acc: L) -> L { match xs { Nil -> acc  Cons(h, t) -> reverse(t, Cons(h, acc)) } }\n\
         fn take(n: Int, xs: L, acc: L) -> L { if n == 0 { reverse(acc, Nil) } else { match xs { Nil -> reverse(acc, Nil)  Cons(h, t) -> take(n - 1, t, Cons(h, acc)) } } }\n\
         fn drop(n: Int, xs: L) -> L { if n == 0 { xs } else { match xs { Nil -> Nil  Cons(_, t) -> drop(n - 1, t) } } }\n\
         fn merge(a: L, b: L, acc: L) -> L { match a { Nil -> reverse(acc, b)  Cons(x, at) -> match b { Nil -> reverse(acc, a)  Cons(y, bt) -> if x <= y { merge(at, b, Cons(x, acc)) } else { merge(a, bt, Cons(y, acc)) } } } }\n\
         fn sort(xs: L, n: Int) -> L { if n <= 1 { xs } else { let half = n / 2  let a = sort(take(half, xs, Nil), half)  let b = sort(drop(half, xs), n - half)  merge(a, b, Nil) } }\n\
         fn sorted(xs: L, last: Int, n: Int) -> Int { match xs { Nil -> n  Cons(h, t) -> if last <= h { sorted(t, h, n + 1) } else { 0 - 1 } } }\n\
         pub fn main() -> Int { sorted(sort(build(0, 1, Nil), 2000), 0, 0) }\n",
        "2000\n",
    ),
    (
        "primes",
        "fn remainder(x: Int, d: Int) -> Int { x - (x / d) * d }\n\
         fn prime(n: Int, d: Int) -> Bool { if d > n / d { True } else { if remainder(n, d) == 0 { False } else { prime(n, d + 2) } } }\n\
         fn go(n: Int, acc: Int) -> Int { if n >= 20000 { acc } else { go(n + 2, acc + (if prime(n, 3) { 1 } else { 0 })) } }\n\
         pub fn main() -> Int { 1 + go(3, 0) }\n",
        "2262\n",
    ),
    (
        "closures",
        "type Fs { Nil, Cons(fn(Int) -> Int, Fs) }\n\
         fn build(k: Int, acc: Fs) -> Fs { if k == 100 { acc } else { build(k + 1, Cons(fn(x) { x + k }, acc)) } }\n\
         fn apply(fs: Fs, x: Int) -> Int { match fs { Nil -> x  Cons(f, t) -> apply(t, f(x)) } }\n\
         fn many(fs: Fs, i: Int, acc: Int) -> Int { if i == 100 { acc } else { many(fs, i + 1, acc + apply(fs, i)) } }\n\
         pub fn main() -> Int { many(build(0, Nil), 0, 0) }\n",
        "499950\n",
    ),
    (
        "effects",
        "effect State { fn get() -> Int  fn put(x: Int) -> Int }\n\
         fn tick(n: Int) -> Int { if n == 0 { get() } else { let x = get()  let _ = put(x + 1)  tick(n - 1) } }\n\
         pub fn main() -> Int {\n  let state = handle { tick(2000) } with {\n    State.get() -> fn(s) { (resume(s))(s) }\n    State.put(v) -> fn(s) { (resume(0))(v) }\n    return(x) -> fn(s) { x }\n  }\n  state(0)\n}\n",
        "2000\n",
    ),
    (
        "failure",
        "fn zero(n: Int) -> Int { n - n }\n\
         pub fn main() -> Int { io.println(\"before\")  9223372036854775807 + (1 / (1 + zero(5))) }\n",
        "",
    ),
];

#[test]
fn optimised_and_unoptimised_code_write_the_same() {
    for (tag, src, want) in PROGRAMS {
        let (_, fast) = build_and_run(tag, src, Optimise::Speed);
        let (_, plain) = build_and_run(tag, src, Optimise::None);
        if want.is_empty() {
            // The overflow is the program's meaning: not folded away, not moved
            // before the line already written.
            assert_eq!(
                plain[0],
                (
                    1,
                    "before\n".to_string(),
                    "elya: integer overflow\n".to_string()
                ),
                "{tag}"
            );
        } else {
            assert_eq!(
                plain[0],
                (0, want.to_string(), String::new()),
                "{tag}: unoptimised"
            );
        }
        assert_eq!(
            fast, plain,
            "{tag}: optimised against unoptimised, plain and under stress"
        );
    }
}

/// The passes ran: in the counting loop and the trial division, the call to
/// `remainder` is inlined and the tail call to the function itself is a loop,
/// as written they are calls. Read from the IR the object is made from, which
/// is the same on every host (the object's size was the measure, and Windows'
/// unwind tables made `sum` the larger optimised: the loop is inlined into
/// `main` and `elya_entry` too).
#[test]
fn the_passes_inline_and_loop() {
    for (tag, src, _) in PROGRAMS
        .iter()
        .filter(|(tag, _, _)| ["sum", "primes"].contains(tag))
    {
        let looping = if *tag == "sum" { "go" } else { "prime" };
        let core = lower(src);
        let body = |o: Optimise| {
            let ir =
                elya_codegen::emit_ir_for(&core, o, &elya_codegen::Machine::host()).expect("emit");
            let head = format!("@elya.{looping}(");
            let start = ir
                .lines()
                .position(|l| l.starts_with("define") && l.contains(&head))
                .unwrap_or_else(|| panic!("{tag}: no {head} in {ir}"));
            let lines: Vec<&str> = ir
                .lines()
                .skip(start + 1)
                .take_while(|l| *l != "}")
                .collect();
            lines.join("\n")
        };
        let (fast, plain) = (body(Optimise::Speed), body(Optimise::None));
        let calls = |b: &str, f: &str| b.contains(&format!("@elya.{f}("));
        assert!(
            calls(&plain, "remainder") && calls(&plain, looping),
            "{tag}: {plain}"
        );
        assert!(
            !calls(&fast, "remainder") && !calls(&fast, looping),
            "{tag}: {fast}"
        );
    }
}

/// A function too long for a person to have written is left as written, and
/// building stays linear: LLVM's instruction combiner took 117 s over one
/// function of a 5,000-`let` block (found by `native_long_blocks`, which went
/// from half a minute to more than fifteen). Here 6,000 `let`s of the shape
/// that was slow (a product, a sum and a remainder each) build in a time the
/// combiner alone would exceed many times over, and compute what they mean.
#[test]
fn a_very_long_function_is_built_as_written() {
    let n = 6_000;
    let mut src = String::from("fn f(x: Int) -> Int {\n");
    let mut prev = "x".to_string();
    for i in 0..n {
        src.push_str(&format!(
            "  let v{i} = ({prev} * 3 + {}) % 1000003\n",
            i % 7 + 1
        ));
        prev = format!("v{i}");
    }
    src.push_str(&format!("  {prev}\n}}\npub fn main() -> Int {{ f(1) }}\n"));
    // What it means, by the same arithmetic.
    let mut want: i64 = 1;
    for i in 0..n {
        want = (want * 3 + (i % 7 + 1)) % 1000003;
    }
    let (src, tag) = (src, "long".to_string());
    let (seconds, runs) = std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(move || {
            let started = std::time::Instant::now();
            let (_, runs) = build_and_run(&tag, &src, Optimise::Speed);
            (started.elapsed().as_secs(), runs)
        })
        .expect("spawn")
        .join()
        .expect("built");
    assert_eq!(runs[0], (0, format!("{want}\n"), String::new()));
    assert!(seconds < 60, "built in {seconds} s");
}
