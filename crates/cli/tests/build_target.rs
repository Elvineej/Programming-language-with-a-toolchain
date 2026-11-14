//! `elya build --target <board>` (spec 2026-11-14-elya-build-target): the real
//! CLI builds an ELF for each board, which QEMU runs, writing what the program
//! means; and a board whose tools are missing is a failure by name, with no
//! ELF. Self-contained like `build_cli.rs`: it spawns `elya` and then QEMU.
//!
//! Needs what the `boards` feature promises (scripts/check.sh's configuration
//! C requires it): `clang`, `ld.lld`, the boards' GCCs and QEMUs.
#![cfg(feature = "boards")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-target-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

const PROGRAM: &str = "type L { Nil, Cons(Int, L) }\n\
    fn sum(xs: L, acc: Int) -> Int { match xs { Nil -> acc  Cons(h, t) -> sum(t, acc + h) } }\n\
    fn build(n: Int, acc: L) -> L { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
    pub fn main() -> Int { io.println(\"on a board\")  sum(build(1000, Nil), 0) }\n";

fn elya(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_elya"))
        .arg("build")
        .args(args)
        .output()
        .expect("spawn elya build")
}

/// The ELF's class and machine: 32-bit, and EM_ARM (40) or EM_RISCV (243).
fn elf_machine(elf: &Path) -> u16 {
    let bytes = std::fs::read(elf).expect("read the ELF");
    assert_eq!(&bytes[..4], b"\x7fELF", "{}", elf.display());
    assert_eq!(bytes[4], 1, "ELFCLASS32");
    assert_eq!(bytes[5], 1, "little-endian");
    u16::from_le_bytes([bytes[18], bytes[19]])
}

fn run_on(qemu: &[&str], elf: &Path) -> (Option<i32>, String, String) {
    let out = Command::new(qemu[0])
        .args(&qemu[1..])
        .args(["-display", "none", "-serial", "none", "-monitor", "none"])
        .args(["-semihosting-config", "enable=on,target=native", "-kernel"])
        .arg(elf)
        .output()
        .expect("spawn QEMU");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The default output is the source's stem with `.elf`; the given one is
/// taken as it is, the flags in either order.
#[test]
fn elya_builds_an_elf_for_each_board_that_qemu_runs() {
    let dir = temp_dir("boards");
    let src = dir.join("prog.elya");
    std::fs::write(&src, PROGRAM).expect("write source");
    let want = (Some(0), "on a board\n500500\n".to_string(), String::new());

    let out = elya(&[src.as_os_str(), "--target".as_ref(), "cortex-m3".as_ref()]);
    assert!(out.status.success(), "{out:?}");
    let elf = dir.join("prog.elf");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("built {}\n", elf.display())
    );
    assert_eq!(elf_machine(&elf), 40);
    assert_eq!(run_on(&["qemu-system-arm", "-M", "mps2-an385"], &elf), want);

    let custom = dir.join("custom.elf");
    let out = elya(&[
        "--target".as_ref(),
        "riscv32".as_ref(),
        src.as_os_str(),
        "-o".as_ref(),
        custom.as_os_str(),
    ]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(elf_machine(&custom), 243);
    assert_eq!(
        run_on(
            &["qemu-system-riscv32", "-M", "virt", "-bios", "none"],
            &custom
        ),
        want
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A board's tool that does not run is named, with what sets it, and no ELF
/// is left behind.
#[test]
fn a_missing_board_tool_is_named() {
    let dir = temp_dir("missing");
    let src = dir.join("prog.elya");
    std::fs::write(&src, PROGRAM).expect("write source");
    let out = Command::new(env!("CARGO_BIN_EXE_elya"))
        .args([
            "build".as_ref(),
            src.as_os_str(),
            "--target".as_ref(),
            "riscv32".as_ref(),
        ])
        .env("ELYA_LLD", dir.join("no-such-ld.lld"))
        .output()
        .expect("spawn elya build");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("board 'riscv32'") && err.contains("(from ELYA_LLD) does not run"),
        "{err}"
    );
    assert!(!dir.join("prog.elf").exists());
    std::fs::remove_dir_all(&dir).ok();
}
