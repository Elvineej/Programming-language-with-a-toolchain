//! A board's link (spec 2026-11-14-elya-build-target D2): an object compiled
//! for a board's processor (`compile_module_for`), the runtime and the board's
//! files built for the same processor, `ld.lld` with the board's linker script,
//! and the `libgcc` of the board's GCC (64-bit division, soft floats). No C
//! library. `elya build --target` and the board tests both link this way, so
//! the two cannot drift apart.

use crate::{CodegenError, Machine};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// A board Elya can build for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Board {
    /// QEMU's `mps2-an385`: a Cortex-M3 with 4 MiB of RAM.
    CortexM3,
    /// QEMU's `virt`: a 32-bit RISC-V (`rv32imac`) with 128 MiB of RAM.
    Riscv32,
}

/// What a board is built with.
struct Spec {
    name: &'static str,
    target: &'static [&'static str],
    dir: &'static str,
    gcc_env: &'static str,
    gcc: &'static [&'static str],
    gcc_target: &'static [&'static str],
    qemu: &'static str,
}

const CORTEX_M3: Spec = Spec {
    name: "cortex-m3",
    target: &[
        "--target=thumbv7m-none-eabi",
        "-mcpu=cortex-m3",
        "-mfloat-abi=soft",
    ],
    dir: "cortex-m3",
    gcc_env: "ELYA_ARM_GCC",
    gcc: &["arm-none-eabi-gcc"],
    gcc_target: &["-mcpu=cortex-m3", "-mthumb"],
    qemu: "qemu-system-arm -M mps2-an385",
};

const RISCV32: Spec = Spec {
    name: "riscv32",
    target: &[
        "--target=riscv32-unknown-elf",
        "-march=rv32imac",
        "-mabi=ilp32",
    ],
    dir: "riscv32",
    gcc_env: "ELYA_RISCV_GCC",
    gcc: &["riscv64-unknown-elf-gcc", "riscv32-unknown-elf-gcc"],
    gcc_target: &["-march=rv32imac", "-mabi=ilp32"],
    qemu: "qemu-system-riscv32 -M virt -bios none",
};

impl Board {
    /// Every board, in the order `elya build --target` lists them.
    pub const ALL: [Board; 2] = [Board::CortexM3, Board::Riscv32];

    /// The board `elya build --target <name>` names, if any.
    pub fn named(name: &str) -> Option<Board> {
        Board::ALL.into_iter().find(|b| b.name() == name)
    }

    /// The board's name on the command line.
    pub fn name(self) -> &'static str {
        self.spec().name
    }

    /// The machine its code is compiled for.
    pub fn machine(self) -> Machine {
        match self {
            Board::CortexM3 => Machine::cortex_m3(),
            Board::Riscv32 => Machine::rv32imac(),
        }
    }

    /// How QEMU runs a program built for it (`<qemu> -kernel prog.elf`).
    /// Without `-nographic`, which makes QEMU's stdout non-blocking: a full
    /// pipe then fails a write (spec 2026-11-14-elya-riscv-board D6).
    pub fn qemu_command(self) -> String {
        format!(
            "{} -display none -serial none -monitor none \
             -semihosting-config enable=on,target=native -kernel",
            self.spec().qemu
        )
    }

    fn spec(self) -> &'static Spec {
        match self {
            Board::CortexM3 => &CORTEX_M3,
            Board::Riscv32 => &RISCV32,
        }
    }
}

fn board_error(board: Board, what: String) -> CodegenError {
    CodegenError::Board(format!("board '{}': {what}", board.name()))
}

/// The tool named by `env` when it is set (that one only), else the first of
/// `names` on PATH that runs.
fn tool(env: &str, names: &[&str]) -> Result<String, String> {
    let runs = |name: &str| {
        Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if let Some(set) = std::env::var_os(env) {
        let set = set.to_string_lossy().into_owned();
        return if runs(&set) {
            Ok(set)
        } else {
            Err(format!("`{set}` (from {env}) does not run"))
        };
    }
    names
        .iter()
        .find(|n| runs(n))
        .map(|n| n.to_string())
        .ok_or_else(|| {
            format!(
                "needs `{}` on PATH (tried {}; or set {env})",
                names[0],
                names.join(", ")
            )
        })
}

/// A board's pieces, built once per process: the runtime, the memory
/// functions, the board's hooks and its start-up, each for its processor, and
/// its `libgcc`.
struct Pieces {
    lld: String,
    objects: Vec<PathBuf>,
    libgcc: PathBuf,
}

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn build_pieces(board: Board) -> Result<Pieces, String> {
    let spec = board.spec();
    let clang = tool("ELYA_CLANG", &["clang", "clang-18"])?;
    let lld = tool("ELYA_LLD", &["ld.lld", "ld.lld-18"])?;
    let gcc = tool(spec.gcc_env, spec.gcc)?;
    // A directory this process creates (it fails if it exists): no other
    // process shares or plants the objects (as `runtime_object` does).
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "elya-board-{}-{}-{nanos}",
        spec.name,
        std::process::id()
    ));
    std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let boards = manifest().join("boards");
    let c: &[&str] = &[
        "-DELYA_FREESTANDING",
        "-ffreestanding",
        "-nostdlib",
        "-O2",
        "-Wall",
        "-Werror",
    ];
    let sources: [(PathBuf, &[&str]); 4] = [
        (manifest().join("src/runtime.c"), c),
        (boards.join("common/mem.c"), c),
        (boards.join(spec.dir).join("board.c"), c),
        (boards.join(spec.dir).join("startup.s"), &[]),
    ];
    let mut objects = Vec::new();
    for (src, flags) in sources {
        let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned());
        let obj = dir.join(format!("{}.o", stem.unwrap_or_default()));
        let out = Command::new(&clang)
            .args(spec.target)
            .args(flags)
            .arg("-c")
            .arg(&src)
            .arg("-o")
            .arg(&obj)
            .output()
            .map_err(|e| format!("`{clang}`: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "`{clang}` could not build {}: {}",
                src.display(),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        objects.push(obj);
    }
    let out = Command::new(&gcc)
        .args(spec.gcc_target)
        .arg("-print-libgcc-file-name")
        .output()
        .map_err(|e| format!("`{gcc}`: {e}"))?;
    let libgcc = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    // A GCC with no library for this processor answers a bare `libgcc.a`.
    if !out.status.success() || !libgcc.is_absolute() || !libgcc.is_file() {
        return Err(format!(
            "`{gcc}` has no libgcc for {} (it answered `{}`)",
            spec.gcc_target.join(" "),
            libgcc.display()
        ));
    }
    Ok(Pieces {
        lld,
        objects,
        libgcc,
    })
}

fn pieces(board: Board) -> Result<&'static Pieces, CodegenError> {
    static M3: OnceLock<Result<Pieces, String>> = OnceLock::new();
    static RV: OnceLock<Result<Pieces, String>> = OnceLock::new();
    let cell = match board {
        Board::CortexM3 => &M3,
        Board::Riscv32 => &RV,
    };
    cell.get_or_init(|| build_pieces(board))
        .as_ref()
        .map_err(|e| board_error(board, e.clone()))
}

/// An object compiled for `board` (by `compile_module_for` with
/// `board.machine()`) -> an ELF that QEMU runs (`board.qemu_command()`). The
/// board's pieces are built at the first link of a process and kept.
pub fn link_for(obj: &Path, exe: &Path, board: Board) -> Result<(), CodegenError> {
    let p = pieces(board)?;
    let out = Command::new(&p.lld)
        .arg("-T")
        .arg(
            manifest()
                .join("boards")
                .join(board.spec().dir)
                .join("link.ld"),
        )
        .arg("-o")
        .arg(exe)
        .args(&p.objects)
        .arg(obj)
        .arg(&p.libgcc)
        .output()
        .map_err(|e| board_error(board, format!("`{}`: {e}", p.lld)))?;
    if !out.status.success() {
        return Err(board_error(
            board,
            format!(
                "`{}` exited {:?}: {}",
                p.lld,
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Board;

    /// The names `--target` takes, and back.
    #[test]
    fn boards_are_named() {
        for b in Board::ALL {
            assert_eq!(Board::named(b.name()), Some(b));
        }
        assert_eq!(Board::named("cortex-m3"), Some(Board::CortexM3));
        assert_eq!(Board::named("riscv32"), Some(Board::Riscv32));
        assert_eq!(Board::named("host"), None);
        assert!(Board::Riscv32.qemu_command().contains("-M virt -bios none"));
    }
}
