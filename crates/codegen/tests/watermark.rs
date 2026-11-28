// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! The watermark: a program Elya builds carries its author's name
//! (`elya_watermark` in runtime.c), and `elya --version` names him.

const MARK: &str =
    "Built with Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).";

#[test]
fn a_built_program_carries_the_watermark() {
    let dir = std::env::temp_dir().join(format!("elya-mark-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = "pub fn main() -> Int { 7 }\n";
    let (m, pd, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    assert!(pd.is_empty(), "front end: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let core = elya::core::lower_module(&m, &table).expect("lowers");
    let obj = dir.join("p.o");
    let exe = dir.join(format!("p{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).expect("compile_module");
    elya_codegen::link(&obj, &exe).expect("link");
    let bytes = std::fs::read(&exe).expect("read the program");
    assert!(
        bytes.windows(MARK.len()).any(|w| w == MARK.as_bytes()),
        "the program does not carry the watermark"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
