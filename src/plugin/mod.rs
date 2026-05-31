//! Plugin system — heavyweight C-FFI loader path.
//!
//! Phase 1 (this commit): ABI types, registry stub, loader stub, MCP tool
//! schemas. No actual plugin loading; load attempts return
//! `LoadError::PhaseNotImplemented`.
//!
//! Phase 2 (future commit): add `libloading = "0.8"` dep and wire
//! `Library::new` + symbol resolution + lifecycle management.
//!
//! Plugin authors should target the C ABI in
//! `installers/plugin-abi/ai_hands_plugin.h`. See that header for the
//! contract (5 entry points, thread-safety, version-skew policy).

pub mod abi;
pub mod loader;
pub mod registry;
