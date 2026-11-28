// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! Modules and name resolution in Elya (self-hosting part 4c, spec
//! docs/superpowers/specs/2026-11-15-elya-resolution-in-elya-design.md D2-D4):
//! `selfhost/resolve.elya`, run in the evaluator, prints exactly `elya
//! resolve --dump` on every program of the parser's corpus, every
//! multi-file program in the Rust tests, and the focused set; and its
//! builtin tables are their Rust twins (D3).
//!
//! Negative controls, each run on selfhost/resolve.elya, reverted and
//! compared byte for byte (`cmp`); each failed this test and the native
//! one (crates/codegen/tests/selfhost_resolve.rs), first on the program named
//! (the evaluator's workers take the largest programs first):
//! K1, a shadowed reference renamed anyway (`rw_var` asking the unit's
//! functions before the scopes): here and natively
//! examples/06_structured.elya (`step`, a parameter of `elya/async`, renamed
//! `elya.async.step`).
//! K2, the R3 filter dropped (`r3` keeping every diagnostic): here
//! delete-token-12332-from-selfhost/parse.elya, natively selfhost/check.elya
//! (E0201 for `parse.parse_tree` after E0600 for `import parse`).
//! K3, the two passes' diagnostics in the other order: the same two
//! programs (resolution's E0200 printed before the parse and module
//! diagnostics).
//! (Rerun 2026-11-27 on the evaluator's subset, below: all three caught.)
//!
//! The evaluator runs on every multi-file and focused program and on the
//! corpus programs with something to resolve (an import, a module or
//! resolution error), none over 100 KB; the native twin on everything. The
//! review measured the subset keeping all 330 mutants it killed, at half the
//! bytes (1,123 s in the gate's configuration A before).
//! The focused set has a program for each besides (shadowing by every kind
//! of binder; a refused name; both passes in one program).

#[path = "support/selfhost_parse.rs"]
mod parse_support;
#[path = "support/selfhost_resolve.rs"]
mod support;

use elya::eval::{Host, World};
use std::time::Instant;

/// The resolver with its entry point replaced by a loop over its arguments
/// (the standard modules' directory first, then each root): one front end
/// per worker.
fn batch_source(resolver: &str) -> String {
    let main = resolver
        .rfind("pub fn main(")
        .expect("the resolver has a public main");
    format!(
        "{}\nfn selfhost_test_batch_loop(i: Int, n: Int) / {{IO}} {{\n  if i == n {{ Unit }} else {{\n    let dump = resolve_dump(io.arg(i), io.arg(0))\n    io.println(string.slice(dump, 0, string.length(dump) - 1))\n    selfhost_test_batch_loop(i + 1, n)\n  }}\n}}\npub fn main() / {{IO}} {{ selfhost_test_batch_loop(1, io.arg_count()) }}\n",
        &resolver[..main]
    )
}

fn front_end(source: &str) -> elya::ast::Module {
    let (module, diagnostics, sources) =
        elya::front_end_with("selfhost/resolve.elya", source, &mut support::reader);
    assert!(
        diagnostics.is_empty(),
        "resolver front end: {}",
        elya::diag::render(&diagnostics, &sources)
    );
    module
}

/// D3: each table in the Elya program equals its Rust twin. The builtin
/// modules and the standard modules are private to src/modules.rs: they
/// are read off E0600's help (every module, builtin first) and E0604 (a
/// builtin module keeps its name).
#[test]
fn the_builtin_tables_are_their_rust_twins() {
    let module = front_end(&support::resolver_source());
    let host = Host {
        args: vec!["-".into(), "-".into(), "--tables".into()],
        ..Host::default()
    };
    let (interpreter, result) = elya::eval::run_module_keeping_host(&module, World::Live, host);
    assert!(result.is_ok(), "{result:?}");
    let session = elya::Session::new();
    let (_, diags) = elya::resolved(&session, "main.elya", "import elya/zz\n", &mut |_| None);
    let help = &diags[0].helps[0];
    let all: Vec<&str> = help
        .strip_prefix("the standard modules are ")
        .expect("E0600's help lists the modules")
        .split(", ")
        .map(|m| m.trim_matches('`'))
        .collect();
    let (builtin, std): (Vec<&str>, Vec<&str>) = all.iter().partition(|m| {
        let (_, d) = elya::resolved(
            &session,
            "main.elya",
            &format!("import {m} as zz\n"),
            &mut |_| None,
        );
        d.iter()
            .any(|d| d.code == "E0604" && d.message.contains("keeps its name"))
    });
    let line = |name: &str, xs: &[&str]| format!("{name}:{}\n", xs.join(" "));
    let builtin: Vec<&str> = builtin
        .iter()
        .map(|m| m.strip_prefix("elya/").unwrap())
        .collect();
    let want = [
        line("builtins", elya::resolve::builtins()),
        line("INT_BINARY_BUILTINS", elya::resolve::INT_BINARY_BUILTINS),
        line("FLOAT_CONVERSIONS", elya::resolve::FLOAT_CONVERSIONS),
        line("STRING_BUILTINS", elya::resolve::STRING_BUILTINS),
        line("BUFFER_BUILTINS", elya::resolve::BUFFER_BUILTINS),
        line("ARRAY_BUILTINS", elya::resolve::ARRAY_BUILTINS),
        line("IO_BUILTINS", elya::resolve::IO_BUILTINS),
        line("BUILTIN_MODULES", &builtin),
        line("STD_SOURCES", &std),
    ]
    .concat();
    parse_support::assert_dump("the builtin tables", interpreter.output(), &want);
}

#[test]
fn the_resolver_in_elya_agrees() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(evaluator_proof)
        .expect("evaluator resolver test thread")
        .join()
        .unwrap();
}

fn evaluator_proof() {
    let start = Instant::now();
    let root = parse_support::root();
    let corpus = support::corpus_programs(&root);
    let multi = support::multi_file_programs(&root);
    let focused = support::focused();
    assert!(multi.len() >= 100, "multi-file programs: {}", multi.len());
    assert!(focused.len() >= 60, "focused programs: {}", focused.len());
    let mut by_file = std::collections::BTreeMap::new();
    for p in &multi {
        let file = p.name.split(' ').next().unwrap_or("").to_string();
        *by_file.entry(file).or_insert(0) += 1;
    }
    println!(
        "RESOLVE multi-file programs by test file: {by_file:?}; found in {:?}",
        start.elapsed()
    );
    let programs: Vec<_> = corpus
        .iter()
        .chain(&multi)
        .chain(&focused)
        .cloned()
        .collect();
    let scratch = parse_support::Scratch::new("resolve-evaluator");
    let std = root.join("src/std");
    let mut cases = Vec::new();
    let mut codes = Vec::new();
    let mut kept = Vec::new();
    for (i, p) in programs.iter().enumerate() {
        let path = support::materialise(scratch.path(), i, p);
        let (want, cs) = support::oracle(&path);
        // The evaluator runs on every multi-file and focused program, and on
        // the corpus programs with something to resolve: a module or
        // resolution error, or an import; and none over 100 KB (the
        // self-hosted sources and their variants). The native twin runs the
        // pass on everything (the review's measure: the subset keeps every
        // mutant killed, at half the bytes).
        let resolving =
            cs.iter().any(|c| support::CODES.contains(&c.as_str())) || want.contains("Import");
        if i >= corpus.len() || (p.root.len() <= 100_000 && resolving) {
            kept.push(i);
        }
        codes.push(cs);
        cases.push((path, want));
    }
    let programs: Vec<_> = kept.iter().map(|&i| programs[i].clone()).collect();
    let corpus_kept = kept.iter().filter(|&&i| i < corpus.len()).count();
    let all_codes = codes.clone();
    let codes: Vec<_> = kept.iter().map(|&i| codes[i].clone()).collect();
    let cases: Vec<_> = kept.iter().map(|&i| cases[i].clone()).collect();
    println!("RESOLVE oracles in {:?}", start.elapsed());
    let focused_codes = support::count_codes(&codes[corpus_kept + multi.len()..]);
    for (code, n) in &focused_codes {
        assert!(*n > 0, "the focused set misses {code}");
    }
    let resolver = batch_source(&support::resolver_source());
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    let mut order: Vec<_> = (0..cases.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(programs[i].root.len() + cases[i].1.len()));
    let mut groups = vec![Vec::new(); workers];
    let mut weights = vec![0usize; workers];
    for index in order {
        let worker = (0..workers).min_by_key(|&i| weights[i]).unwrap();
        weights[worker] += programs[index].root.len() + cases[index].1.len();
        groups[worker].push(index);
    }
    let compared = std::thread::scope(|scope| {
        let mut threads = Vec::new();
        for group in groups {
            let (resolver, cases, programs, std) = (&resolver, &cases, &programs, &std);
            threads.push(
                std::thread::Builder::new()
                    .stack_size(1024 * 1024 * 1024)
                    .spawn_scoped(scope, move || {
                        let module = front_end(resolver);
                        let mut args = vec![support::elya_path(std).into()];
                        for &i in &group {
                            args.push(support::elya_path(&cases[i].0).into());
                        }
                        let host = Host {
                            args,
                            ..Host::default()
                        };
                        let (interpreter, result) =
                            elya::eval::run_module_keeping_host(&module, World::Live, host);
                        let actual = interpreter.output();
                        let mut at = 0;
                        for &i in &group {
                            let want = &cases[i].1;
                            let end = at + want.len();
                            let got = actual
                                .get(at..end)
                                .unwrap_or_else(|| actual.get(at..).unwrap_or(""));
                            parse_support::assert_dump(
                                &format!("{}; evaluator result: {result:?}", programs[i].name),
                                got,
                                want,
                            );
                            at = end;
                        }
                        parse_support::assert_dump(
                            "unexpected output after the last program",
                            actual.get(at..).unwrap_or(""),
                            "",
                        );
                        assert!(result.is_ok(), "{result:?}");
                        println!(
                            "RESOLVE worker compared={} elapsed={:?}",
                            group.len(),
                            start.elapsed()
                        );
                        group.len()
                    })
                    .expect("evaluator resolver worker"),
            );
        }
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .sum::<usize>()
    });
    assert_eq!(compared, cases.len(), "every program is compared once");
    let errors = support::count_codes(&all_codes);
    let with_error = all_codes
        .iter()
        .filter(|cs| cs.iter().any(|c| support::CODES.contains(&c.as_str())))
        .count();
    println!(
        "RESOLVE corpus={} (the evaluator on {corpus_kept}) multi-file={} focused={} total={compared}; programs with a module or resolution error={with_error}; by code (programs): {errors:?}; focused by code: {focused_codes:?}; elapsed={:?}",
        corpus.len(),
        multi.len(),
        focused.len(),
        start.elapsed()
    );
}
