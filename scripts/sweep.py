#!/usr/bin/env python3
"""A differential sweep: every program the repository knows, through the
evaluator (`elya run --value`) and natively (`elya build` + the binary).

Spec: docs/superpowers/specs/2026-10-16-elya-differential-sweep-design.md (D3).
A developer tool, not part of the gate. Standard library only.

    python3 scripts/sweep.py [--out DIR] [--elya PATH] [--jobs N] [--timeout S]

Builds `target/debug/elya` with the codegen feature unless `--elya` is given.
Writes DIR/programs/*.elya, DIR/report.md and DIR/results.tsv, and prints the
summary. Exit status 1 when any result is in FAILING (a disagreement, a native
crash or hang, a panic of the CLI); `differ-timing` (a program reading the real
clock) is noise, and `both-fail-differently` is listed for a person to read.
"""

import argparse
import concurrent.futures
import hashlib
import os
import pathlib
import re
import shutil
import subprocess
import sys
from collections import Counter

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Results that fail the sweep (exit status 1): a disagreement, or the toolchain
# itself crashing, panicking or hanging where the evaluator finished.
FAILING = ["differ", "native-crash", "native-timeout", "build-panic", "front-end-panic", "eval-panic", "sweep-error"]

# A Rust string literal: "..." with escapes, or r#"..."# raw strings. Line
# comments and char literals are matched too, and skipped, so a `'"'` or a quote
# in a comment does not throw the scan off (the review's finding 6).
STR_RE = re.compile(
    r"//[^\n]*|'(?:[^'\\\n]|\\.)'|\br(#*)\"(.*?)\"\1|b?\"((?:[^\"\\]|\\.)*)\"", re.S
)


def unescape(body: str) -> str:
    out = []
    i = 0
    while i < len(body):
        c = body[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        i += 1
        if i >= len(body):
            break
        e = body[i]
        if e == "\n":  # line continuation: skip the newline and leading whitespace
            i += 1
            while i < len(body) and body[i] in " \t\r\n":
                i += 1
            continue
        if e == "n":
            out.append("\n")
        elif e == "t":
            out.append("\t")
        elif e == "r":
            out.append("\r")
        elif e == "0":
            out.append("\0")
        elif e in "\"'\\":
            out.append(e)
        elif e == "x":
            out.append(chr(int(body[i + 1 : i + 3], 16)))
            i += 2
        elif e == "u":
            j = body.index("}", i)
            out.append(chr(int(body[i + 2 : j], 16)))
            i = j
        else:
            out.append("\\" + e)
        i += 1
    return "".join(out)


def literals(path: pathlib.Path):
    text = path.read_text(encoding="utf-8", errors="replace")
    for m in STR_RE.finditer(text):
        if m.group(2) is not None:
            yield m.group(2)
        elif m.group(3) is not None:
            try:
                yield unescape(m.group(3))
            except (ValueError, IndexError):
                continue  # a malformed escape: not a program


def collect():
    progs = {}

    def add(origin, src):
        if "fn main(" not in src:
            return
        key = hashlib.sha1(src.encode()).hexdigest()[:12]
        progs.setdefault(key, (origin, src))

    for p in sorted((ROOT / "examples").glob("*.elya")):
        add(f"examples/{p.name}", p.read_text(encoding="utf-8"))
    for p in sorted((ROOT / "tests" / "ui").glob("*.elya")):
        add(f"tests/ui/{p.name}", p.read_text(encoding="utf-8"))
    rs = sorted((ROOT / "tests").glob("*.rs")) + sorted(ROOT.glob("crates/*/tests/*.rs"))
    for p in rs:
        rel = p.relative_to(ROOT)
        for lit in literals(p):
            add(str(rel), lit)
            if "{{" in lit:  # a format! template: its literal braces
                add(str(rel), lit.replace("{{", "{").replace("}}", "}"))
    return progs


def run(cmd, timeout, env=None):
    try:
        r = subprocess.run(cmd, capture_output=True, timeout=timeout, env=env)
        return r.returncode, r.stdout.decode("utf-8", "replace"), r.stderr.decode("utf-8", "replace")
    except subprocess.TimeoutExpired:
        return None, "", ""


def panicked(rc, err):
    """The CLI itself failed: a Rust panic (exit 101) or a signal, never one of
    its own exit statuses (0 success, 1 a reported failure, 2 usage)."""
    return rc is not None and (rc not in (0, 1, 2) or "panicked at" in err)


def reads_clock(path: pathlib.Path) -> bool:
    code = re.sub(r"//[^\n]*", "", path.read_text(encoding="utf-8"))
    return "time.now_ms" in code


def native_message(err: str) -> str:
    for line in err.splitlines():
        if line.startswith("elya: "):
            return line[len("elya: ") :].strip()
    return ""


def classify(elya, path: pathlib.Path, timeout):
    try:
        return classify_inner(elya, path, timeout)
    except Exception as e:  # one broken program never kills the sweep
        return "sweep-error", repr(e)[:200]


def classify_inner(elya, path: pathlib.Path, timeout):
    rc, _, err = run([elya, "check", str(path)], timeout)
    if rc is None:
        return "check-timeout", ""
    if panicked(rc, err):
        return "front-end-panic", err.strip()[-240:]
    if rc != 0:
        return "not-checked", ""
    erc, eout, eerr = run([elya, "run", "--value", str(path)], timeout)
    if panicked(erc, eerr):
        return "eval-panic", eerr.strip()[-240:]
    exe = path.with_suffix(".exe" if os.name == "nt" else "")
    brc, bout, berr = run([elya, "build", str(path), "-o", str(exe)], timeout * 4)
    if brc is None:
        return "build-error", "build timed out"
    if erc is None and brc != 0:
        return "eval-timeout", "and native refused or failed to build"
    if brc != 0:
        if "panicked at" in berr:
            m = re.search(r"panicked at ([^\n]*)\n([^\n]*)", berr)
            return "build-panic", (m.group(1) + " " + m.group(2) if m else "")[:240]
        m = re.search(r"unsupported construct \((.*)\)", berr)
        if m:
            return "refused", m.group(1)
        last = [l for l in berr.strip().splitlines() if l.strip()]
        return "build-error", (last[-1] if last else berr)[:200]
    nrc, nout, nerr = run([str(exe)], timeout)
    try:
        exe.unlink()
    except OSError:
        pass
    if erc is None and nrc is None:
        return "both-timeout", ""
    if erc is None:
        return "eval-timeout", ""
    if nrc is None:
        return "native-timeout", ""
    if nrc < 0 or nrc > 128:
        return "native-crash", f"rc={nrc} stderr={nerr.strip()[:200]}"
    if erc == 0 and nrc == 0:
        if eout == nout:
            return "same", ""
        # A program that reads the real clock may print elapsed times that differ.
        kind = "differ-timing" if reads_clock(path) else "differ"
        return kind, f"evaluator={eout[-200:]!r} native={nout[-200:]!r}"
    if erc != 0 and nrc != 0:
        # The evaluator prints no value line after a failure; neither does native.
        if eout != nout:
            return "differ", f"both fail; evaluator={eout[-200:]!r} native={nout[-200:]!r}"
        msg = native_message(nerr)
        if erc == nrc and msg and msg in eerr:
            return "both-fail", msg
        # Both failed after the same output, but not provably the same way
        # (another status, or words that differ): listed, for a person to read.
        return "both-fail-differently", f"evaluator rc={erc}; native rc={nrc} {nerr.strip()[-160:]!r}"
    return "differ", f"evaluator rc={erc} {eerr.strip()[-200:]!r}; native rc={nrc} {nerr.strip()[-200:]!r}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "target" / "sweep"))
    ap.add_argument("--elya")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 2)
    ap.add_argument("--timeout", type=float, default=30.0)
    a = ap.parse_args()
    elya = a.elya
    if not elya:
        subprocess.run(
            ["cargo", "build", "-q", "-p", "elya-cli", "--features", "codegen"], cwd=ROOT, check=True
        )
        elya = str(ROOT / "target" / "debug" / ("elya.exe" if os.name == "nt" else "elya"))
    if not pathlib.Path(elya).is_file():
        sys.exit(f"sweep: no elya binary at {elya}")
    out = pathlib.Path(a.out)
    pdir = out / "programs"
    shutil.rmtree(pdir, ignore_errors=True)
    pdir.mkdir(parents=True, exist_ok=True)
    progs = collect()
    items = []
    for i, (key, (origin, src)) in enumerate(sorted(progs.items(), key=lambda kv: (kv[1][0], kv[0]))):
        stem = re.sub(r"[^A-Za-z0-9]+", "_", origin)[-40:]
        path = pdir / f"{i:04d}-{stem}-{key}.elya"
        path.write_text(src, encoding="utf-8")
        items.append((origin, path))
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, a.jobs)) as pool:
        results = list(pool.map(lambda it: (it[0], it[1], *classify(elya, it[1], a.timeout)), items))
    kinds = Counter(r[2] for r in results)
    refusals = Counter(r[3] for r in results if r[2] == "refused")
    with open(out / "results.tsv", "w", encoding="utf-8") as f:
        for origin, path, kind, detail in results:
            f.write(f"{kind}\t{path.name}\t{origin}\t{detail}\n")
    lines = [f"# Sweep: {len(results)} programs", "", "| result | count |", "|---|---|"]
    lines += [f"| {k} | {n} |" for k, n in kinds.most_common()]
    lines += ["", "## Native refusals, ranked", "", "| reason | count |", "|---|---|"]
    lines += [f"| {k} | {n} |" for k, n in refusals.most_common()]
    for want in FAILING + ["build-error", "both-fail-differently", "eval-timeout", "both-timeout", "check-timeout", "differ-timing"]:
        rows = [r for r in results if r[2] == want]
        if rows:
            lines += ["", f"## {want}", ""]
            lines += [f"- `{r[1].name}` ({r[0]}): {r[3]}" for r in rows]
    (out / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 1 if any(kinds[k] for k in FAILING) else 0


if __name__ == "__main__":
    sys.exit(main())
