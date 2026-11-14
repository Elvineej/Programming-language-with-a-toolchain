//! The programs of `tests/buffer.rs`, shared with
//! `crates/codegen/tests/native_buffer.rs` (no tests here).

/// Text built by a buffer: empty, one push, across the first growth (64
/// bytes), non-ASCII, and through a function and a handler (CPS code natively).
pub const BUILDS: &str = r#"effect Note { fn note(s: String) -> Unit }
fn repeat(b: Buffer, s: String, n: Int) -> Buffer {
  if n == 0 { b } else { repeat(buffer.push(b, s), s, n - 1) }
}
fn noted(b: Buffer) -> Buffer {
  let _ = note("in a handler")
  buffer.push(b, "!")
}
pub fn main() -> Int {
  let empty = buffer.finish(buffer.new())
  io.println("[" <> empty <> "]")
  io.println(buffer.finish(buffer.push(buffer.new(), "one")))
  // 64 bytes: the finished string's slot is the buffer's size, so a buffer
  // not kept while its text is copied out is reused under it.
  io.println(buffer.finish(buffer.push(buffer.new(), "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_+=")))
  let long = buffer.finish(repeat(buffer.new(), "0123456789", 13))
  io.println(long)
  io.println(buffer.finish(repeat(buffer.push(buffer.new(), "é"), "ünï→", 40)))
  let h = handle { buffer.finish(noted(buffer.push(buffer.new(), "handled"))) } with {
    Note.note(s) -> { io.println(s)  resume(Unit) }
  }
  io.println(h)
  string.length(long)
}
"#;

pub fn builds_out() -> String {
    format!(
        "[]\none\nabcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_+=\n{}\né{}\nin a handler\nhandled!\n",
        "0123456789".repeat(13),
        "ünï→".repeat(40)
    )
}

/// `n` appends of ten bytes, by a buffer: the measurement of spec D5.
pub fn appends(n: u32) -> String {
    format!(
        "fn build(b: Buffer, n: Int) -> Buffer {{\n  if n == 0 {{ b }} else {{ build(buffer.push(b, \"0123456789\"), n - 1) }}\n}}\n\
         pub fn main() -> Int {{ string.length(buffer.finish(build(buffer.new(), {n}))) }}\n"
    )
}
