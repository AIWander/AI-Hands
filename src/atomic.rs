//! Compile-time-safe atomic tool handles for the hands meta-tool layer.
//!
//! # Problem (Phase C)
//! Meta-tools dispatched to UIA/browser primitives via
//! `uia_lib::handle_tool_call("uia_tool_name_string", args)`.
//! A misspelled string produced a runtime "Unknown tool" error —
//! three concrete Phase C bugs:
//!   1. `hands_app_action` — "Unknown tool: uia_app_launch" (name mismatch)
//!   2. `hands_scan_qr` — RwLock-poison crash on wrong tool routing
//!   3. `hands_login_recovery` — nested meta→meta dispatch deadlock (3000ms timeout)
//!
//! # Solution (Phase D)
//! Replace all string literals in meta-tool dispatch with **typed zero-sized
//! structs (ZSTs)**. Each struct represents exactly one canonical tool name,
//! baked into its `AtomicTool` impl — not scattered across every call site.
//!
//! A typo like `UiaFocusWIndow.call(args)` → **compiler error** (no such type).
//! The only place a string can be wrong is inside this file, in the macro call.
//!
//! # Design: `dyn AtomicTool` vs generics
//! `AtomicTool` is object-safe: `fn call(&self, args: &Value) -> Value`.
//!
//! - **Concrete ZSTs (production):** call sites write `UiaFocusWindow.call(args)`.
//!   ZSTs have zero size and zero runtime overhead — the compiler inlines the
//!   string constant and calls `uia_lib::handle_tool_call` directly.
//! - **`dyn AtomicTool` (testing/mocking):** any `Box<dyn AtomicTool>` works.
//!   Adds one vtable indirection per call — acceptable for test doubles.
//! - **Generics `T: AtomicTool`:** monomorphized at compile time, but makes
//!   function signatures verbose. Not needed for our call pattern (all sites
//!   use concrete types).
//!
//! Decision: concrete ZSTs at all production call sites; trait exists for mocks.
//!
//! # Adding a new atomic tool
//! ```rust
//! uia_tool!(UiaNewTool, "uia_new_tool");
//! ```
//! That's it. The type is then usable anywhere as `UiaNewTool.call(args)`.

use serde_json::Value;

// ══════════════════════════════════════════════════════════════
// AtomicTool trait
// ══════════════════════════════════════════════════════════════

/// Compile-time-safe atomic tool handle.
///
/// Each `impl AtomicTool` bakes exactly one canonical tool name.
/// Meta-tools reference the **type**, not the string — mismatches become
/// compile errors instead of runtime "Unknown tool" failures.
///
/// All UIA tools are synchronous (`uia_lib::handle_tool_call` returns `Value`
/// without blocking an async executor), so this trait is sync.
/// Browser/vision tools that require async are wrapped separately in
/// [`browser`] as standalone `async fn`s.
pub trait AtomicTool: Send + Sync {
    /// The canonical tool name — used for logging and diagnostics only.
    /// **Not** used in dispatch; the string is baked into the `call` body.
    #[allow(dead_code)]
    fn name(&self) -> &'static str;

    /// Execute the atomic tool and return its JSON result.
    ///
    /// Equivalent to the old `uia_lib::handle_tool_call(self.name(), args)`
    /// but with the string verified at compile time through the type system.
    fn call(&self, args: &Value) -> Value;
}

// ══════════════════════════════════════════════════════════════
// Macro: generate one ZST wrapper per UIA tool name
// ══════════════════════════════════════════════════════════════

/// Generate a zero-sized `AtomicTool` wrapper for one UIA tool name.
///
/// `uia_tool!(UiaFoo, "uia_foo")` expands to a struct `UiaFoo` whose
/// `call` method calls `uia_lib::handle_tool_call("uia_foo", args)`.
/// The string `"uia_foo"` lives only inside this macro expansion.
macro_rules! uia_tool {
    ($Type:ident, $name:literal) => {
        pub struct $Type;

        impl AtomicTool for $Type {
            #[inline(always)]
            fn name(&self) -> &'static str {
                $name
            }

            #[inline(always)]
            fn call(&self, args: &Value) -> Value {
                uia_lib::handle_tool_call($name, args)
            }
        }
    };
}

// ══════════════════════════════════════════════════════════════
// UIA atomic tool handles
//
// One struct per UIA tool name used by meta-tools.
// Extend this list (and only this list) when adding new UIA calls.
//
// Note: doc-comments (///) before macro invocations are not rendered
// by rustdoc. Using // comments here instead; struct-level docs would
// require embedding them inside the macro expansion.
// ══════════════════════════════════════════════════════════════

// Focus a window by hwnd or title substring.
uia_tool!(UiaFocusWindow, "uia_focus_window");

// Send keystrokes (e.g. `{"keys": "alt+f4"}`).
uia_tool!(UiaKeyPress, "uia_key_press");

// Set window state: minimize / maximize / restore.
uia_tool!(UiaWindowState, "uia_window_state");

// Snap a window to a screen edge on a target monitor.
uia_tool!(UiaWindowSnap, "uia_window_snap");

// Find elements by role/name (used for dialog probe in app_action).
uia_tool!(UiaFind, "uia_find");

// Find elements by name/automation_id with depth limit (primary desktop search).
uia_tool!(UiaFindElement, "uia_find_element");

// Read UIA global state properties (e.g. foreground_window).
uia_tool!(UiaGetState, "uia_get_state");

// Enumerate top-level windows.
uia_tool!(UiaListWindow, "uia_list_window");

// Simulate a mouse click at coordinates or element name.
uia_tool!(UiaClick, "uia_click");

// Type text (keystroke simulation, no window-title binding).
uia_tool!(UiaType, "uia_type");

// Type text with a window-title binding (Start menu search fallback path).
uia_tool!(UiaTypeText, "uia_type_text");

// ══════════════════════════════════════════════════════════════
// Browser async tool handles
//
// Browser tools are async and require a SharedBrowser reference,
// so they cannot implement AtomicTool directly.  They are wrapped
// as typed `async fn`s here — the string is baked in, not scattered.
// ══════════════════════════════════════════════════════════════

/// Typed async wrappers for browser_mcp atomic tools.
///
/// Each function corresponds to exactly one `browser_mcp::tools::handle_tool`
/// call.  The tool-name string lives here and nowhere else.
pub mod browser {
    use browser_mcp::browser::SharedBrowser;
    use browser_mcp::types::ToolResult;
    use serde_json::Value;

    /// Capture a screenshot of the active browser page.
    ///
    /// Previously: `browser_mcp::tools::handle_tool(browser, "screenshot", args).await`
    /// Compile-time guarantee: "screenshot" can only be wrong here, not at each call site.
    #[inline]
    pub async fn screenshot(browser: &SharedBrowser, args: Value) -> ToolResult {
        browser_mcp::tools::handle_tool(browser, "screenshot", args).await
    }
}

// ══════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify each atomic tool reports the correct canonical name.
    /// This catches the common mistake of copy-pasting a macro call
    /// and forgetting to update the string.
    #[test]
    fn atomic_tool_names_are_correct() {
        assert_eq!(UiaFocusWindow.name(), "uia_focus_window");
        assert_eq!(UiaKeyPress.name(), "uia_key_press");
        assert_eq!(UiaWindowState.name(), "uia_window_state");
        assert_eq!(UiaWindowSnap.name(), "uia_window_snap");
        assert_eq!(UiaFind.name(), "uia_find");
        assert_eq!(UiaFindElement.name(), "uia_find_element");
        assert_eq!(UiaGetState.name(), "uia_get_state");
        assert_eq!(UiaListWindow.name(), "uia_list_window");
        assert_eq!(UiaClick.name(), "uia_click");
        assert_eq!(UiaType.name(), "uia_type");
        assert_eq!(UiaTypeText.name(), "uia_type_text");
    }

    /// Verify ZST size is zero (no runtime overhead).
    #[test]
    fn atomic_tools_are_zero_sized() {
        assert_eq!(std::mem::size_of::<UiaFocusWindow>(), 0);
        assert_eq!(std::mem::size_of::<UiaKeyPress>(), 0);
        assert_eq!(std::mem::size_of::<UiaClick>(), 0);
        assert_eq!(std::mem::size_of::<UiaFind>(), 0);
        assert_eq!(std::mem::size_of::<UiaFindElement>(), 0);
    }

    /// Verify AtomicTool is object-safe (supports dyn dispatch for mocking).
    #[test]
    fn atomic_tool_is_object_safe() {
        fn accepts_dyn(_tool: &dyn AtomicTool) {}
        accepts_dyn(&UiaFocusWindow);
        accepts_dyn(&UiaKeyPress);
        accepts_dyn(&UiaClick);
    }
}
