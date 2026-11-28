// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

use std::{collections::BTreeSet, fs, path::Path};

fn without_terminal_colors(diagnostic: &str) -> String {
    let mut parts = diagnostic.split("\x1b[");
    let mut plain = parts.next().unwrap().to_owned();
    for part in parts {
        let (parameters, rest) = part.split_once('m').expect("ANSI color sequence");
        assert!(parameters.bytes().all(|b| b.is_ascii_digit() || b == b';'));
        plain.push_str(rest);
    }
    plain
}

#[test]
fn guide_programs_match_their_documented_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let guide = root.join("docs/guide");
    let page = fs::read_to_string(guide.join("effects.md")).expect("read effects guide");
    let page = page.replace("\r\n", "\n");
    let mut blocks = page.split("\n```elya\n");
    let mut before = blocks.next().unwrap();
    let mut documented = BTreeSet::new();

    for block in blocks {
        let link = before.trim_end().lines().last().expect("example link");
        let (_, relative) = link.split_once("](").expect("Markdown example link");
        let relative = relative.strip_suffix(')').expect("close example link");
        let file = relative
            .strip_prefix("effects/")
            .expect("example directory");
        assert!(!file.contains('/') && file.ends_with(".elya"), "{link}");
        assert!(
            documented.insert(file.to_owned()),
            "duplicate example: {file}"
        );

        let (code, rest) = block.split_once("```\n\n").expect("close program block");
        let (fence, rest) = rest.split_once('\n').expect("open output block");
        let (expected, following) = rest.split_once("```").expect("close output block");
        before = following;

        let source = fs::read_to_string(guide.join(relative)).expect("read example");
        assert_eq!(code, source.replace("\r\n", "\n"), "source for {file}");

        let name = format!("docs/guide/{relative}");
        let run = elya::run_source_outcome(&name, &source);
        assert_eq!(run.status, None, "unexpected exit status for {file}");
        match fence {
            "```text" => {
                assert_eq!(run.error, None, "unexpected diagnostic for {file}");
                assert_eq!(run.output, expected, "stdout for {file}");
            }
            "```text stderr" => {
                assert!(run.output.is_empty(), "unexpected stdout for {file}");
                let diagnostic = run.error.expect("example should fail");
                assert_eq!(
                    without_terminal_colors(&diagnostic),
                    expected,
                    "stderr for {file}"
                );
            }
            _ => panic!("unknown output block for {file}: {fence}"),
        }
    }

    let files: BTreeSet<_> = fs::read_dir(guide.join("effects"))
        .expect("read examples directory")
        .map(|entry| {
            entry
                .expect("read directory entry")
                .file_name()
                .into_string()
                .unwrap()
        })
        .filter(|file| file.ends_with(".elya"))
        .collect();
    assert!(!files.is_empty(), "expected guide examples");
    assert_eq!(documented, files, "every example belongs in the guide");
}
