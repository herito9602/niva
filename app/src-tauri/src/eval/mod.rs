//! `niva-eval` (task 2.10): the E1 tool-choice evaluation, run through the
//! real assistant planning path against a real Ollama.
//!
//! Compiled only with the `eval` cargo feature: it needs the fakes of
//! `niva-core` (`testing`) and the YAML reader, and neither may ship in the
//! app. `bin/niva-eval.rs` is a thin `main` over [`cli::run`].

pub mod cases;
pub mod cli;
pub mod lang;
pub mod matcher;
pub mod report;
pub mod runner;
