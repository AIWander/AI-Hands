//! Plugin loader.
//!
//! Phase 1 (this commit): stub that validates the path exists and otherwise
//! returns `LoadError::PhaseNotImplemented`. The point of shipping the
//! stub is so callers (the MCP `hands_plugin_load` handler) get a clear,
//! actionable error message instead of a panic or a `todo!()`, and so the
//! error-shape is locked in before phase 2 touches it.
//!
//! `AbiVersionMismatch`, `SymbolMissing`, and `InitFailed` aren't returned
//! by the phase-1 stub but exist now so the public error surface is locked
//! in. Allow the dead-code warning for the phase-1 variants.

#![allow(dead_code)]
//!
//! Phase 2 will add the `libloading = "0.8"` crate and wire:
//!   1. `libloading::Library::new(path)` — `LoadLibrary` on Windows,
//!      `dlopen` on Unix.
//!   2. `lib.get::<AbiVersionFn>(abi::SYM_ABI_VERSION)` to read the
//!      plugin's ABI version. Reject on major mismatch with
//!      `abi::ABI_VERSION_MAJOR`.
//!   3. `lib.get::<PluginInitFn>(abi::SYM_INIT)` and call it. Walk the
//!      returned `PluginInfo` + tool descriptors, converting the C
//!      strings into owned `String`s for `LoadedPlugin`/`LoadedTool`.
//!   4. Store the `Library` handle inside the registry entry alongside
//!      the cached `PluginCallFn`/`PluginFreeStringFn`/`PluginShutdownFn`
//!      pointers (phase 2 will extend `LoadedPlugin` with these).
//!   5. Insert into `registry`.
//!
//! Phase 2 will also need to handle:
//!   - Panic-safety around `PluginInitFn` (catch_unwind across FFI).
//!   - Calling `PluginShutdownFn` on unload via a Drop wrapper on the
//!     `Library` handle, so the library outlives every cached fn ptr.

use super::registry::LoadedPlugin;

/// Errors returned by `load_from_path`. Phase 2 will add more variants
/// (e.g. `LibraryOpenFailed`, `InitReturnedNull`) but the existing
/// variants are part of the host's public error surface and must not
/// be reordered.
#[derive(Debug)]
pub enum LoadError {
    /// Phase 1 sentinel — the wiring isn't in place yet. Callers should
    /// surface this as a clear "phase 2 will add this" message.
    PhaseNotImplemented,
    /// The provided path does not exist on disk.
    PathNotFound(String),
    /// Plugin's reported major version does not match the host's.
    AbiVersionMismatch { expected: u32, actual: u32 },
    /// One of the required `ai_hands_plugin_*` symbols was not exported.
    SymbolMissing(String),
    /// `ai_hands_plugin_init` returned NULL or otherwise signaled failure.
    InitFailed(String),
}

/// Attempt to load a plugin from a dynamic-library path.
///
/// Phase 1 behavior:
///   - Validates the path exists. If not, returns `PathNotFound`.
///   - Otherwise returns `PhaseNotImplemented` so the caller can show a
///     clear "wiring lands in phase 2" message rather than panicking.
pub fn load_from_path(path: &str) -> Result<LoadedPlugin, LoadError> {
    if !std::path::Path::new(path).exists() {
        return Err(LoadError::PathNotFound(path.to_string()));
    }
    Err(LoadError::PhaseNotImplemented)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn load_from_path_returns_path_not_found_for_missing_path() {
        // Pick a path that's vanishingly unlikely to exist.
        let bogus =
            r"C:\__definitely_not_a_real_plugin_path__\plugin_phase1_stub_does_not_exist.dll";
        match load_from_path(bogus) {
            Err(LoadError::PathNotFound(p)) => assert_eq!(p, bogus),
            other => panic!("expected PathNotFound, got {:?}", other),
        }
    }

    #[test]
    fn load_from_path_returns_phase_not_implemented_for_existing_path() {
        // Create a real file that isn't actually a plugin. The phase 1
        // stub doesn't try to dlopen it — it only checks existence and
        // then returns PhaseNotImplemented, which is exactly what we want
        // to verify so phase 2 can replace the inner body without
        // changing the error contract for missing paths.
        let mut f = tempfile::NamedTempFile::new().expect("tempfile create");
        writeln!(f, "this is not actually a plugin").unwrap();
        let path = f.path().to_string_lossy().into_owned();

        match load_from_path(&path) {
            Err(LoadError::PhaseNotImplemented) => {}
            other => panic!("expected PhaseNotImplemented, got {:?}", other),
        }
    }

    #[test]
    fn load_error_variants_are_debug_formattable() {
        // Each variant must be Debug-printable so the MCP handler can
        // surface unknown failures via `format!("{:?}", e)`. If anyone
        // adds a non-Debug field this test breaks immediately.
        let variants = [
            LoadError::PhaseNotImplemented,
            LoadError::PathNotFound("x".into()),
            LoadError::AbiVersionMismatch {
                expected: 1,
                actual: 2,
            },
            LoadError::SymbolMissing("ai_hands_plugin_init".into()),
            LoadError::InitFailed("returned null".into()),
        ];
        for v in &variants {
            let s = format!("{:?}", v);
            assert!(!s.is_empty());
        }
    }
}
