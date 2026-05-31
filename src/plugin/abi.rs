//! Rust-side mirror of the C ABI declared in
//! `installers/plugin-abi/ai_hands_plugin.h`.
//!
//! Keep these declarations in lock-step with the header — any change here
//! that affects struct layout, enum values, or function-pointer signatures
//! is a major-version bump on both sides.
//!
//! Phase 1 (this commit): types only. No callers yet — the registry and
//! loader stubs live in sibling modules. Phase 2 will add the `libloading`
//! dep and use these function-pointer types to resolve symbols from the
//! dynamic library.

#![allow(dead_code)] // phase 1 — public ABI surface, callers come in phase 2

use std::ffi::c_char;

/// Mirrors `AI_HANDS_PLUGIN_ABI_VERSION_MAJOR` in the C header. Bump on
/// breaking changes (struct layout, enum reassignment, signature changes).
pub const ABI_VERSION_MAJOR: u32 = 1;

/// Mirrors `AI_HANDS_PLUGIN_ABI_VERSION_MINOR` in the C header. Bump on
/// purely additive changes (new optional symbols, appended status codes).
pub const ABI_VERSION_MINOR: u32 = 0;

/// Pack major/minor the same way the C macro does:
/// `(major << 16) | (minor & 0xFFFF)`.
pub const fn pack_abi_version(major: u32, minor: u32) -> u32 {
    (major << 16) | (minor & 0xFFFF)
}

/// Status codes returned by `ai_hands_plugin_call`. Values must match the
/// `AiHandsStatus` enum in the C header byte-for-byte.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    ErrUnknownTool = 1,
    ErrInvalidArgs = 2,
    ErrInternal = 3,
    ErrTimeout = 4,
    ErrNotImplemented = 5,
}

/// Mirror of `AiHandsToolDescriptor`. The host treats every pointer as
/// borrowed from the plugin for the plugin's lifetime — we never free
/// these from the Rust side.
#[repr(C)]
pub struct ToolDescriptor {
    pub name: *const c_char,
    pub description: *const c_char,
    pub input_schema_json: *const c_char,
}

/// Mirror of `AiHandsPluginInfo`. Same ownership rules as `ToolDescriptor`.
#[repr(C)]
pub struct PluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub author: *const c_char,
    pub description: *const c_char,
    pub tool_count: u32,
    pub tools: *const ToolDescriptor,
}

// ----- Function pointer types for the 5 plugin entry points. -----
//
// Phase 2 will use `libloading::Library::get` with these types to resolve
// symbols out of the loaded dynamic library. Keeping the typedefs here
// keeps the ABI definition co-located with the rest of the surface.

/// `uint32_t ai_hands_plugin_abi_version(void);`
pub type AbiVersionFn = unsafe extern "C" fn() -> u32;

/// `const AiHandsPluginInfo* ai_hands_plugin_init(void);`
pub type PluginInitFn = unsafe extern "C" fn() -> *const PluginInfo;

/// `int32_t ai_hands_plugin_call(const char*, const char*, char**);`
pub type PluginCallFn = unsafe extern "C" fn(
    tool_name: *const c_char,
    input_json: *const c_char,
    output_json: *mut *mut c_char,
) -> i32;

/// `void ai_hands_plugin_free_string(char*);`
pub type PluginFreeStringFn = unsafe extern "C" fn(*mut c_char);

/// `void ai_hands_plugin_shutdown(void);`
pub type PluginShutdownFn = unsafe extern "C" fn();

// ----- Symbol names exported by every plugin. -----

pub const SYM_ABI_VERSION: &[u8] = b"ai_hands_plugin_abi_version\0";
pub const SYM_INIT: &[u8] = b"ai_hands_plugin_init\0";
pub const SYM_CALL: &[u8] = b"ai_hands_plugin_call\0";
pub const SYM_FREE_STRING: &[u8] = b"ai_hands_plugin_free_string\0";
pub const SYM_SHUTDOWN: &[u8] = b"ai_hands_plugin_shutdown\0";

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn abi_version_constants_match_expected() {
        assert_eq!(ABI_VERSION_MAJOR, 1, "phase 1 ships ABI v1");
        assert_eq!(ABI_VERSION_MINOR, 0, "phase 1 ships ABI v1.0");
        assert_eq!(
            pack_abi_version(ABI_VERSION_MAJOR, ABI_VERSION_MINOR),
            (1u32 << 16),
            "pack_abi_version must match the C macro"
        );
        assert_eq!(pack_abi_version(2, 3), (2u32 << 16) | 3);
    }

    #[test]
    fn status_repr_is_c() {
        // `#[repr(C)]` enums with explicit discriminants must round-trip
        // through i32 cleanly. This guards against accidental reordering.
        assert_eq!(Status::Ok as i32, 0);
        assert_eq!(Status::ErrUnknownTool as i32, 1);
        assert_eq!(Status::ErrInvalidArgs as i32, 2);
        assert_eq!(Status::ErrInternal as i32, 3);
        assert_eq!(Status::ErrTimeout as i32, 4);
        assert_eq!(Status::ErrNotImplemented as i32, 5);
    }

    #[test]
    fn structs_are_pod_friendly() {
        // Compile-time check that the descriptor structs are the size we
        // expect — three pointer fields for the tool descriptor, four
        // pointer fields + a u32 + a pointer for the plugin info. If
        // anyone accidentally adds a field without bumping ABI_VERSION_MAJOR
        // this test will catch it.
        let ptr_sz = size_of::<*const c_char>();
        assert_eq!(
            size_of::<ToolDescriptor>(),
            ptr_sz * 3,
            "ToolDescriptor must be exactly 3 pointer fields (ABI break otherwise)"
        );
        // PluginInfo: 4 pointers + u32 + (padding to align) + pointer.
        // Just sanity-check it's at least 4 ptr + 4 bytes + 1 ptr.
        assert!(
            size_of::<PluginInfo>() >= ptr_sz * 5 + 4,
            "PluginInfo size shrank — ABI break"
        );
    }

    #[test]
    fn symbol_names_are_nul_terminated() {
        for sym in [
            SYM_ABI_VERSION,
            SYM_INIT,
            SYM_CALL,
            SYM_FREE_STRING,
            SYM_SHUTDOWN,
        ] {
            assert_eq!(
                sym.last().copied(),
                Some(0),
                "plugin symbol names must be NUL-terminated for libloading"
            );
        }
    }
}
