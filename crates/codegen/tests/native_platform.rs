//! The runtime behind its platform layer (spec 2026-11-13-elya-platform-layer):
//! `runtime.c` built freestanding for bare-metal targets, and Elya programs
//! linked with a board's platform and NO C library, writing what the hosted
//! build writes.

use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-platform-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn runtime_c() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime.c")
}

fn llvm_nm() -> String {
    for name in ["llvm-nm", "llvm-nm-18"] {
        if Command::new(name).arg("--version").output().is_ok() {
            return name.to_string();
        }
    }
    panic!("no llvm-nm on PATH");
}

/// The whole runtime builds freestanding (`-ffreestanding -nostdlib -Werror`)
/// for a PC with no operating system, a Cortex-M4 and a 32-bit RISC-V. What it
/// leaves undefined is what a board gives: bytes out and stop
/// (`elya_platform_write`, `elya_platform_exit`), the three memory functions a
/// freestanding C compiler may call, and the compiler's own arithmetic helpers
/// (`__aeabi_*`, `__*df*`, `__*di*`: soft floats and 64-bit division on 32-bit
/// machines). No C library function.
#[test]
fn the_runtime_builds_freestanding() {
    let dir = temp_dir("freestanding");
    for target in [
        "x86_64-unknown-none-elf",
        "thumbv7em-none-eabi",
        "riscv32-unknown-none-elf",
    ] {
        let obj = dir.join(format!("runtime-{target}.o"));
        let built = Command::new("clang")
            .args([
                "-DELYA_FREESTANDING",
                "-ffreestanding",
                "-nostdlib",
                "-O2",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-c",
            ])
            .arg(format!("--target={target}"))
            .arg(runtime_c())
            .arg("-o")
            .arg(&obj)
            .output()
            .expect("spawn clang");
        assert!(
            built.status.success(),
            "{target}: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        let nm = Command::new(llvm_nm())
            .arg("-u")
            .arg(&obj)
            .output()
            .expect("spawn llvm-nm");
        let undefined: Vec<String> = String::from_utf8_lossy(&nm.stdout)
            .lines()
            .filter_map(|l| l.split_whitespace().last().map(str::to_string))
            .collect();
        for u in &undefined {
            assert!(
                u == "elya_platform_write"
                    || u == "elya_platform_exit"
                    || u == "memcpy"
                    || u == "memset"
                    || u == "memcmp"
                    || u.starts_with("__"),
                "{target}: the runtime needs `{u}`: {undefined:?}"
            );
        }
        assert!(
            undefined.iter().any(|u| u == "elya_platform_write"),
            "{target}: {undefined:?}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A board for the machine the tests run on: no C library at all (`-nostdlib
/// -static`, its own `_start`), bytes out and stop by system call, the
/// collector's memory one fixed region. Status 70 names a board that returned.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const BOARD: &str = r#"
#include <stddef.h>
#include <stdint.h>
void elya_heap_init(void *base, size_t bytes);
int main(int argc, char **argv);
static long sys3(long n, long a, long b, long c) {
    long r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return r;
}
void elya_platform_write(int stream, const char *p, size_t n) {
    while (n) {
        long w = sys3(1, stream, (long)p, (long)n);
        if (w <= 0) break;
        p += w; n -= (size_t)w;
    }
}
void elya_platform_exit(int code) { for (;;) sys3(231, code, 0, 0); }
__attribute__((no_builtin)) void *memcpy(void *d, const void *s, size_t n) {
    unsigned char *a = d; const unsigned char *b = s;
    for (size_t i = 0; i < n; i++) a[i] = b[i];
    return d;
}
__attribute__((no_builtin)) void *memset(void *d, int c, size_t n) {
    unsigned char *a = d;
    for (size_t i = 0; i < n; i++) a[i] = (unsigned char)c;
    return d;
}
__attribute__((no_builtin)) int memcmp(const void *x, const void *y, size_t n) {
    const unsigned char *a = x, *b = y;
    for (size_t i = 0; i < n; i++) if (a[i] != b[i]) return a[i] < b[i] ? -1 : 1;
    return 0;
}
static unsigned char region[REGION] __attribute__((aligned(4096)));
void board_start(void) {
    elya_heap_init(region, sizeof region);
    elya_platform_exit(main(0, 0) == 0 ? 0 : 70);
}
__asm__(".globl _start\n_start:\n xor %ebp, %ebp\n and $-16, %rsp\n call board_start\n");
"#;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn lower(src: &str) -> elya::core::CoreModule {
    let (module, diags, _) = elya::front_end_with("board.elya", src, &mut |_| None);
    assert!(diags.is_empty(), "{diags:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diags.is_empty(), "{diags:?}");
    elya::core::lower_module(&module, &table).expect("lowers")
}

/// (status, stdout, stderr) of `src` built for the board (a region of `region`
/// bytes) and of `src` built hosted.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn board_and_hosted(tag: &str, src: &str, region: usize) -> [(i32, String, String); 2] {
    board_and_hosted_with(tag, src, region, "-DELYA_SHADOW_ROOTS=1024")
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn board_and_hosted_with(
    tag: &str,
    src: &str,
    region: usize,
    define: &str,
) -> [(i32, String, String); 2] {
    let dir = temp_dir(tag);
    let core = lower(src);
    let obj = dir.join("program.o");
    elya_codegen::compile_module(&core, &obj).expect("compile_module");
    let board_c = dir.join("board.c");
    std::fs::write(&board_c, BOARD).expect("write board");
    let exe = dir.join("on-board");
    let built = Command::new("clang")
        .args([
            "-DELYA_FREESTANDING",
            "-ffreestanding",
            "-nostdlib",
            "-static",
        ])
        .args(["-fno-pie", "-O2", "-Wall", "-Werror"])
        .arg(format!("-DREGION={region}"))
        .arg(define)
        .arg(&obj)
        .arg(runtime_c())
        .arg(&board_c)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("spawn clang");
    assert!(
        built.status.success(),
        "{tag}: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let hosted = dir.join("hosted");
    elya_codegen::link(&obj, &hosted).expect("link");
    let run = |exe: &Path| {
        let out = Command::new(exe).output().expect("run");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let both = [run(&exe), run(&hosted)];
    std::fs::remove_dir_all(&dir).ok();
    both
}

/// Programs that print, build and drop data (collections in a 256 KiB region),
/// handle effects, and write Floats and Strings write on the board, with no C
/// library, exactly what they write hosted.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn programs_on_a_board_write_what_they_write_hosted() {
    let programs = [
        ("int", "pub fn main() { 6 * 7 }\n", "42\n"),
        ("negative", "pub fn main() { 0 - 9223372036854775807 - 1 }\n", "-9223372036854775808\n"),
        (
            "println",
            "pub fn main() { io.println(\"hello\") io.println(\"\") io.println(\"wörld\") 7 }\n",
            "hello\n\nwörld\n7\n",
        ),
        (
            "lists",
            "type List { Nil, Cons(Int, List) }\n\
             fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
             fn sum(xs, acc) { match xs { Cons(h, t) -> sum(t, acc + h) Nil -> acc } }\n\
             fn rounds(n, acc) { if n == 0 { acc } else { rounds(n - 1, acc + sum(build(1000, Nil), 0)) } }\n\
             pub fn main() { io.println(int.to_string(rounds(300, 0))) rounds(300, 0) }\n",
            "150150000\n150150000\n",
        ),
        (
            "strings",
            "fn go(n, s) { if n == 0 { s } else { go(n - 1, s <> int.to_string(n)) } }\n\
             pub fn main() { go(12, \"n:\") }\n",
            "\"n:121110987654321\"\n",
        ),
        (
            "handler",
            "effect Ask { fn ask() -> Int }\n\
             fn twice() -> Int { ask() + ask() }\n\
             pub fn main() -> Int { handle { twice() } with { Ask.ask() -> resume(21)  return(x) -> x } }\n",
            "42\n",
        ),
        (
            "floats",
            "pub fn main() { io.println(float.to_string(0.1 +. 0.2)) io.println(float.to_string(1.0e21)) 1.0 /. 3.0 }\n",
            "0.30000000000000004\n1000000000000000000000.0\n0.3333333333333333\n",
        ),
        (
            "data",
            "type Shape { Circle(Int), Rect(Int, Int) }\n\
             pub fn main() { Rect(3, 4) }\n",
            "Rect(3, 4)\n",
        ),
    ];
    for (tag, src, want) in programs {
        let [board, hosted] = board_and_hosted(tag, src, 256 * 1024);
        assert_eq!(
            hosted,
            (0, want.to_string(), String::new()),
            "{tag}: hosted"
        );
        assert_eq!(board, hosted, "{tag}: the board");
    }
}

/// A failure is the same line and the same status on the board, after the
/// output already written.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_failure_on_a_board_is_the_hosted_line_and_status() {
    let src = "fn zero() { 0 }\npub fn main() { io.println(\"before\") 1 / zero() }\n";
    let [board, hosted] = board_and_hosted("failure", src, 256 * 1024);
    assert_eq!(
        hosted,
        (
            1,
            "before\n".to_string(),
            "elya: division by zero\n".to_string()
        )
    );
    assert_eq!(board, hosted);
}

#[path = "../../../tests/programs/files_and_arguments.rs"]
#[allow(dead_code)]
mod programs;

/// A board has no files, no arguments and empty input (spec
/// 2026-11-14-elya-files-and-arguments D5): the hooks' defaults, by name.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_board_has_no_files_no_arguments_and_no_input() {
    use programs::{NO_WORLD, NO_WORLD_FAILURE, NO_WORLD_OUT};
    let [board, hosted] = board_and_hosted("no-world", NO_WORLD, 256 * 1024);
    assert_eq!(
        board,
        (
            1,
            NO_WORLD_OUT.to_string(),
            format!("elya: {NO_WORLD_FAILURE}\n")
        )
    );
    // Hosted, the same questions have the host's answers (there is no
    // `a.txt` where the test runs).
    assert_eq!(
        hosted,
        (
            1,
            NO_WORLD_OUT.to_string(),
            "elya: cannot read `a.txt`: no such file or directory\n".to_string()
        )
    );
}

/// A live set larger than the board's region is the out-of-memory line, status
/// 1: no chunk is asked of an operating system that is not there.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_board_out_of_memory_fails_by_name() {
    let src = "type List { Nil, Cons(Int, List) }\n\
               fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
               fn len(xs, acc) { match xs { Cons(_, t) -> len(t, acc + 1) Nil -> acc } }\n\
               pub fn main() { len(build(100000, Nil), 0) }\n";
    let [board, hosted] = board_and_hosted("oom", src, 64 * 1024);
    assert_eq!(hosted, (0, "100000\n".to_string(), String::new()));
    assert_eq!(
        board,
        (1, String::new(), "elya: out of memory\n".to_string())
    );
}

/// A board's roots grow past the fixed array into its region (the review's R2:
/// a board giving no page source had 1024 roots in all): a recursion 5000 deep
/// writes what it writes hosted.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_board_grows_its_roots_in_its_region() {
    let src = "type L { Nil, Cons(Int, L) }\n\
               fn build(n) { if n == 0 { Nil } else { Cons(n, build(n - 1)) } }\n\
               fn len(xs) { match xs { Nil -> 0  Cons(h, t) -> 1 + len(t) } }\n\
               pub fn main() { len(build(5000)) }\n";
    let [board, hosted] = board_and_hosted("roots", src, 1024 * 1024);
    assert_eq!(hosted, (0, "5000\n".to_string(), String::new()));
    assert_eq!(board, hosted);
}

/// Roots past the fixed array keep what they hold across collections: 5000
/// frames each hold a cell of their own across the call below them, whose
/// bottom makes 30,000 garbage cells (collections in 256 KiB), then each frame
/// reads its cell.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_board_keeps_what_its_grown_roots_hold() {
    let src = "type L { Nil, Cons(Int, L) }\n\
               fn head(xs) { match xs { Nil -> 0  Cons(h, t) -> h } }\n\
               fn churn(n, acc) { if n == 0 { acc } else { churn(n - 1, acc + head(Cons(n, Nil))) } }\n\
               fn d(n, xs) { if n == 0 { churn(30000, 0) } else { d(n - 1, Cons(n, Nil)) + head(xs) } }\n\
               pub fn main() { d(5000, Cons(0, Nil)) }\n";
    let [board, hosted] = board_and_hosted("kept", src, 256 * 1024);
    assert_eq!(hosted, (0, "462517499\n".to_string(), String::new()));
    assert_eq!(board, hosted);
}

/// The runtime's settings are read as the C library read them: a leading space
/// before a number is skipped (the review's N3: `ELYA_GC_STRESS=' 1'` turned
/// stress mode off).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn settings_read_as_the_c_library_reads_them() {
    let dir = temp_dir("settings");
    let core = lower("type L { Nil, Cons(Int, L) }\npub fn main() { match Cons(1, Cons(2, Nil)) { Nil -> 0  Cons(h, t) -> h } }\n");
    let obj = dir.join("program.o");
    elya_codegen::compile_module(&core, &obj).expect("compile_module");
    let exe = dir.join("hosted");
    elya_codegen::link(&obj, &exe).expect("link");
    for stress in [" 1", "\t1", "+1"] {
        let out = Command::new(&exe)
            .env("ELYA_GC_STRESS", stress)
            .env("ELY_GC_STATS", "1")
            .output()
            .expect("run");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("collections=") && !err.contains("collections=0 "),
            "ELYA_GC_STRESS={stress:?}: {err}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// More roots at once than the region holds is named, not a write past it.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn a_board_out_of_roots_fails_by_name() {
    let src = "fn d(n: Int, s: String) -> Int { if n == 0 { 0 } else { d(n - 1, s) + string.length(s) } }\n\
               pub fn main() { d(30000, \"ab\") }\n";
    let [board, hosted] = board_and_hosted("noroots", src, 64 * 1024);
    assert_eq!(hosted, (0, "60000\n".to_string(), String::new()));
    assert_eq!(
        board,
        (
            1,
            String::new(),
            "elya: out of memory growing the shadow stack\n".to_string()
        )
    );
}

/// The runtime's working memory is free again as soon as it is freed (the
/// review's B1): `==` on data 10,000 times, a long list searched by `==`, and
/// 3,000 operations shown and decoded by a `with any` handler, each in a region
/// its live data barely touches. 250 `==` filled 256 KiB before.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn working_memory_on_a_board_is_not_garbage() {
    let programs = [
        (
            "eq",
            "type P { P(Int, Int) }\n\
             fn go(n, a, b, acc) { if n == 0 { acc } else { if a == b { go(n - 1, a, b, acc + 1) } else { go(n - 1, a, b, acc) } } }\n\
             pub fn main() { go(10000, P(1, 2), P(1, 2), 0) }\n",
            256 * 1024,
            "10000\n",
        ),
        (
            "eq-lists",
            "type L { Nil, Cons(Int, L) }\n\
             fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
             fn go(n, a, b, acc) { if n == 0 { acc } else { if a == b { go(n - 1, a, b, acc + 1) } else { go(n - 1, a, b, acc) } } }\n\
             pub fn main() { go(2000, build(300, Nil), build(300, Nil), 0) }\n",
            256 * 1024,
            "2000\n",
        ),
        (
            "eq-lists-differ",
            "type L { Nil, Cons(Int, L) }\n\
             fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
             fn go(n, a, b, acc) { if n == 0 { acc } else { if a == b { go(n - 1, a, b, acc + 1) } else { go(n - 1, a, b, acc) } } }\n\
             pub fn main() { go(2000, Cons(1, build(300, Nil)), Cons(2, build(300, Nil)), 0) }\n",
            256 * 1024,
            "0\n",
        ),
        (
            "search",
            "type P { P(Int, Int) }\n\
             type L { Nil, Cons(P, L) }\n\
             fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(P(n, n), acc)) } }\n\
             fn count(xs, p, acc) { match xs { Nil -> acc  Cons(h, t) -> count(t, p, if h == p { acc + 1 } else { acc }) } }\n\
             pub fn main() { count(build(40000, Nil), P(7, 7), 0) }\n",
            4 * 1024 * 1024,
            "1\n",
        ),
        (
            "any",
            "type S { Box(Int, Int), Dot }\n\
             effect Q { fn mix(a: Int, c: S) -> Int }\n\
             fn rec(body) { handle { body() } with any { op(o) -> { let v = op.decode(o, int.to_string(string.length(op.show(o))))  resume(v) } } }\n\
             fn go(n, acc) { if n == 0 { acc } else { go(n - 1, acc + mix(n, Box(n, 1))) } }\n\
             pub fn main() { rec(fn() { go(3000, 0) }) }\n",
            256 * 1024,
            "",
        ),
    ];
    for (tag, src, region, want) in programs {
        let [board, hosted] = board_and_hosted(tag, src, region);
        assert_eq!(hosted.0, 0, "{tag}: hosted {hosted:?}");
        if !want.is_empty() {
            assert_eq!(hosted.1, want, "{tag}: hosted");
        }
        assert_eq!(board, hosted, "{tag}: the board");
    }
}
