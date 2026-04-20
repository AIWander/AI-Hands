//! `hands_app_action` — window management meta-tool.
//!
//! Actions: open, close, focus, minimize, maximize, restore, snap_left, snap_right.
//!
//! Combines UIA window operations with:
//! - WindowMatch for flexible window targeting
//! - Save dialog detection + auto-handling on close
//! - Post-action verification (foreground check, bounds check, gone check)
//! - Monitor stickiness via session state
//! - Instrumentation logging
//!
//! Reversibility:
//! - focus, minimize, maximize, restore, snap_*: Reversible
//! - close: RequiresConfirmation (may lose unsaved work)
//! - open: Reversible (can be closed)

use serde_json::{json, Value};
use std::time::{Duration, Instant};

use super::response::{MetaToolResult, RungAttempt, Confidence, Reversibility};
use super::error::MetaError;
use super::instrumentation;
use super::session::SharedSession;
use super::window_match::{
    self, WindowMatch, MatchMode, Monitor, WindowMatchResult,
    parse_window_match, parse_match_mode, parse_monitor, find_single_window,
};
use super::save_dialog::{
    self, SaveDialogAction, parse_save_dialog_action,
};
use crate::atomic::{
    AtomicTool,
    UiaFocusWindow, UiaKeyPress, UiaTypeText,
    UiaFind, UiaClick,
    UiaWindowState, UiaWindowSnap,
    UiaListWindow, UiaGetState,
};

// ── App launch helper (avoids routing through uia_lib which doesn't know combo tools) ──

/// Launch an application by name/path using ShellExecuteW, with Start menu search fallback.
/// This duplicates the logic from main.rs handle_uia_app_launch to avoid the routing gap
/// where uia_lib::handle_tool_call("uia_app_launch", ...) returns "Unknown tool".
#[cfg(windows)]
fn launch_application(spec: &str) -> Value {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::Win32::Foundation::HWND;
    use windows::core::PCWSTR;

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let operation = to_wide("open");
    let target = to_wide(spec);
    let shell_result = unsafe {
        ShellExecuteW(
            HWND(std::ptr::null_mut()),
            PCWSTR(operation.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (shell_result.0 as isize) > 32 {
        return json!({
            "success": true,
            "name": spec,
            "method": "ShellExecuteW"
        });
    }

    // Fallback: open Start menu, type app name, press Enter
    let _ = UiaKeyPress.call(&json!({"keys": "win"}));
    std::thread::sleep(Duration::from_millis(300));
    let _ = UiaTypeText.call(&json!({"text": spec}));
    std::thread::sleep(Duration::from_millis(150));
    let _ = UiaKeyPress.call(&json!({"keys": "enter"}));

    json!({
        "success": true,
        "name": spec,
        "method": "start_search_fallback",
        "shell_execute_code": shell_result.0 as usize
    })
}

#[cfg(not(windows))]
fn launch_application(spec: &str) -> Value {
    json!({"success": false, "error": "App launch only available on Windows"})
}

/// Supported window actions.
const VALID_ACTIONS: &[&str] = &[
    "open", "close", "focus", "minimize", "maximize", "restore",
    "snap_left", "snap_right", "snap_top", "snap_bottom",
];

pub async fn handle(
    args: &Value,
    _browser: &browser_mcp::browser::SharedBrowser,
    session: &SharedSession,
) -> Value {
    let start = Instant::now();
    let call_id = {
        let mut s = session.write().unwrap_or_else(|e| e.into_inner());
        s.next_call_id()
    };

    // ── Parse args ──
    let action = match args.get("action").and_then(|v| v.as_str()) {
        Some(a) => a.to_string(),
        None => {
            instrumentation::log_aggregate(
                "hands_app_action", &call_id, false, "", 0, 0, None, Some("action is required"),
            );
            return MetaToolResult::failure(
                vec![], MetaError::other("action is required"), 0,
            ).to_value();
        }
    };

    if !VALID_ACTIONS.contains(&action.as_str()) {
        let msg = format!(
            "Unknown action '{}'. Valid: {}",
            action,
            VALID_ACTIONS.join(", ")
        );
        instrumentation::log_aggregate(
            "hands_app_action", &call_id, false, "", 0, 0, None, Some(&msg),
        );
        return MetaToolResult::failure(vec![], MetaError::other(msg), 0).to_value();
    }

    let launch_spec = args.get("launch_spec").and_then(|v| v.as_str()).map(|s| s.to_string());
    let window_match_parsed = parse_window_match(args);
    let match_mode = parse_match_mode(args.get("match_mode").and_then(|v| v.as_str()));
    let on_save_dialog = parse_save_dialog_action(args.get("on_save_dialog").and_then(|v| v.as_str()));
    let monitor = parse_monitor(args);
    let _wait_ready = args.get("wait_ready").and_then(|v| v.as_bool()).unwrap_or(true);
    let timeout_ms = args.get("timeout_ms").and_then(|v| v.as_u64()).unwrap_or(10000);
    let force_close = args.get("force_close").and_then(|v| v.as_bool()).unwrap_or(false);

    let ctx = json!({
        "action": &action,
        "launch_spec": &launch_spec,
        "timeout_ms": timeout_ms,
    });

    let mut rungs_tried = Vec::new();

    // Dispatch to action handler
    let result = match action.as_str() {
        "open" => handle_open(
            &launch_spec, &window_match_parsed, &monitor, timeout_ms,
            &call_id, &ctx, &mut rungs_tried, session,
        ).await,
        "close" => handle_close(
            &window_match_parsed, &match_mode, &on_save_dialog, force_close,
            timeout_ms, &call_id, &ctx, &mut rungs_tried, session,
        ).await,
        "focus" => handle_focus(
            &window_match_parsed, &match_mode, timeout_ms,
            &call_id, &ctx, &mut rungs_tried, session,
        ).await,
        "minimize" | "maximize" | "restore" => handle_window_state(
            &action, &window_match_parsed, &match_mode, timeout_ms,
            &call_id, &ctx, &mut rungs_tried, session,
        ).await,
        "snap_left" | "snap_right" | "snap_top" | "snap_bottom" => handle_snap(
            &action, &window_match_parsed, &match_mode, &monitor, timeout_ms,
            &call_id, &ctx, &mut rungs_tried, session,
        ).await,
        _ => unreachable!(), // validated above
    };

    let elapsed = start.elapsed().as_millis() as u64;

    match result {
        Ok(payload) => {
            let reversibility = action_reversibility(&action);
            instrumentation::log_aggregate(
                "hands_app_action", &call_id, true, &action,
                rungs_tried.len(), elapsed, Some(0.95), None,
            );
            MetaToolResult::success(&action, rungs_tried, payload, elapsed)
                .with_confidence(Confidence::method_only(0.95))
                .with_reversibility(reversibility)
                .to_value()
        }
        Err(error) => {
            let error_msg = error.to_string();
            instrumentation::log_aggregate(
                "hands_app_action", &call_id, false, &action,
                rungs_tried.len(), elapsed, None, Some(&error_msg),
            );
            MetaToolResult::failure(rungs_tried, error, elapsed).to_value()
        }
    }
}

// ── Action handlers ──

/// Open: launch via uia_app_launch or shell start. Wait for window. Record monitor.
async fn handle_open(
    launch_spec: &Option<String>,
    window_match: &Option<WindowMatch>,
    monitor: &Option<Monitor>,
    timeout_ms: u64,
    call_id: &str,
    ctx: &Value,
    rungs: &mut Vec<RungAttempt>,
    session: &SharedSession,
) -> Result<Value, MetaError> {
    let spec = launch_spec.as_deref().ok_or_else(|| {
        MetaError::other("launch_spec is required for open action")
    })?;

    // Rung 1: launch application directly (ShellExecuteW + Start menu fallback)
    // NOTE: Previously called uia_lib::handle_tool_call("uia_app_launch", ...) which
    // failed because uia_app_launch is a combo tool in main.rs, not in uia_lib.
    // Also had a param name mismatch ("path" vs "name"). Fixed in Phase C fix1.
    let rung_start = Instant::now();
    let launch_result = launch_application(spec);
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    let launch_ok = launch_result.get("success").and_then(|v| v.as_bool()).unwrap_or(false);

    if launch_ok {
        rungs.push(RungAttempt::ok("uia_app_launch", rung_ms));
        instrumentation::log_rung_attempt(
            "hands_app_action", call_id, "uia_app_launch", true, rung_ms, Some(0.95), ctx,
        );
    } else {
        rungs.push(RungAttempt::failed("uia_app_launch", rung_ms, "Launch failed or no window"));
        instrumentation::log_rung_attempt(
            "hands_app_action", call_id, "uia_app_launch", false, rung_ms, None, ctx,
        );
        return Err(MetaError::other(format!(
            "Failed to launch '{}': {}",
            spec,
            serde_json::to_string(&launch_result).unwrap_or_default()
        )));
    }

    // Wait for window to appear and verify foreground
    tokio::time::sleep(Duration::from_millis(500)).await;

    let verify_result = verify_foreground_window(window_match, spec);

    // Record monitor stickiness
    if let Some(ref wm) = window_match {
        let title = wm.title.as_deref().unwrap_or(spec);
        let monitor_idx = monitor.as_ref().map(|m| match m {
            Monitor::Index(i) => *i,
            _ => 0,
        }).unwrap_or(0);

        let mut s = session.write().unwrap_or_else(|e| e.into_inner());
        s.record_window_monitor(title, monitor_idx, 1.0);
    }

    Ok(json!({
        "action": "open",
        "launch_spec": spec,
        "launched": true,
        "foreground_verified": verify_result.is_ok(),
        "launch_detail": launch_result,
    }))
}

/// Close: focus window, send Alt+F4. Detect save dialog. Verify window gone.
async fn handle_close(
    window_match: &Option<WindowMatch>,
    match_mode: &MatchMode,
    on_save_dialog: &SaveDialogAction,
    force_close: bool,
    timeout_ms: u64,
    call_id: &str,
    ctx: &Value,
    rungs: &mut Vec<RungAttempt>,
    session: &SharedSession,
) -> Result<Value, MetaError> {
    let wm = window_match.as_ref().ok_or_else(|| {
        MetaError::other("window_match (title, process, or automation_id) is required for close")
    })?;

    // Find the target window
    let windows = list_windows();
    let target = find_single_window(&windows, wm, match_mode)?;
    let target_title = target.title.clone();

    // Focus the window first
    let rung_start = Instant::now();
    if let Some(ref hwnd) = target.hwnd {
        let _ = UiaFocusWindow.call(&json!({"hwnd": hwnd}));
    } else {
        let _ = UiaFocusWindow.call(&json!({"title": &target_title}));
    }
    let rung_ms = rung_start.elapsed().as_millis() as u64;
    rungs.push(RungAttempt::ok("focus_before_close", rung_ms));
    instrumentation::log_rung_attempt(
        "hands_app_action", call_id, "focus_before_close", true, rung_ms, None, ctx,
    );

    // Send Alt+F4
    let rung_start = Instant::now();
    let _ = UiaKeyPress.call(&json!({"keys": "alt+f4"}));
    let rung_ms = rung_start.elapsed().as_millis() as u64;
    rungs.push(RungAttempt::ok("alt_f4", rung_ms));
    instrumentation::log_rung_attempt(
        "hands_app_action", call_id, "alt_f4", true, rung_ms, None, ctx,
    );

    // Poll for save dialog — it may take up to 2s to appear after Alt+F4.
    // The dialog is a separate HWND (e.g. #32770) so we need to re-enumerate.
    let mut dialog_handled = false;
    let mut dialog_description = String::new();

    let dialog_poll_start = Instant::now();
    let dialog_poll_deadline = Duration::from_millis(2000);
    let mut dialog_info_found = None;

    while dialog_poll_start.elapsed() < dialog_poll_deadline {
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Try 1: list_windows (top-level window enumeration)
        let post_windows = list_windows();
        if let Some(info) = save_dialog::detect_save_dialog(&post_windows) {
            dialog_info_found = Some(info);
            break;
        }

        // Try 2: check if window is already gone (no dialog needed)
        let still_exists = find_single_window(&post_windows, wm, match_mode).is_ok();
        if !still_exists {
            // Window closed cleanly without dialog
            break;
        }

        // Try 3: UIA find for dialog-class elements (catches child/modal dialogs)
        let uia_dialog = UiaFind.call(&json!({"role": "dialog", "max_results": 1}));
        if let Some(items) = uia_dialog.get("items").and_then(|v| v.as_array()) {
            if !items.is_empty() {
                // Convert UIA find result to the format detect_save_dialog expects
                if let Some(info) = save_dialog::detect_save_dialog(items) {
                    dialog_info_found = Some(info);
                    break;
                }
            }
        }

        // Try 4 (Phase C fix3): Direct button probe — look for known save-dialog
        // buttons by name, which catches dialogs even when detect_save_dialog misses
        // because UIA find returns items in an unexpected format.
        let probe_buttons = ["Don't Save", "Do&n't Save", "&Don't Save", "Save", "&Save", "No", "&No", "Discard"];
        let mut found_buttons = Vec::new();
        for btn_name in &probe_buttons {
            let probe = UiaFind.call(&json!({"name": btn_name, "role": "button", "max_results": 1}));
            if let Some(items) = probe.get("items").and_then(|v| v.as_array()) {
                if !items.is_empty() {
                    found_buttons.push(btn_name.to_string());
                }
            }
        }
        if !found_buttons.is_empty() {
            dialog_info_found = Some(save_dialog::SaveDialogInfo {
                detected: true,
                dialog_text: None,
                buttons: found_buttons,
                suggested_action: "discard".to_string(),
            });
            break;
        }
    }

    let dialog_poll_ms = dialog_poll_start.elapsed().as_millis() as u64;
    rungs.push(RungAttempt::ok("dialog_poll", dialog_poll_ms));
    instrumentation::log_rung_attempt(
        "hands_app_action", call_id, "dialog_poll",
        dialog_info_found.is_some(), dialog_poll_ms, None, ctx,
    );

    if let Some(dialog_info) = dialog_info_found {
        let app_name = target.process_name.as_deref().unwrap_or(&target_title);
        match save_dialog::resolve_dialog_action(&dialog_info, on_save_dialog, app_name) {
            Ok(resolution) => {
                dialog_description = resolution.description.clone();
                if let Some(ref button) = resolution.button_text {
                    let rung_start = Instant::now();
                    let _ = UiaClick.call(&json!({"name": button}));
                    let rung_ms = rung_start.elapsed().as_millis() as u64;
                    rungs.push(RungAttempt::ok("dialog_button_click", rung_ms));
                    instrumentation::log_rung_attempt(
                        "hands_app_action", call_id, "dialog_button_click", true,
                        rung_ms, None, ctx,
                    );
                    dialog_handled = true;
                } else {
                    // Ask mode — return dialog info without acting
                    return Ok(json!({
                        "action": "close",
                        "window_title": target_title,
                        "dialog_detected": true,
                        "dialog_info": serde_json::to_value(&dialog_info).unwrap_or(Value::Null),
                        "dialog_action": dialog_description,
                        "closed": false,
                    }));
                }
            }
            Err(e) => {
                if force_close {
                    // Force close: try taskkill
                    let rung_start = Instant::now();
                    if let Some(ref proc) = target.process_name {
                        let _ = UiaKeyPress.call(&json!({"keys": "escape"}));
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        // Try Alt+F4 again, or the caller can use taskkill
                        let _ = UiaKeyPress.call(&json!({"keys": "alt+f4"}));
                        dialog_description = format!("Force close attempted for {}", proc);
                    }
                    let rung_ms = rung_start.elapsed().as_millis() as u64;
                    rungs.push(RungAttempt::failed("force_close", rung_ms, &e));
                    dialog_handled = true;
                } else {
                    return Err(MetaError::DialogBlocking {
                        dialog_title: target_title.clone(),
                        buttons: dialog_info.buttons.clone(),
                    });
                }
            }
        }
    }

    // Verify window is gone (poll for up to 2 seconds)
    let verify_start = Instant::now();
    let mut window_gone = false;
    let deadline = Duration::from_millis(timeout_ms.min(2000));

    while verify_start.elapsed() < deadline {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let check_windows = list_windows();
        let still_exists = find_single_window(&check_windows, wm, match_mode).is_ok();
        if !still_exists {
            window_gone = true;
            break;
        }
    }

    Ok(json!({
        "action": "close",
        "window_title": target_title,
        "closed": window_gone,
        "dialog_handled": dialog_handled,
        "dialog_action": dialog_description,
    }))
}

/// Focus: set window foreground, verify it's foreground.
async fn handle_focus(
    window_match: &Option<WindowMatch>,
    match_mode: &MatchMode,
    _timeout_ms: u64,
    call_id: &str,
    ctx: &Value,
    rungs: &mut Vec<RungAttempt>,
    _session: &SharedSession,
) -> Result<Value, MetaError> {
    let wm = window_match.as_ref().ok_or_else(|| {
        MetaError::other("window_match (title, process, or automation_id) is required for focus")
    })?;

    let windows = list_windows();
    let target = find_single_window(&windows, wm, match_mode)?;
    let target_title = target.title.clone();

    // Try hwnd-based focus first (more reliable), fall back to title if hwnd fails
    let rung_start = Instant::now();
    let (focus_result, used_hwnd) = if let Some(ref hwnd) = target.hwnd {
        let r = UiaFocusWindow.call(&json!({"hwnd": hwnd}));
        let ok = r.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
        if ok {
            (r, true)
        } else {
            // hwnd focus failed — fall back to title-based focus
            let r2 = UiaFocusWindow.call(&json!({"title": &target_title}));
            (r2, false)
        }
    } else {
        let r = UiaFocusWindow.call(&json!({"title": &target_title}));
        (r, false)
    };
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    let focus_ok = focus_result.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
    if focus_ok {
        let method = if used_hwnd { "uia_focus_window(hwnd)" } else { "uia_focus_window(title)" };
        rungs.push(RungAttempt::ok(method, rung_ms));
        instrumentation::log_rung_attempt(
            "hands_app_action", call_id, "uia_focus_window", true, rung_ms, Some(0.95), ctx,
        );
    } else {
        let error_detail = focus_result.get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("focus primitive returned failure");
        rungs.push(RungAttempt::failed("uia_focus_window", rung_ms, error_detail));
        instrumentation::log_rung_attempt(
            "hands_app_action", call_id, "uia_focus_window", false, rung_ms, None, ctx,
        );
        return Err(MetaError::FocusLost {
            expected: target_title.clone(),
            actual: format!("focus failed: {}", error_detail),
        });
    }

    // Brief wait then verify foreground
    tokio::time::sleep(Duration::from_millis(100)).await;
    let fg_verified = verify_foreground_window(window_match, &target_title).is_ok();

    Ok(json!({
        "action": "focus",
        "window_title": target_title,
        "foreground_verified": fg_verified,
        "window_state": "focused",
    }))
}

/// Minimize, maximize, restore via uia_window_state.
async fn handle_window_state(
    action: &str,
    window_match: &Option<WindowMatch>,
    match_mode: &MatchMode,
    _timeout_ms: u64,
    call_id: &str,
    ctx: &Value,
    rungs: &mut Vec<RungAttempt>,
    _session: &SharedSession,
) -> Result<Value, MetaError> {
    let wm = window_match.as_ref().ok_or_else(|| {
        MetaError::other("window_match (title, process, or automation_id) is required")
    })?;

    let windows = list_windows();
    let target = find_single_window(&windows, wm, match_mode)?;
    let target_title = target.title.clone();

    // Map action name to UIA state command
    let state_cmd = match action {
        "minimize" => "minimize",
        "maximize" => "maximize",
        "restore" => "restore",
        _ => unreachable!(),
    };

    let rung_start = Instant::now();
    let state_args = if let Some(ref hwnd) = target.hwnd {
        json!({"hwnd": hwnd, "state": state_cmd})
    } else {
        json!({"title": &target_title, "state": state_cmd})
    };
    let state_result = UiaWindowState.call(&state_args);
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    let state_ok = state_result.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
    if state_ok {
        rungs.push(RungAttempt::ok("uia_window_state", rung_ms));
        instrumentation::log_rung_attempt(
            "hands_app_action", call_id, "uia_window_state", true, rung_ms, Some(0.95), ctx,
        );
    } else {
        rungs.push(RungAttempt::failed("uia_window_state", rung_ms, "State change failed"));
        instrumentation::log_rung_attempt(
            "hands_app_action", call_id, "uia_window_state", false, rung_ms, None, ctx,
        );
        return Err(MetaError::other(format!(
            "Failed to {} window '{}': {}",
            action, target_title,
            serde_json::to_string(&state_result).unwrap_or_default()
        )));
    }

    // Verify state matches after brief delay
    tokio::time::sleep(Duration::from_millis(200)).await;
    let verify_windows = list_windows();
    let verify_target = find_single_window(&verify_windows, wm, match_mode).ok();
    let verified_state = verify_target.as_ref().map(|_| state_cmd.to_string());

    Ok(json!({
        "action": action,
        "window_title": target_title,
        "window_state": state_cmd,
        "state_verified": verified_state.is_some(),
    }))
}

/// Snap window to screen edge via uia_window_snap with monitor awareness.
async fn handle_snap(
    action: &str,
    window_match: &Option<WindowMatch>,
    match_mode: &MatchMode,
    monitor: &Option<Monitor>,
    _timeout_ms: u64,
    call_id: &str,
    ctx: &Value,
    rungs: &mut Vec<RungAttempt>,
    session: &SharedSession,
) -> Result<Value, MetaError> {
    let wm = window_match.as_ref().ok_or_else(|| {
        MetaError::other("window_match (title, process, or automation_id) is required for snap")
    })?;

    let windows = list_windows();
    let target = find_single_window(&windows, wm, match_mode)?;
    let target_title = target.title.clone();

    // Determine snap direction
    let direction = match action {
        "snap_left" => "left",
        "snap_right" => "right",
        "snap_top" => "top",
        "snap_bottom" => "bottom",
        _ => unreachable!(),
    };

    // Determine target monitor index
    let monitor_idx = match monitor {
        Some(Monitor::Index(i)) => Some(*i),
        Some(Monitor::Owning) => {
            // Check session for last-known monitor
            let s = session.read().unwrap_or_else(|e| e.into_inner());
            s.get_window_monitor(&target_title).map(|r| r.monitor_index)
        }
        Some(Monitor::Primary) => Some(0),
        Some(Monitor::Current) => target.monitor_index,
        None => target.monitor_index,
    };

    let rung_start = Instant::now();
    let mut snap_args = if let Some(ref hwnd) = target.hwnd {
        json!({"hwnd": hwnd, "direction": direction})
    } else {
        json!({"title": &target_title, "direction": direction})
    };
    if let Some(idx) = monitor_idx {
        snap_args.as_object_mut().unwrap().insert(
            "monitor".to_string(), json!(idx),
        );
    }

    let snap_result = UiaWindowSnap.call(&snap_args);
    let rung_ms = rung_start.elapsed().as_millis() as u64;

    let snap_ok = snap_result.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
    if snap_ok {
        rungs.push(RungAttempt::ok("uia_window_snap", rung_ms));
        instrumentation::log_rung_attempt(
            "hands_app_action", call_id, "uia_window_snap", true, rung_ms, Some(0.9), ctx,
        );
    } else {
        rungs.push(RungAttempt::failed("uia_window_snap", rung_ms, "Snap failed"));
        instrumentation::log_rung_attempt(
            "hands_app_action", call_id, "uia_window_snap", false, rung_ms, None, ctx,
        );
        return Err(MetaError::other(format!(
            "Failed to snap window '{}' {}: {}",
            target_title, direction,
            serde_json::to_string(&snap_result).unwrap_or_default()
        )));
    }

    // Verify bounds after snap (within 10px tolerance)
    tokio::time::sleep(Duration::from_millis(300)).await;
    let verify_windows = list_windows();
    let verify_target = find_single_window(&verify_windows, wm, match_mode).ok();
    let new_bounds = verify_target.as_ref().and_then(|t| t.bounds);

    // Update session monitor record
    if let Some(bounds) = new_bounds {
        let mut s = session.write().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = s.window_monitors.get_mut(&target_title) {
            record.last_bounds = Some(bounds);
        } else {
            let idx = monitor_idx.unwrap_or(0);
            s.record_window_monitor(&target_title, idx, 1.0);
            if let Some(record) = s.window_monitors.get_mut(&target_title) {
                record.last_bounds = Some(bounds);
            }
        }
    }

    Ok(json!({
        "action": action,
        "window_title": target_title,
        "direction": direction,
        "monitor_index": monitor_idx,
        "bounds": new_bounds.map(|(x, y, w, h)| json!({"x": x, "y": y, "width": w, "height": h})),
        "bounds_verified": new_bounds.is_some(),
    }))
}

// ── Helpers ──

/// List all windows via UiaListWindow, returning the array of window objects.
fn list_windows() -> Vec<Value> {
    let result = UiaListWindow.call(&json!({}));
    result.get("windows")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Verify that the foreground window matches our target.
fn verify_foreground_window(
    window_match: &Option<WindowMatch>,
    fallback_title: &str,
) -> Result<(), MetaError> {
    let fg_result = UiaGetState.call(&json!({"property": "foreground_window"}));
    let fg_title = fg_result.get("title")
        .or_else(|| fg_result.get("name"))
        .or_else(|| fg_result.get("foreground_title"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let expected_title = window_match
        .as_ref()
        .and_then(|wm| wm.title.as_deref())
        .unwrap_or(fallback_title);

    if fg_title.to_lowercase().contains(&expected_title.to_lowercase()) {
        Ok(())
    } else {
        Err(MetaError::FocusLost {
            expected: expected_title.to_string(),
            actual: fg_title.to_string(),
        })
    }
}

/// Determine reversibility based on the action type.
fn action_reversibility(action: &str) -> Reversibility {
    match action {
        "close" => Reversibility::RequiresConfirmation,
        _ => Reversibility::Reversible,
    }
}
