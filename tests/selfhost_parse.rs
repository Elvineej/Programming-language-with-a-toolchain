// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! Byte-for-byte parser parity on clean, malformed and token-deleted inputs.
//! The oracle is the recovered tree followed by lexing and parsing diagnostics.

#[path = "support/selfhost_parse.rs"]
mod support;

use elya::eval::{Host, World};
use std::path::Path;
use std::time::Instant;

#[test]
fn repository_and_embedded_programs_agree() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(evaluator_proof)
        .expect("evaluator parser test thread")
        .join()
        .unwrap();
}

// Only the IO entry point is replaced for this proof. The parser functions and
// public parse_dump remain the exact shipped source. Running one interpreter
// per worker avoids reconstructing its function/constructor tables for every
// tiny source. The native suite separately runs the actual main on every case.
fn batch_source(parser: &str, count: usize) -> String {
    let main = parser
        .rfind("pub fn main(")
        .expect("parser has a public main");
    assert!(!parser[..main].contains("fn selfhost_test_batch_loop("));
    format!(
        "{}\nfn selfhost_test_batch_loop(i: Int) / {{IO}} {{\n  if i == {count} {{ Unit }} else {{\n    let dump = parse_dump(io.read_file(io.arg(i)))\n    io.println(string.slice(dump, 0, string.length(dump) - 1))\n    selfhost_test_batch_loop(i + 1)\n  }}\n}}\npub fn main() / {{IO}} {{ selfhost_test_batch_loop(0) }}\n",
        &parser[..main]
    )
}

fn evaluator_proof() {
    let start = Instant::now();
    let parser = support::parser_source();
    let corpus = support::parity_corpus(Path::new(env!("CARGO_MANIFEST_DIR")));
    assert!(corpus.embedded >= 60, "at least 60 clean embedded programs");
    assert!(
        corpus.broken_embedded > 0,
        "malformed embedded programs are collected"
    );
    corpus.report();
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    let mut order: Vec<_> = (0..corpus.cases.len()).collect();
    order.sort_by_key(|&i| {
        std::cmp::Reverse(corpus.cases[i].text.len() + corpus.cases[i].dump.len())
    });
    let mut groups = vec![Vec::new(); workers];
    let mut weights = vec![0usize; workers];
    for index in order {
        let worker = (0..workers).min_by_key(|&i| weights[i]).unwrap();
        weights[worker] += corpus.cases[index].text.len() + corpus.cases[index].dump.len();
        groups[worker].push(index);
    }
    let mut assigned: Vec<_> = groups.iter().flatten().copied().collect();
    assigned.sort_unstable();
    assert_eq!(
        assigned,
        (0..corpus.cases.len()).collect::<Vec<_>>(),
        "every corpus input is assigned exactly once"
    );
    let compared = std::thread::scope(|scope| {
        let mut threads = Vec::new();
        for (worker, group) in groups.into_iter().enumerate() {
            let (parser, cases) = (&parser, &corpus.cases);
            threads.push(
                std::thread::Builder::new()
                    .stack_size(1024 * 1024 * 1024)
                    .spawn_scoped(scope, move || {
                        let worker_start = Instant::now();
                        let source = batch_source(parser, group.len());
                        let (module, diagnostics, sources) = elya::front_end_with(
                            "selfhost/parse.elya",
                            &source,
                            &mut support::reader,
                        );
                        assert!(
                            diagnostics.is_empty(),
                            "parser front end: {}",
                            elya::diag::render(&diagnostics, &sources)
                        );
                        let scratch = support::Scratch::new(&format!("evaluator-{worker}"));
                        let mut args = Vec::new();
                        for &index in &group {
                            let path = scratch.path().join(format!("input-{index}.elya"));
                            std::fs::write(&path, &cases[index].text)
                                .expect("write evaluator input");
                            args.push(path.into_os_string());
                        }
                        let host = Host {
                            args,
                            ..Host::default()
                        };
                        let (interpreter, result) =
                            elya::eval::run_module_keeping_host(&module, World::Live, host);
                        let actual = interpreter.output();
                        let mut at = 0;
                        for &index in &group {
                            let case = &cases[index];
                            let end = at + case.dump.len();
                            let got = actual
                                .get(at..end)
                                .unwrap_or_else(|| actual.get(at..).unwrap_or(""));
                            support::assert_dump(
                                &format!(
                                    "{}; evaluator result: {result:?}",
                                    support::case_label(case)
                                ),
                                got,
                                &case.dump,
                            );
                            at = end;
                        }
                        support::assert_dump(
                            "unexpected evaluator output after final input",
                            actual.get(at..).unwrap_or(""),
                            "",
                        );
                        assert!(result.is_ok(), "worker {worker}: {result:?}");
                        println!(
                            "EVALUATOR worker={worker} compared={} elapsed={:?}",
                            group.len(),
                            worker_start.elapsed()
                        );
                        group.len()
                    })
                    .expect("evaluator parser worker"),
            );
        }
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .sum::<usize>()
    });
    assert_eq!(
        compared,
        corpus.cases.len(),
        "every corpus input is compared exactly once"
    );
    println!("EVALUATOR compared={compared} workers={workers} elapsed={:?} (including batched IO entry points, front ends, corpus and files)", start.elapsed());
}
