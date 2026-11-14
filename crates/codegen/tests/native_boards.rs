//! Elya programs compiled for two boards and run on QEMU (specs
//! 2026-11-13-elya-boards, 2026-11-14-elya-riscv-board): a Cortex-M3
//! (`mps2-an385`) and a 32-bit RISC-V (`virt`). The program, the runtime built
//! freestanding for the same processor, and the board's start-up, linked with
//! no C library, write what the hosted build writes, at both optimisation
//! levels.
//!
//! The `boards` feature says the boards can be run here, so each tool they need
//! is required, by name: `qemu-system-arm` and `qemu-system-riscv32` here, and
//! for the link (`elya_codegen::link_for`) `clang`, `ld.lld`, and
//! `arm-none-eabi-gcc` and `riscv64-unknown-elf-gcc` for their `libgcc`.
#![cfg(feature = "boards")]

use elya_codegen::{Board, CodegenError, Machine, Optimise};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-boards-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn first_on_path(names: &[&str]) -> String {
    for name in names {
        let ok = Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return name.to_string();
        }
    }
    panic!("the `boards` tests need one of {names:?} on PATH");
}

/// The QEMU that runs a board's programs (the board's own link is the
/// library's, `link_for`, which `elya build --target` uses too).
fn qemu(board: Board) -> &'static str {
    static ARM: OnceLock<String> = OnceLock::new();
    static RV: OnceLock<String> = OnceLock::new();
    let (cell, name) = match board {
        Board::CortexM3 => (&ARM, "qemu-system-arm"),
        Board::Riscv32 => (&RV, "qemu-system-riscv32"),
    };
    cell.get_or_init(|| first_on_path(&[name]))
}

fn lower(src: &str) -> elya::core::CoreModule {
    let (module, diags, _) = elya::front_end_with("board.elya", src, &mut |_| None);
    assert!(diags.is_empty(), "{diags:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diags.is_empty(), "{diags:?}");
    elya::core::lower_module(&module, &table).expect("lowers")
}

type Run = (i32, String, String);

/// Runs to the end (300 s at most), reading its output while it runs: a pipe
/// read only after the end would stop a program that writes more than the
/// pipe holds (the review of 52bf757).
fn wait(mut cmd: Command, what: &str) -> Run {
    use std::io::Read;
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    let drain = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            r.read_to_end(&mut bytes).ok();
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = drain(Box::new(child.stdout.take().expect("stdout")));
    let err = drain(Box::new(child.stderr.take().expect("stderr")));
    let deadline = Instant::now() + Duration::from_secs(300);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if Instant::now() > deadline {
            child.kill().ok();
            child.wait().ok();
            panic!("{what}: still running after 300 s");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    (
        status.code().unwrap_or(-1),
        out.join().expect("stdout reader"),
        err.join().expect("stderr reader"),
    )
}

/// `src` hosted, then on every board at each optimisation level, named.
fn hosted_and_boards(tag: &str, src: &str) -> (Run, Vec<(String, Run)>) {
    let dir = temp_dir(tag);
    let core = lower(src);
    let obj = dir.join("hosted.o");
    elya_codegen::compile_module(&core, &obj).expect("compile_module");
    let exe = dir.join("hosted");
    elya_codegen::link(&obj, &exe).expect("link");
    let hosted = wait(Command::new(&exe), &format!("{tag} hosted"));
    let mut runs = Vec::new();
    for board in Board::ALL {
        for (level, opt) in [("O2", Optimise::Speed), ("O0", Optimise::None)] {
            let what = format!("{tag} on {} at {level}", board.name());
            let obj = dir.join(format!("{}-{level}.o", board.name()));
            elya_codegen::compile_module_for(&core, &obj, opt, &board.machine())
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            let elf = dir.join(format!("{}-{level}.elf", board.name()));
            elya_codegen::link_for(&obj, &elf, board).unwrap_or_else(|e| panic!("{what}: {e}"));
            let command = board.qemu_command();
            let mut words = command.split_whitespace();
            let _program = words.next();
            let mut qemu_cmd = Command::new(qemu(board));
            qemu_cmd.args(words).arg(&elf);
            runs.push((what.clone(), wait(qemu_cmd, &what)));
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    (hosted, runs)
}

fn same_on_boards(tag: &str, src: &str, want: Run) {
    let (hosted, runs) = hosted_and_boards(tag, src);
    assert_eq!(hosted, want, "{tag}: hosted");
    for (what, run) in runs {
        assert_eq!(run, hosted, "{what}");
    }
}

fn ok(out: &str) -> Run {
    (0, out.to_string(), String::new())
}

/// Loops, recursion, data, closures, a handler with state, clauses of two and
/// three Ints (whose calls the ARM padding changes, held in the clause table),
/// 130 KiB of output (more than a pipe holds), and a failure: each board writes
/// what the host writes, and stops with its status.
#[test]
fn programs_on_the_boards_write_what_they_write_hosted() {
    let programs: [(&str, &str, Run); 11] = [
        (
            "sum",
            "fn remainder(x: Int, d: Int) -> Int { x - (x / d) * d }\n\
             fn go(i: Int, acc: Int) -> Int { if i == 300000 { acc } else { go(i + 1, acc + remainder(i * i, 7)) } }\n\
             pub fn main() -> Int { go(0, 0) }\n",
            ok("599998\n"),
        ),
        (
            "fib",
            "fn fib(n: Int) -> Int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\n\
             pub fn main() -> Int { fib(22) }\n",
            ok("17711\n"),
        ),
        (
            "primes",
            "fn remainder(x: Int, d: Int) -> Int { x - (x / d) * d }\n\
             fn prime(n: Int, d: Int) -> Bool { if d > n / d { True } else { if remainder(n, d) == 0 { False } else { prime(n, d + 2) } } }\n\
             fn go(n: Int, acc: Int) -> Int { if n >= 20000 { acc } else { go(n + 2, acc + (if prime(n, 3) { 1 } else { 0 })) } }\n\
             pub fn main() -> Int { 1 + go(3, 0) }\n",
            ok("2262\n"),
        ),
        (
            "queens",
            "type L { Nil, Cons(Int, L) }\n\
             fn safe(col: Int, xs: L, dist: Int) -> Bool { match xs { Nil -> True  Cons(h, t) -> if col == h || col - h == dist || h - col == dist { False } else { safe(col, t, dist + 1) } } }\n\
             fn positions(row: Int, col: Int, xs: L, acc: Int) -> Int { if col == 7 { acc } else { let a = if safe(col, xs, 1) { solve(row + 1, Cons(col, xs)) } else { 0 }  positions(row, col + 1, xs, acc + a) } }\n\
             fn solve(row: Int, xs: L) -> Int { if row == 7 { 1 } else { positions(row, 0, xs, 0) } }\n\
             pub fn main() -> Int { solve(0, Nil) }\n",
            ok("40\n"),
        ),
        (
            "trees",
            "type Tree { Leaf, Branch(Tree, Tree) }\n\
             fn build(depth: Int) -> Tree { if depth == 0 { Leaf } else { Branch(build(depth - 1), build(depth - 1)) } }\n\
             fn count(t: Tree) -> Int { match t { Leaf -> 1  Branch(a, b) -> 1 + count(a) + count(b) } }\n\
             fn rounds(n: Int, acc: Int) -> Int { if n == 0 { acc } else { let t = build(9)  rounds(n - 1, acc + count(t)) } }\n\
             pub fn main() -> Int { rounds(3, 0) }\n",
            ok("3069\n"),
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
            ok("2000\n"),
        ),
        (
            "closures",
            "type Fs { Nil, Cons(fn(Int) -> Int, Fs) }\n\
             fn build(k: Int, acc: Fs) -> Fs { if k == 100 { acc } else { build(k + 1, Cons(fn(x) { x + k }, acc)) } }\n\
             fn apply(fs: Fs, x: Int) -> Int { match fs { Nil -> x  Cons(f, t) -> apply(t, f(x)) } }\n\
             fn many(fs: Fs, i: Int, acc: Int) -> Int { if i == 100 { acc } else { many(fs, i + 1, acc + apply(fs, i)) } }\n\
             pub fn main() -> Int { many(build(0, Nil), 0, 0) }\n",
            ok("499950\n"),
        ),
        (
            "state",
            "effect State { fn get() -> Int  fn put(x: Int) -> Int }\n\
             fn tick(n: Int) -> Int { if n == 0 { get() } else { let x = get()  let _ = put(x + 1)  tick(n - 1) } }\n\
             pub fn main() -> Int {\n\
               let state = handle { tick(2000) } with {\n\
                 State.get() -> fn(s) { (resume(s))(s) }\n\
                 State.put(v) -> fn(s) { (resume(0))(v) }\n\
                 return(x) -> fn(s) { x }\n\
               }\n\
               state(0)\n\
             }\n",
            ok("2000\n"),
        ),
        (
            "clauses",
            "effect Mix { fn two(a: Int, b: Int) -> Int  fn three(a: Int, b: Int, c: Int) -> Int }\n\
             fn go(n: Int, acc: Int) -> Int { if n == 0 { acc } else { go(n - 1, acc + two(n, 1) + three(n, 2, 3)) } }\n\
             pub fn main() -> Int { handle { go(1000, 0) } with { Mix.two(a, b) -> resume(a * b)  Mix.three(a, b, c) -> resume(a + b * c)  return(x) -> x } }\n",
            ok("1007000\n"),
        ),
        (
            "output",
            "fn line(n: Int) -> Int { io.println(\"0123456789012345678901234567890123456789 \" <> int.to_string(n))  n }\n\
             fn go(n: Int, acc: Int) -> Int { if n == 0 { acc } else { go(n - 1, acc + line(n)) } }\n\
             pub fn main() -> Int { go(3000, 0) }\n",
            (0, (1..=3000).rev().map(|n| format!("0123456789012345678901234567890123456789 {n}\n")).collect::<String>() + "4501500\n", String::new()),
        ),
        (
            "failure",
            "fn zero(n: Int) -> Int { n - n }\n\
             pub fn main() -> Int { io.println(\"before\")  9223372036854775807 + (1 / (1 + zero(5))) }\n",
            (
                1,
                "before\n".to_string(),
                "elya: integer overflow\n".to_string(),
            ),
        ),
    ];
    for (tag, src, want) in programs {
        same_on_boards(tag, src, want);
    }
}

/// Every shape of call LLVM's ARM back end could disagree with itself about
/// (spec 2026-11-13-elya-boards D4), and every way RISC-V's one function type
/// carries words (spec 2026-11-14-elya-riscv-board D1): functions whose arguments stack 4, 8, 12, 16 and 24 bytes, with
/// words, pointers and Bools in each position, each calling itself in tail
/// position, each called by an ordinary call, a ring of tail calls between
/// five different shapes, ordinary recursion 3,000 deep through stacked
/// arguments, and a four-parameter closure called both ways.
#[test]
fn every_call_shape_on_the_boards_writes_what_it_writes_hosted() {
    let src = "type L { Nil, Cons(Int, L) }\n\
        fn len(xs: L, acc: Int) -> Int { match xs { Nil -> acc  Cons(_, t) -> len(t, acc + 1) } }\n\
        fn not(f: Bool) -> Bool { if f { False } else { True } }\n\
        fn s3(n: Int, x: Int, y: Int) -> Int { if n == 0 { x * 3 + y } else { s3(n - 1, y, x + 1) } }\n\
        fn p3(p: L, n: Int, x: Int) -> Int { if n == 0 { len(p, x) } else { p3(Cons(n, p), n - 1, x + 2) } }\n\
        fn ppp(p: L, q: L, r: L, n: Int) -> Int { if n == 0 { len(p, 0) * 100 + len(q, 0) * 10 + len(r, 0) } else { ppp(q, r, Cons(n, p), n - 1) } }\n\
        fn p5(p: L, q: L, r: L, s: L, t: L) -> Int { match t { Nil -> len(p, 0) + 2 * len(q, 0) + 3 * len(r, 0) + 4 * len(s, 0)  Cons(_, u) -> p5(q, r, s, Cons(1, p), u) } }\n\
        fn wwpw(x: Int, y: Int, p: L, z: Int) -> Int { if z == 0 { x - y + len(p, 0) } else { wwpw(y, x + z, Cons(z, p), z - 1) } }\n\
        fn w5(a: Int, b: Int, c: Int, d: Int, e: Int) -> Int { if e == 0 { a + 2 * b + 3 * c + 4 * d } else { w5(b, c, d, a + e, e - 1) } }\n\
        fn wwb(x: Int, y: Int, f: Bool) -> Int { if x == 0 { if f { y } else { 0 - y } } else { wwb(x - 1, y + x, not(f)) } }\n\
        fn pwp(p: L, x: Int, q: L) -> Int { if x == 0 { len(p, 0) - 2 * len(q, 0) } else { pwp(q, x - 1, Cons(x, p)) } }\n\
        fn wwwb(x: Int, y: Int, z: Int, f: Bool) -> Int { if z == 0 { if f { x } else { y } } else { wwwb(y, x + z, z - 1, not(f)) } }\n\
        fn ring1(n: Int, x: Int, y: Int) -> Int { if n == 0 { x * 7 + y } else { ring2(Cons(x, Nil), n, y, Nil, x) } }\n\
        fn ring2(p: L, n: Int, x: Int, q: L, y: Int) -> Int { ring3(n, x + len(p, 0), y, len(q, 0), 1) }\n\
        fn ring3(n: Int, b: Int, c: Int, d: Int, e: Int) -> Int { ring4(Cons(b, Nil), n, c + d + e) }\n\
        fn ring4(p: L, n: Int, x: Int) -> Int { ring5(n - 1, x + len(p, 0), n == 3) }\n\
        fn ring5(n: Int, x: Int, f: Bool) -> Int { ring1(n, x - 1, if f { 1 } else { 2 }) }\n\
        fn deep(n: Int, a: Int, b: Int) -> Int { if n == 0 { a - b } else { 1 + deep(n - 1, b, a + 1) } }\n\
        fn deep5(n: Int, a: Int, p: L, b: Int, c: Int) -> Int { if n == 0 { a + b + c + len(p, 0) } else { deep5(n - 1, b, Cons(c, p), c, a) + 1 } }\n\
        fn callk(k: fn(Int, Int, Int, Int) -> Int, n: Int) -> Int { if n == 0 { k(1, 2, 3, 4) } else { k(n, n + 1, n + 2, n + 3) + callk(k, n - 1) } }\n\
        fn show(x: Int) -> Int { io.println(int.to_string(x))  x }\n\
        pub fn main() -> Int {\n\
          let k = 5\n\
          let a = show(s3(1000, 1, 2)) + show(p3(Nil, 100, 0)) + show(ppp(Nil, Nil, Nil, 100))\n\
          let b = show(p5(Nil, Nil, Nil, Nil, Cons(1, Cons(2, Cons(3, Cons(4, Cons(5, Cons(6, Cons(7, Nil))))))))) + show(wwpw(1, 2, Nil, 100))\n\
          let c = show(w5(1, 2, 3, 4, 100)) + show(wwb(100, 0, True)) + show(pwp(Nil, 101, Nil)) + show(wwwb(1, 2, 101, False))\n\
          let d = show(ring1(1000, 1, 2)) + show(deep(3000, 1, 2)) + show(deep5(3000, 1, Nil, 2, 3))\n\
          a + b + c + d + show(callk(fn(a, b, c, d) { a * 1000 + b * 100 + c * 10 + d - k }, 100))\n\
        }\n";
    same_on_boards(
        "shapes",
        src,
        ok("2005\n300\n3664\n19\n149\n12530\n5050\n-52\n2552\n7009\n2999\n6006\n5623579\n5665810\n"),
    );
}

/// A recursion deeper than a board's stack stops at the guard below it with
/// the hosted build's line and status (boards spec D3), after the output
/// already written, at both levels: it never writes over the static data or the
/// heap.
#[test]
fn a_stack_overflow_on_the_boards_is_named() {
    let src = "fn deep(n: Int, a: Int, b: Int) -> Int { if n == 0 { a - b } else { 1 + deep(n - 1, b, a + 1) } }\n\
               pub fn main() -> Int { io.println(\"before\")  deep(1000000, 1, 2) }\n";
    let (hosted, runs) = hosted_and_boards("overflow", src);
    assert_eq!(hosted, ok("before\n999999\n"), "hosted");
    for (what, run) in runs {
        assert_eq!(
            run,
            (
                1,
                "before\n".to_string(),
                "elya: native stack overflow: recursion deeper than the 1024 KiB stack\n"
                    .to_string()
            ),
            "{what}"
        );
    }
}

/// The examples: async, structured concurrency, multi-shot search, replay,
/// `with any` recording, failure as an effect, a CEK machine. Each writes on
/// each board what it writes hosted.
#[path = "../../../tests/programs/files_and_arguments.rs"]
#[allow(dead_code)]
mod programs;

/// A board has no files, no arguments and empty input (spec
/// 2026-11-14-elya-files-and-arguments D5), by name, at both levels.
#[test]
fn a_board_has_no_files_no_arguments_and_no_input() {
    use programs::{NO_WORLD, NO_WORLD_FAILURE, NO_WORLD_OUT};
    let (hosted, runs) = hosted_and_boards("no-world", NO_WORLD);
    assert_eq!(
        hosted,
        (
            1,
            NO_WORLD_OUT.to_string(),
            "elya: cannot read `a.txt`: no such file or directory\n".to_string()
        ),
        "hosted"
    );
    for (what, run) in runs {
        assert_eq!(
            run,
            (
                1,
                NO_WORLD_OUT.to_string(),
                format!("elya: {NO_WORLD_FAILURE}\n")
            ),
            "{what}"
        );
    }
}

#[test]
fn the_examples_on_the_boards_write_what_they_write_hosted() {
    let dir = manifest().join("../../examples");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("examples")
        .map(|e| e.expect("an example").path())
        .filter(|p| p.extension().map(|e| e == "elya").unwrap_or(false))
        .collect();
    names.sort();
    assert!(names.len() >= 10, "{names:?}");
    for path in names {
        let tag = path
            .file_stem()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let src = std::fs::read_to_string(&path).expect("read");
        // The timers example's real-clock half orders three requests by
        // deadlines 10 ms apart, each counted from its own task's start, so a
        // machine slowed by the other tests can reorder them (HANDOFF, "Also
        // open"). This test failed once, unrecorded, in its first ten runs and
        // not in 26 since, under load; that example is the one whose output
        // depends on timing. It runs again, twice at most, before a difference
        // fails the test.
        let tries = if tag == "08_timers" { 3 } else { 1 };
        for attempt in 1..=tries {
            let (hosted, runs) = hosted_and_boards(&tag, &src);
            assert_eq!(hosted.0, 0, "{tag}: hosted {hosted:?}");
            let differs = runs.iter().find(|(_, run)| *run != hosted);
            match differs {
                None => break,
                Some((what, run)) if attempt == tries => assert_eq!(*run, hosted, "{what}"),
                Some((what, _)) => eprintln!("{what}: differs at try {attempt}, running again"),
            }
        }
    }
}

/// A machine with no Elya back end is refused by name, not by LLVM.
#[test]
fn another_machine_is_refused_by_name() {
    let dir = temp_dir("other");
    let core = lower("pub fn main() -> Int { 1 }\n");
    let other = Machine {
        triple: "aarch64-unknown-none".into(),
        cpu: String::new(),
        features: String::new(),
    };
    let r = elya_codegen::compile_module_for(&core, &dir.join("p.o"), Optimise::Speed, &other);
    assert!(
        matches!(r, Err(CodegenError::Unsupported(m)) if m.contains("32-bit RISC-V")),
        "{r:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
