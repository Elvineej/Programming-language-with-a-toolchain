# Long-Running Commands (Windows / PowerShell)

This repo builds LLVM through vcpkg and links it into Rust via `inkwell`/`llvm-sys`.
A cold `vcpkg install llvm` or a first `cargo build --features codegen` can run **20+
minutes**. Cline executes every command in one terminal and waits for the prompt to
come back; a command that does not return quickly hangs that terminal and forces the
user to click **"Proceed While Running"** by hand.

**The rule: every command you execute must return to the prompt within a few seconds.**
Long work runs detached, writes to a log, and is inspected on a later turn.

## Hard rules

- **Never use `Start-Sleep`, `timeout`, `pause`, `Wait-Process`, `-Wait`, `.WaitForExit()`,
  or any other blocking wait to pass time while another process finishes.** Waiting is
  not your job — the next turn is the wait. This is a convention, not a mechanism:
  nothing in the repo enforces it for you, so it holds only as long as you follow it.
- **Never run a long build in the foreground of a Cline-executed command.** That includes
  `vcpkg install`, `cmake --build`, `ninja`, `cargo build --features codegen`, and
  `cargo test --features codegen`. For this repo's gate, use `./scripts/check-bg.ps1`
  — see [Running the gate](#running-the-gate-fmt--clippy--test).
- **Start long builds detached and redirect both output streams to log files:**
  ```powershell
  Start-Process -NoNewWindow -FilePath <exe> -ArgumentList <args> `
    -RedirectStandardOutput <log> -RedirectStandardError <errlog>
  ```
  Do **not** add `-Wait`. `-RedirectStandardOutput` and `-RedirectStandardError` must be
  **two different paths** — PowerShell fails if they are the same file.
- **Every command executed must return to the prompt within a few seconds.** If you cannot
  say how long a command takes, assume it is long and detach it.
- **To check on a background build, run exactly one short command that exits immediately.**
  Either tail the log or test for the expected artifact:
  ```powershell
  Get-Content <log> -Tail 20
  Test-Path C:\vcpkg\installed\vcpkg\info\llvm_*_x64-windows-static-md-rel.list
  ```
  One command per turn. No loops, no `while (-not (Test-Path ...))`, no re-checking inside
  the same invocation.
- **If the build is not finished, say so plainly and check again on the next turn.** Report
  what the log's last lines show; do not wait inside the command, and do not start a second
  copy of the same build.
- **Never chain a network or build command into a pipe filter in a single invocation.**
  `vcpkg install ... | Select-String error` keeps the pipeline open for the entire build —
  it is a foreground build wearing a filter. Write to a temp file first, then read the file:
  ```powershell
  # later turn, after the build finished
  Select-String -Path $env:TEMP\vcpkg-llvm.out.log -Pattern 'error|failed' | Select-Object -Last 20
  ```
- **Logs go outside the repo working tree** (`$env:TEMP\...`), never under a tracked path.
- Cargo on this machine needs `$env:CARGO_INCREMENTAL="0"` (the incremental cache hangs).
  Set it in the session before `Start-Process`; the detached child inherits it.

## Running the gate (fmt + clippy + test)

`scripts/check.ps1` **is** the gate: `cargo fmt --all -- --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test --all`. It is a line-for-line twin
of `scripts/check.sh` and must stay one — keep wrappers, switches, and conveniences out of it.

A cold run takes minutes, so **never invoke it bare from Cline.** Use `scripts/check-bg.ps1`,
which does not redefine the gate — it shells out to `scripts/check.ps1`, so there is exactly
one definition of what the gate runs:

```powershell
./scripts/check-bg.ps1            # starts detached, prints PID + log paths, returns at once
```

Then, on a **later** turn, exactly one status command:

```powershell
./scripts/check-bg.ps1 -Status    # tails both logs, says RUNNING / FINISHED - PASS / FINISHED - FAIL
```

- Starting refuses to launch a second copy while one is already running. If you get
  "already running", just use `-Status`.
- `-Status` never waits. If it says `RUNNING`, report that plainly and check again next turn.
- Logs are `$env:TEMP\elya-check.out.log` and `$env:TEMP\elya-check.err.log`; the PID is in
  `$env:TEMP\elya-check.pid` and the exit code lands in `$env:TEMP\elya-check.done` on completion.
- Bare `./scripts/check.ps1` is unchanged and still correct **for a human at their own terminal**.
  It is the form Cline must not use.

## Concrete example: this project's vcpkg LLVM build

Target: `llvm` for the `x64-windows-static-md-rel` triplet
(`C:\vcpkg\triplets\x64-windows-static-md-rel.cmake` — static libs, dynamic CRT, release only),
which is what `llvm-sys` links against for the `codegen` feature.

### Before — hangs Cline's terminal

```powershell
# BAD: 20+ minute foreground build, terminal is stuck the whole time
C:\vcpkg\vcpkg.exe install "llvm[core,target-x86,tools]:x64-windows-static-md-rel"

# BAD: blocking wait to "let it finish"
Start-Sleep -Seconds 900

# BAD: pipe filter holds the pipeline open for the entire build
C:\vcpkg\vcpkg.exe install "llvm:x64-windows-static-md-rel" | Select-String "error"
```

### After — returns immediately, checked on later turns

Turn 1 — start it detached and stop:

```powershell
$env:CARGO_INCREMENTAL="0"
Start-Process -NoNewWindow -FilePath "C:\vcpkg\vcpkg.exe" `
  -ArgumentList 'install','llvm[core,target-x86,tools]:x64-windows-static-md-rel' `
  -RedirectStandardOutput "$env:TEMP\vcpkg-llvm.out.log" `
  -RedirectStandardError  "$env:TEMP\vcpkg-llvm.err.log"
```

Then report: "vcpkg LLVM build started detached; logging to `$env:TEMP\vcpkg-llvm.out.log`."

Turn 2 — one short check:

```powershell
Get-Content "$env:TEMP\vcpkg-llvm.out.log" -Tail 20
```

Turn 3 — artifact check, which is the real completion signal:

```powershell
Test-Path "C:\vcpkg\installed\vcpkg\info\llvm_*_x64-windows-static-md-rel.list"
```

vcpkg writes that `.list` marker only once a port is fully installed, so it is the true
completion signal — more reliable than reading the log, and version-agnostic. A staged
`C:\vcpkg\packages\llvm_x64-windows-static-md-rel\` tree means the build is still
mid-flight, not finished.

`False` means not done: say "LLVM is still building — last log line was `<X>`; I'll check
again next turn," and move on to work that does not depend on it. Only once the artifact
exists do you run the dependent build — also detached:

```powershell
$env:CARGO_INCREMENTAL="0"
Start-Process -NoNewWindow -FilePath "cargo" `
  -ArgumentList 'build','--features','codegen' `
  -RedirectStandardOutput "$env:TEMP\elya-codegen-build.out.log" `
  -RedirectStandardError  "$env:TEMP\elya-codegen-build.err.log"
```

## Existing repo material that contradicts this rule

Do not copy these command forms verbatim; translate them to the detached pattern above.

- `docs/superpowers/plans/*.md` — the slice plans quote gate steps as
  `cargo test ... 2>&1 | grep -E "test result|FAILED"` (~49 occurrences, concentrated in
  `2026-08-13-elya-slice-4b1-closures.md`). These are exactly the pipe-into-filter form.
  They were written for a human at a normal shell; run the command detached to a log and
  `Select-String -Path` the log afterwards.
- `docs/superpowers/plans/2026-08-24-elya-slice-5b1-native-codegen-arith-mvp.md` — the
  Task 1–5 gates (`cargo build --features codegen`, `cargo test --features codegen`,
  `./scripts/check.ps1`) are the slowest commands in the repo (llvm-sys links a large
  native library). Always detach them.
- `scripts/check.sh` — the POSIX twin of the gate. It has no wrapper equivalent, so on a
  POSIX shell detach it by hand
  (`nohup ./scripts/check.sh >/tmp/elya-check.out.log 2>/tmp/elya-check.err.log &`).
  On PowerShell, use `./scripts/check-bg.ps1` instead of foregrounding `check.ps1`.
- `Continuous-Claude/.claude/agents/atlas.md` (~line 59) — recommends `npm run dev & sleep 5`
  to wait for a test server. Blocking wait; replace with a detached start plus a later
  one-shot readiness check.
- `Continuous-Claude/.claude/scripts/agent-animation.sh` (~line 50) — `sleep 0.3` inside an
  infinite render loop. Never execute this from Cline; it never returns.

Aligned already, no change needed: `Continuous-Claude/.claude/skills/no-polling-agents/SKILL.md`
and `.../background-agent-pings/SKILL.md` both list `sleep N && <check>` polling under
explicit **DON'T** headings.
