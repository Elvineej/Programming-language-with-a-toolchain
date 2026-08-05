//! The Lyra compiler (Slice 1: tree-walking interpreter).

pub mod span;
pub mod diag;
pub mod lex;
pub mod ast;
pub mod parse;
pub mod resolve;
pub mod eval;

/// Shared, explicitly-threaded compiler state. No globals live outside this.
#[derive(Debug, Default)]
pub struct Session {}

impl Session {
    pub fn new() -> Self {
        Session {}
    }
}
