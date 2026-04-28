//! Script templates — pre-built multi-step workflows.
//!
//! Templates generate `hands_script` step arrays — they do not execute anything
//! directly. The caller feeds the output to `hands_script` for execution.

pub mod login;
