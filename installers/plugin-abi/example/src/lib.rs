//! Example AI-Hands plugin.
//!
//! Exports the 5 entry points required by the C-FFI ABI (see
//! `installers/plugin-abi/ai_hands_plugin.h` in the AI-Hands repo) and
//! exposes a single tool, `example_echo`, that round-trips a `message`
//! field. Build with `cargo build --release` and look in `target/release/`
//! for the produced `.dll`/`.so`/`.dylib`.
//!
//! This crate is intentionally dependency-free: it only uses `std`. That
//! makes it the cleanest demonstration of what a plugin actually has to
//! do — no JSON crate, no async runtime, just NUL-terminated strings and
//! a few `extern "C"` functions.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, CStr, CString};
use std::ptr;

// ---------- ABI constants — must match ai_hands_plugin.h ----------

const ABI_VERSION_MAJOR: u32 = 1;
const ABI_VERSION_MINOR: u32 = 0;

const STATUS_OK: i32 = 0;
const STATUS_ERR_UNKNOWN_TOOL: i32 = 1;
const STATUS_ERR_INVALID_ARGS: i32 = 2;

// ---------- Tool descriptors (kept alive for the plugin's lifetime) ----------
//
// The ABI says the host borrows these pointers — never frees them. We
// satisfy that by stashing CStrings in a process-lifetime static, then
// handing out the raw pointers in the descriptor.

#[repr(C)]
pub struct ToolDescriptor {
    pub name: *const c_char,
    pub description: *const c_char,
    pub input_schema_json: *const c_char,
}

#[repr(C)]
pub struct PluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub author: *const c_char,
    pub description: *const c_char,
    pub tool_count: u32,
    pub tools: *const ToolDescriptor,
}

// We need to put `ToolDescriptor` and `PluginInfo` instances behind a
// `static` so their addresses are stable for the plugin's lifetime, but
// `static` items must be `Sync` and raw pointers aren't `Sync`/`Send` by
// default. Wrap the descriptors in a newtype with `unsafe impl Sync` —
// the pointers themselves point into permanently-leaked CStrings, so
// they're trivially safe to share across threads (read-only, no aliasing
// hazards).
#[repr(transparent)]
struct StaticDescriptor(ToolDescriptor);
#[repr(transparent)]
struct StaticInfo(PluginInfo);

// SAFETY: the pointers inside `ToolDescriptor` and `PluginInfo` point into
// `Box::leak`ed `CString`s and a `Box::leak`ed array of descriptors, all
// of which live for the rest of the process. The structs are read-only
// from every thread, so sharing them is safe.
//
// We need both Send (so OnceLock<T>: Sync requires T: Send + Sync) and
// Sync. Both are safe here because the contents are immutable static data.
unsafe impl Send for StaticDescriptor {}
unsafe impl Sync for StaticDescriptor {}
unsafe impl Send for StaticInfo {}
unsafe impl Sync for StaticInfo {}

use std::sync::OnceLock;

static DESCRIPTORS: OnceLock<&'static [StaticDescriptor]> = OnceLock::new();
static INFO: OnceLock<StaticInfo> = OnceLock::new();

fn leak_cstr(s: &str) -> *const c_char {
    // Leak a CString so its buffer lives for the rest of the process.
    // The ABI requires these pointers to remain valid until shutdown.
    let boxed = CString::new(s).expect("plugin string contains an interior NUL").into_boxed_c_str();
    Box::leak(boxed).as_ptr()
}

fn descriptors() -> &'static [StaticDescriptor] {
    DESCRIPTORS.get_or_init(|| {
        let arr: Box<[StaticDescriptor]> = Box::new([StaticDescriptor(ToolDescriptor {
            name: leak_cstr("example_echo"),
            description: leak_cstr(
                "Echo back the `message` field as `{\"echoed\":<message>}`.",
            ),
            input_schema_json: leak_cstr(
                r#"{"type":"object","properties":{"message":{"type":"string","description":"Text to echo back"}},"required":["message"]}"#,
            ),
        })]);
        Box::leak(arr)
    })
}

fn info() -> &'static PluginInfo {
    &INFO
        .get_or_init(|| {
            let d = descriptors();
            // The descriptor array stores StaticDescriptor (newtype) but the
            // C ABI expects `*const ToolDescriptor`. The newtype is
            // `#[repr(transparent)]`-equivalent (single field, same layout
            // as ToolDescriptor since ToolDescriptor is #[repr(C)] and
            // StaticDescriptor wraps only it), so the pointer cast is sound.
            StaticInfo(PluginInfo {
                name: leak_cstr("example_plugin"),
                version: leak_cstr("0.1.0"),
                author: leak_cstr("AI-Hands"),
                description: leak_cstr("Minimal example plugin — echoes a message field."),
                tool_count: d.len() as u32,
                tools: d.as_ptr() as *const ToolDescriptor,
            })
        })
        .0
}

// ---------- Entry points ----------

#[no_mangle]
pub extern "C" fn ai_hands_plugin_abi_version() -> u32 {
    (ABI_VERSION_MAJOR << 16) | (ABI_VERSION_MINOR & 0xFFFF)
}

#[no_mangle]
pub extern "C" fn ai_hands_plugin_init() -> *const PluginInfo {
    info() as *const PluginInfo
}

/// SAFETY: `tool_name` and `input_json` must be NUL-terminated UTF-8.
/// `output_json` must be a valid `*mut *mut c_char`. The host promises
/// both per the ABI contract.
#[no_mangle]
pub unsafe extern "C" fn ai_hands_plugin_call(
    tool_name: *const c_char,
    input_json: *const c_char,
    output_json: *mut *mut c_char,
) -> i32 {
    if tool_name.is_null() || input_json.is_null() || output_json.is_null() {
        return STATUS_ERR_INVALID_ARGS;
    }

    let name = match CStr::from_ptr(tool_name).to_str() {
        Ok(s) => s,
        Err(_) => return STATUS_ERR_INVALID_ARGS,
    };
    if name != "example_echo" {
        return STATUS_ERR_UNKNOWN_TOOL;
    }

    let input = match CStr::from_ptr(input_json).to_str() {
        Ok(s) => s,
        Err(_) => return STATUS_ERR_INVALID_ARGS,
    };

    // Extremely small hand-rolled JSON parse so the example stays
    // dependency-free. Real plugins should use serde_json.
    let Some(message) = extract_string_field(input, "message") else {
        return STATUS_ERR_INVALID_ARGS;
    };

    let response = format!(r#"{{"echoed":"{}"}}"#, escape_json(&message));
    let out = match CString::new(response) {
        Ok(c) => c,
        Err(_) => return STATUS_ERR_INVALID_ARGS,
    };
    *output_json = out.into_raw();
    STATUS_OK
}

/// SAFETY: `s` must be a pointer previously returned from this plugin via
/// `ai_hands_plugin_call`'s `output_json`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn ai_hands_plugin_free_string(s: *mut c_char) {
    if !s.is_null() {
        // Reclaim the CString we leaked via `into_raw` so the allocator
        // can recycle the buffer.
        drop(CString::from_raw(s));
    }
}

#[no_mangle]
pub extern "C" fn ai_hands_plugin_shutdown() {
    // No-op: our only state is the OnceLock-owned static strings, which
    // live until process exit by design. A real plugin would release any
    // open file handles, sockets, threads, etc. here.
}

// ---------- helpers ----------

/// Tiny JSON-ish parser: find `"key"\s*:\s*"value"` and return value with
/// minimal unescaping. Good enough for the example; do NOT copy this into
/// production code. Use serde_json instead.
fn extract_string_field(src: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let after_key = src.split_once(&needle[..])?.1;
    let after_colon = after_key.split_once(':')?.1.trim_start();
    let after_open = after_colon.strip_prefix('"')?;
    // Find first un-escaped closing quote.
    let mut out = String::new();
    let mut iter = after_open.chars().peekable();
    while let Some(ch) = iter.next() {
        match ch {
            '"' => return Some(out),
            '\\' => match iter.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// silence the unused-import warning on platforms where ptr isn't needed
// inside the OnceLock paths above (it is referenced in null-checks).
#[allow(dead_code)]
fn _unused_ptr_marker() -> *const u8 {
    ptr::null()
}
