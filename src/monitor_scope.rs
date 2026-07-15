use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

pub const TOOL_NAME: &str = "hands_monitor_scope";

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorDescriptor {
    pub index: usize,
    pub display_id: u32,
    pub stable_id: String,
    pub stable_physical: bool,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub rotation: f32,
    pub frequency: f32,
    pub is_primary: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Fingerprint {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_bits: u32,
}

impl From<&MonitorDescriptor> for Fingerprint {
    fn from(value: &MonitorDescriptor) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
            scale_bits: value.scale_factor.to_bits(),
        }
    }
}

#[derive(Clone, Debug)]
enum Binding {
    Fixed {
        stable_id: String,
        fingerprint: Fingerprint,
        browser_window_title: Option<String>,
    },
    Primary {
        browser_window_title: Option<String>,
    },
}

impl Binding {
    fn browser_window_title(&self) -> Option<&str> {
        match self {
            Self::Fixed {
                browser_window_title,
                ..
            }
            | Self::Primary {
                browser_window_title,
            } => browser_window_title.as_deref(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ScopeState {
    binding: Option<Binding>,
    config_error: Option<String>,
}

static STATE: OnceLock<RwLock<ScopeState>> = OnceLock::new();
static SAFE_REFS: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
static SAFE_REF_MONITOR: OnceLock<RwLock<Option<String>>> = OnceLock::new();
static PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn state() -> &'static RwLock<ScopeState> {
    STATE.get_or_init(|| RwLock::new(initial_state_from_env()))
}

fn safe_refs() -> &'static RwLock<HashSet<String>> {
    SAFE_REFS.get_or_init(|| RwLock::new(HashSet::new()))
}

fn safe_ref_monitor() -> &'static RwLock<Option<String>> {
    SAFE_REF_MONITOR.get_or_init(|| RwLock::new(None))
}

fn sync_safe_ref_monitor(monitor: &MonitorDescriptor) {
    let mut identity = safe_ref_monitor()
        .write()
        .unwrap_or_else(|error| error.into_inner());
    if identity.as_deref() != Some(monitor.stable_id.as_str()) {
        safe_refs()
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        *identity = Some(monitor.stable_id.clone());
    }
}

fn scope_locked() -> bool {
    std::env::var("HANDS_MONITOR_SCOPE_LOCKED")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn inactive_selector_state(selector: &str, locked: bool) -> Option<ScopeState> {
    if !selector.is_empty() && !selector.eq_ignore_ascii_case("off") {
        return None;
    }

    if locked {
        return Some(ScopeState {
            binding: None,
            config_error: Some(
                "HANDS_MONITOR_SCOPE_LOCKED requires an explicit non-off HANDS_MONITOR_SCOPE selector (primary, stable:<id>, display:<id>, or index:<n>); refusing to start unlocked"
                    .to_string(),
            ),
        });
    }

    Some(ScopeState::default())
}

fn initial_state_from_env() -> ScopeState {
    let raw = std::env::var("HANDS_MONITOR_SCOPE").unwrap_or_default();
    let selector = raw.trim();
    if let Some(state) = inactive_selector_state(selector, scope_locked()) {
        return state;
    }

    let browser_window_title = std::env::var("HANDS_MONITOR_BROWSER_TITLE")
        .ok()
        .filter(|value| !value.trim().is_empty());

    if selector.eq_ignore_ascii_case("primary") {
        return ScopeState {
            binding: Some(Binding::Primary {
                browser_window_title,
            }),
            config_error: None,
        };
    }

    let monitors = match list_monitors() {
        Ok(monitors) => monitors,
        Err(error) => {
            return ScopeState {
                binding: None,
                config_error: Some(format!(
                    "HANDS_MONITOR_SCOPE could not enumerate displays: {error}"
                )),
            }
        }
    };

    let selected = if let Some(value) = selector.strip_prefix("stable:") {
        monitors
            .iter()
            .find(|monitor| monitor.stable_id.eq_ignore_ascii_case(value))
    } else if let Some(value) = selector.strip_prefix("display:") {
        value
            .parse::<u32>()
            .ok()
            .and_then(|id| monitors.iter().find(|monitor| monitor.display_id == id))
    } else {
        selector
            .strip_prefix("index:")
            .unwrap_or(selector)
            .parse::<usize>()
            .ok()
            .and_then(|index| monitors.get(index))
    };

    match selected {
        Some(monitor) if monitor.stable_physical => ScopeState {
            binding: Some(Binding::Fixed {
                stable_id: monitor.stable_id.clone(),
                fingerprint: Fingerprint::from(monitor),
                browser_window_title,
            }),
            config_error: None,
        },
        Some(_) => ScopeState {
            binding: None,
            config_error: Some(
                "HANDS_MONITOR_SCOPE fixed mode requires a physical monitor identity; use primary for a virtual display"
                    .to_string(),
            ),
        },
        None => ScopeState {
            binding: None,
            config_error: Some(format!(
                "Invalid HANDS_MONITOR_SCOPE '{selector}'. Use off, primary, index:N, display:ID, or stable:ID."
            )),
        },
    }
}

#[cfg(windows)]
fn stable_display_identity(x: i32, y: i32, fallback_id: u32) -> (String, bool) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayDevicesW, GetMonitorInfoW, MonitorFromPoint, DISPLAY_DEVICEW, MONITORINFO,
        MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::EDD_GET_DEVICE_INTERFACE_NAME;

    fn utf16(array: &[u16]) -> String {
        let end = array
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(array.len());
        String::from_utf16_lossy(&array[..end])
    }

    let handle = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    if handle.0.is_null() {
        return (format!("logical:{fallback_id}"), false);
    }

    let mut monitor_info = MONITORINFOEXW::default();
    monitor_info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let monitor_info_ptr = &mut monitor_info as *mut MONITORINFOEXW as *mut MONITORINFO;
    if !unsafe { GetMonitorInfoW(handle, monitor_info_ptr) }.as_bool() {
        return (format!("logical:{fallback_id}"), false);
    }

    let mut device = DISPLAY_DEVICEW {
        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    if !unsafe {
        EnumDisplayDevicesW(
            PCWSTR(monitor_info.szDevice.as_ptr()),
            0,
            &mut device,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
    }
    .as_bool()
    {
        return (format!("logical:{fallback_id}"), false);
    }

    let interface_path = utf16(&device.DeviceID);
    if !interface_path.is_empty() {
        return (interface_path.to_ascii_lowercase(), true);
    }
    let device_key = utf16(&device.DeviceKey);
    if !device_key.is_empty() {
        return (device_key.to_ascii_lowercase(), true);
    }
    (format!("logical:{fallback_id}"), false)
}

#[cfg(not(windows))]
fn stable_display_identity(_x: i32, _y: i32, fallback_id: u32) -> (String, bool) {
    (format!("logical:{fallback_id}"), false)
}

fn descriptors_from_screens(screens: &[screenshots::Screen]) -> Vec<MonitorDescriptor> {
    screens
        .iter()
        .enumerate()
        .map(|(index, screen)| {
            let info = screen.display_info;
            let (stable_id, stable_physical) = stable_display_identity(info.x, info.y, info.id);
            MonitorDescriptor {
                index,
                display_id: info.id,
                stable_id,
                stable_physical,
                x: info.x,
                y: info.y,
                width: info.width,
                height: info.height,
                scale_factor: info.scale_factor,
                rotation: info.rotation,
                frequency: info.frequency,
                is_primary: info.is_primary,
            }
        })
        .collect()
}

pub fn list_monitors() -> Result<Vec<MonitorDescriptor>, String> {
    let screens = screenshots::Screen::all().map_err(|error| error.to_string())?;
    Ok(descriptors_from_screens(&screens))
}

fn monitor_json(monitor: &MonitorDescriptor) -> Value {
    json!({
        "index": monitor.index,
        "display_id": monitor.display_id,
        "stable_id": monitor.stable_id,
        "stable_physical": monitor.stable_physical,
        "is_primary": monitor.is_primary,
        "logical_bounds": {
            "x": monitor.x,
            "y": monitor.y,
            "width": monitor.width,
            "height": monitor.height
        },
        "scale_factor": monitor.scale_factor,
        "rotation": monitor.rotation,
        "frequency": monitor.frequency
    })
}

fn strict_error(
    code: &str,
    message: impl Into<String>,
    monitor: Option<&MonitorDescriptor>,
) -> Value {
    json!({
        "success": false,
        "error": message.into(),
        "monitor_scope": {
            "strict": true,
            "fail_closed": true,
            "code": code,
            "resolved_monitor": monitor.map(monitor_json)
        }
    })
}

fn resolve_binding_from(
    binding: &Binding,
    monitors: Vec<MonitorDescriptor>,
) -> Result<MonitorDescriptor, Value> {
    match binding {
        Binding::Primary { .. } => monitors
            .into_iter()
            .find(|monitor| monitor.is_primary)
            .ok_or_else(|| {
                strict_error("primary_missing", "No primary monitor is available", None)
            }),
        Binding::Fixed {
            stable_id,
            fingerprint,
            ..
        } => {
            let monitor = monitors
                .into_iter()
                .find(|monitor| monitor.stable_id == *stable_id)
                .ok_or_else(|| {
                    strict_error(
                        "topology_changed",
                        format!("Bound physical display '{stable_id}' is no longer present; recalibrate the scope"),
                        None,
                    )
                })?;
            if Fingerprint::from(&monitor) != *fingerprint {
                return Err(strict_error(
                    "topology_changed",
                    "The bound display geometry or DPI changed; recalibrate the fixed scope",
                    Some(&monitor),
                ));
            }
            Ok(monitor)
        }
    }
}

fn resolve_binding(binding: &Binding) -> Result<MonitorDescriptor, Value> {
    let monitors =
        list_monitors().map_err(|error| strict_error("enumeration_failed", error, None))?;
    resolve_binding_from(binding, monitors)
}

pub fn selected_screen(
    requested_monitor: usize,
) -> Result<(screenshots::Screen, MonitorDescriptor), String> {
    let screens = screenshots::Screen::all()
        .map_err(|error| format!("Could not enumerate screens: {error}"))?;
    let monitors = descriptors_from_screens(&screens);
    let guard = state().read().unwrap_or_else(|error| error.into_inner());
    if let Some(error) = &guard.config_error {
        return Err(error.clone());
    }
    let monitor = if let Some(binding) = &guard.binding {
        resolve_binding_from(binding, monitors).map_err(|error| {
            error
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("monitor scope rejected capture")
                .to_string()
        })?
    } else {
        monitors
            .get(requested_monitor)
            .cloned()
            .ok_or_else(|| format!("Monitor {requested_monitor} is not available"))?
    };
    let screen = screens
        .get(monitor.index)
        .copied()
        .ok_or_else(|| "Display enumeration changed during selection".to_string())?;
    if screen.display_info.id != monitor.display_id {
        return Err("Display enumeration changed during selection".to_string());
    }
    Ok((screen, monitor))
}

pub fn validate_monitor_snapshot(expected: &MonitorDescriptor) -> Result<(), String> {
    let monitors = list_monitors()?;
    let current = monitors
        .into_iter()
        .find(|monitor| monitor.stable_id == expected.stable_id)
        .ok_or_else(|| "Selected monitor disappeared during capture".to_string())?;
    if current.index != expected.index
        || current.display_id != expected.display_id
        || Fingerprint::from(&current) != Fingerprint::from(expected)
    {
        return Err("Display topology changed during capture; result was discarded".to_string());
    }
    Ok(())
}

pub fn active_monitor() -> Result<Option<MonitorDescriptor>, Value> {
    let guard = state().read().unwrap_or_else(|error| error.into_inner());
    if let Some(error) = &guard.config_error {
        return Err(strict_error("configuration_error", error.clone(), None));
    }
    match &guard.binding {
        Some(binding) => resolve_binding(binding).map(Some),
        None => Ok(None),
    }
}

fn binding_browser_title() -> Option<String> {
    state()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .binding
        .as_ref()
        .and_then(Binding::browser_window_title)
        .map(str::to_string)
}

pub fn status() -> Value {
    let guard = state().read().unwrap_or_else(|error| error.into_inner());
    let mode = match &guard.binding {
        Some(Binding::Fixed { .. }) => "fixed",
        Some(Binding::Primary { .. }) => "primary",
        None => "off",
    };
    let (resolved, resolution_error) = match guard.binding.as_ref().map(resolve_binding) {
        Some(Ok(monitor)) => (Some(monitor), None),
        Some(Err(error)) => (None, error.get("error").cloned()),
        None => (None, None),
    };
    json!({
        "enabled": guard.binding.is_some(),
        "mode": mode,
        "strict": guard.binding.is_some() || guard.config_error.is_some(),
        "fail_closed": guard.binding.is_some() || guard.config_error.is_some(),
        "locked": scope_locked(),
        "browser_window_title": guard.binding.as_ref().and_then(Binding::browser_window_title),
        "resolved_monitor": resolved.as_ref().map(monitor_json),
        "configuration_error": guard.config_error.clone(),
        "resolution_error": resolution_error,
        "env": {
            "scope": "HANDS_MONITOR_SCOPE",
            "browser_title": "HANDS_MONITOR_BROWSER_TITLE",
            "locked": "HANDS_MONITOR_SCOPE_LOCKED"
        },
        "recommendation": {
            "unattended": "fixed stable_id binding with HANDS_MONITOR_SCOPE_LOCKED=1",
            "interactive": "primary mode",
            "security_grade": "use a separate Windows session or VM"
        }
    })
}

pub fn tool_definition() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "List monitors and manage one central fail-closed monitor fence. Use action=set with mode=fixed for unattended automation (physical device identity plus topology fingerprint), or mode=primary for interactive work that should follow the current primary display. Set an explicit HANDS_MONITOR_SCOPE together with HANDS_MONITOR_SCOPE_LOCKED=1 for unattended runs so the agent cannot clear or redirect its own fence; locked-without-scope is a fail-closed configuration error. While active, screen capture is pinned, UIA discovery is filtered, global coordinates/input are checked, titled windows must belong to the scope, and topology drift fails closed.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "get", "set", "clear"], "default": "get"},
                "mode": {"type": "string", "enum": ["fixed", "primary"], "default": "fixed"},
                "monitor": {"type": "integer", "description": "Current monitor index to resolve and bind when mode=fixed"},
                "display_id": {"type": "integer", "description": "Current logical display ID from action=list; accepted for selection but not stored as the physical binding"},
                "stable_id": {"type": "string", "description": "Physical monitor interface identity from action=list; strongest unattended selector"},
                "browser_window_title": {"type": "string", "description": "Optional dedicated visible browser window title. Required before visible browser/CDP tools can run under the fence."}
            }
        }
    })
}

pub fn handle_tool(args: &Value) -> Value {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("get")
        .to_ascii_lowercase();

    if action == "list" {
        return match list_monitors() {
            Ok(monitors) => json!({
                "success": true,
                "monitors": monitors.iter().map(monitor_json).collect::<Vec<_>>(),
                "scope": status()
            }),
            Err(error) => strict_error("enumeration_failed", error, None),
        };
    }

    if action == "get" {
        if let Err(error) = active_monitor() {
            return error;
        }
        return json!({"success": true, "scope": status()});
    }

    if scope_locked() && matches!(action.as_str(), "set" | "clear") {
        return strict_error(
            "scope_locked",
            "The monitor fence is policy-locked by HANDS_MONITOR_SCOPE_LOCKED; only list/get are allowed",
            active_monitor().ok().flatten().as_ref(),
        );
    }

    if action == "clear" {
        let mut guard = state().write().unwrap_or_else(|error| error.into_inner());
        *guard = ScopeState::default();
        drop(guard);
        safe_refs()
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        *safe_ref_monitor()
            .write()
            .unwrap_or_else(|error| error.into_inner()) = None;
        return json!({"success": true, "scope": status()});
    }

    if action != "set" {
        return strict_error(
            "invalid_action",
            "action must be list, get, set, or clear",
            None,
        );
    }

    let browser_window_title = args
        .get("browser_window_title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("fixed")
        .to_ascii_lowercase();
    let monitors = match list_monitors() {
        Ok(monitors) => monitors,
        Err(error) => return strict_error("enumeration_failed", error, None),
    };

    let binding = if mode == "primary" {
        Binding::Primary {
            browser_window_title,
        }
    } else if mode == "fixed" {
        let selected = if let Some(stable_id) = args.get("stable_id").and_then(Value::as_str) {
            monitors
                .iter()
                .find(|monitor| monitor.stable_id.eq_ignore_ascii_case(stable_id))
        } else if let Some(display_id) = args.get("display_id").and_then(Value::as_u64) {
            monitors
                .iter()
                .find(|monitor| monitor.display_id == display_id as u32)
        } else {
            let index = args.get("monitor").and_then(Value::as_u64).unwrap_or(0) as usize;
            monitors.get(index)
        };
        let Some(selected) = selected else {
            return strict_error("monitor_missing", "Requested monitor was not found", None);
        };
        if !selected.stable_physical {
            return strict_error(
                "stable_identity_unavailable",
                "A fixed unattended fence requires a physical monitor identity; use primary mode for this virtual/logical display",
                Some(selected),
            );
        }
        Binding::Fixed {
            stable_id: selected.stable_id.clone(),
            fingerprint: Fingerprint::from(selected),
            browser_window_title,
        }
    } else {
        return strict_error("invalid_mode", "mode must be fixed or primary", None);
    };

    let resolved = match resolve_binding(&binding) {
        Ok(monitor) => monitor,
        Err(error) => return error,
    };
    if let Some(title) = binding.browser_window_title() {
        if !window_query_unique_in_monitor(title, &resolved) {
            return strict_error(
                "browser_window_out_of_scope",
                format!("Browser window '{title}' was not found on the requested monitor"),
                Some(&resolved),
            );
        }
    }

    let mut guard = state().write().unwrap_or_else(|error| error.into_inner());
    *guard = ScopeState {
        binding: Some(binding),
        config_error: None,
    };
    safe_refs()
        .write()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
    *safe_ref_monitor()
        .write()
        .unwrap_or_else(|error| error.into_inner()) = Some(resolved.stable_id.clone());
    drop(guard);

    json!({"success": true, "scope": status()})
}

fn explicit_monitor(args: &Value) -> Option<Result<usize, String>> {
    let value = args.get("monitor").or_else(|| args.get("monitor_index"))?;
    if let Some(index) = value.as_u64() {
        return Some(Ok(index as usize));
    }
    if let Some(text) = value.as_str() {
        if text.eq_ignore_ascii_case("primary") {
            return Some(
                list_monitors()
                    .ok()
                    .and_then(|monitors| monitors.into_iter().find(|monitor| monitor.is_primary))
                    .map(|monitor| monitor.index)
                    .ok_or_else(|| "No primary monitor is available".to_string()),
            );
        }
        return Some(Err(format!(
            "Ambiguous monitor selector '{text}' is not allowed by a strict scope"
        )));
    }
    Some(Err(
        "monitor must be an integer index or 'primary'".to_string()
    ))
}

fn inject_monitor(args: &Value, monitor: &MonitorDescriptor) -> Result<Value, Value> {
    if let Some(requested) = explicit_monitor(args) {
        match requested {
            Ok(index) if index == monitor.index => {}
            Ok(index) => {
                return Err(strict_error(
                    "monitor_mismatch",
                    format!(
                        "Call requested monitor {index}, but the strict scope is monitor {}",
                        monitor.index
                    ),
                    Some(monitor),
                ))
            }
            Err(error) => return Err(strict_error("monitor_mismatch", error, Some(monitor))),
        }
    }
    let mut scoped = args.clone();
    if !scoped.is_object() {
        scoped = Value::Object(Map::new());
    }
    scoped
        .as_object_mut()
        .expect("scoped args must be an object")
        .insert("monitor".to_string(), json!(monitor.index));
    Ok(scoped)
}

fn point_inside(monitor: &MonitorDescriptor, x: i64, y: i64) -> bool {
    x >= monitor.x as i64
        && y >= monitor.y as i64
        && x < monitor.x as i64 + monitor.width as i64
        && y < monitor.y as i64 + monitor.height as i64
}

fn validate_coordinate_pairs(
    name: &str,
    args: &Value,
    monitor: &MonitorDescriptor,
) -> Result<(), Value> {
    if name.starts_with("vision_") {
        return Ok(());
    }
    for (x_key, y_key) in [
        ("x", "y"),
        ("click_x", "click_y"),
        ("from_x", "from_y"),
        ("to_x", "to_y"),
    ] {
        if let (Some(x), Some(y)) = (
            args.get(x_key).and_then(Value::as_i64),
            args.get(y_key).and_then(Value::as_i64),
        ) {
            if !point_inside(monitor, x, y) {
                return Err(strict_error(
                    "coordinate_out_of_scope",
                    format!("Coordinate ({x},{y}) for '{name}' is outside the monitor fence"),
                    Some(monitor),
                ));
            }
        }
    }
    Ok(())
}

fn window_rows() -> Vec<Value> {
    let value = uia_lib_backend::handle_tool_call("uia_list_window", &json!({}));
    value
        .get("windows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn value_rect(value: &Value) -> Option<(i64, i64, i64, i64)> {
    let object = value.as_object()?;
    let nested = object
        .get("rect")
        .or_else(|| object.get("bounds"))
        .or_else(|| object.get("bounding_rect"));
    let source = nested.and_then(Value::as_object).unwrap_or(object);
    let x = source.get("x")?.as_i64()?;
    let y = source.get("y")?.as_i64()?;
    let width = source.get("width")?.as_i64()?;
    let height = source.get("height")?.as_i64()?;
    Some((x, y, width, height))
}

fn rect_owned_by_monitor(rect: (i64, i64, i64, i64), monitor: &MonitorDescriptor) -> bool {
    let (x, y, width, height) = rect;
    width > 0
        && height > 0
        && point_inside(monitor, x, y)
        && point_inside(monitor, x + width - 1, y + height - 1)
}

pub fn validate_target_rect(x: i32, y: i32, width: i32, height: i32) -> Result<(), Value> {
    let Some(monitor) = active_monitor()? else {
        return Ok(());
    };
    if rect_owned_by_monitor((x as i64, y as i64, width as i64, height as i64), &monitor) {
        Ok(())
    } else {
        Err(strict_error(
            "window_geometry_out_of_scope",
            format!(
                "Requested window rectangle ({x},{y},{width},{height}) is not fully contained in the monitor fence"
            ),
            Some(&monitor),
        ))
    }
}

fn window_query_unique_in_monitor(query: &str, monitor: &MonitorDescriptor) -> bool {
    let query = query.to_ascii_lowercase();
    let matches: Vec<Value> = window_rows()
        .into_iter()
        .filter(|window| {
            window
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains(&query)
        })
        .collect();
    matches.len() == 1
        && value_rect(&matches[0]).is_some_and(|rect| rect_owned_by_monitor(rect, monitor))
}

fn window_query_is_unique(query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    window_rows()
        .iter()
        .filter(|window| {
            window
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains(&query)
        })
        .count()
        == 1
}

fn window_selector_in_monitor(args: &Value, monitor: &MonitorDescriptor) -> bool {
    if let Some(title) = args
        .get("title")
        .or_else(|| args.get("window_title"))
        .and_then(Value::as_str)
    {
        return window_query_unique_in_monitor(title, monitor);
    }
    if let Some(hwnd) = args.get("hwnd").and_then(Value::as_str) {
        return window_rows().iter().any(|window| {
            window.get("hwnd").and_then(Value::as_str) == Some(hwnd)
                && value_rect(window).is_some_and(|rect| rect_owned_by_monitor(rect, monitor))
        });
    }
    false
}

#[cfg(windows)]
fn foreground_in_monitor(monitor: &MonitorDescriptor) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect};
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return false;
    }
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok()
        && rect_owned_by_monitor(
            (
                rect.left as i64,
                rect.top as i64,
                (rect.right - rect.left) as i64,
                (rect.bottom - rect.top) as i64,
            ),
            monitor,
        )
}

#[cfg(not(windows))]
fn foreground_in_monitor(_monitor: &MonitorDescriptor) -> bool {
    false
}

fn visible_browser_call(name: &str, args: &Value) -> bool {
    if name == "hands_capture" {
        let target = args
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or("screen");
        return target == "browser"
            || target.starts_with('#')
            || target.starts_with('.')
            || target.starts_with('[');
    }
    if name == "hands_find" {
        return !matches!(
            args.get("scope").and_then(Value::as_str),
            Some("desktop" | "screen")
        );
    }
    if name == "hands_click" {
        return args
            .get("page_context")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            != "desktop";
    }
    if name == "hands_scan_qr" {
        return args
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("screen")
            == "browser";
    }
    name.starts_with("browser_")
        || matches!(
            name,
            "hands_navigate"
                | "hands_read_page"
                | "hands_click"
                | "hands_type"
                | "hands_fill_form"
                | "hands_capture"
                | "hands_find"
                | "hands_verify"
                | "hands_verify_expectations"
                | "hands_login_recovery"
                | "hands_network_subscribe"
                | "hands_network_poll"
                | "hands_network_unsubscribe"
                | "hands_network_subscriptions"
                | "retry_click"
                | "file_upload"
                | "element_drag"
        )
}

fn validate_browser_binding(
    name: &str,
    args: &Value,
    monitor: &MonitorDescriptor,
) -> Result<(), Value> {
    if !visible_browser_call(name, args) {
        return Ok(());
    }
    let Some(title) = binding_browser_title() else {
        return Err(strict_error(
            "browser_unbound",
            format!(
                "'{name}' is a visible browser operation. Bind browser_window_title with hands_monitor_scope before using it under a strict fence."
            ),
            Some(monitor),
        ));
    };
    if !window_query_unique_in_monitor(&title, monitor) {
        return Err(strict_error(
            "browser_window_out_of_scope",
            format!("Bound browser window '{title}' is no longer on the scoped monitor"),
            Some(monitor),
        ));
    }
    Ok(())
}

fn prepare_script(args: &Value) -> Result<Value, Value> {
    let mut scoped = args.clone();
    let Some(steps) = scoped.get_mut("steps").and_then(Value::as_array_mut) else {
        return Ok(scoped);
    };
    for step in steps {
        let Some(tool) = step.get("tool").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        let step_args = step.get("args").cloned().unwrap_or_else(|| json!({}));
        let prepared = prepare_call(&tool, &step_args)?;
        if let Some(object) = step.as_object_mut() {
            object.insert("args".to_string(), prepared);
        }
    }
    Ok(scoped)
}

pub fn prepare_call(name: &str, args: &Value) -> Result<Value, Value> {
    if name == TOOL_NAME {
        return Ok(args.clone());
    }
    let Some(monitor) = active_monitor()? else {
        return Ok(args.clone());
    };

    if name == "hands_script" {
        return prepare_script(args);
    }

    validate_browser_binding(name, args, &monitor)?;
    validate_coordinate_pairs(name, args, &monitor)?;

    if matches!(name, "hands_plugin_load" | "hands_plugin_call") {
        return Err(strict_error(
            "unscoped_plugin_ability",
            format!(
                "'{name}' is disabled under a strict monitor fence because the native plugin ABI cannot prove monitor-scope enforcement"
            ),
            Some(&monitor),
        ));
    }

    if matches!(
        name,
        "uia_read_value"
            | "uia_watch"
            | "uia_poll_event"
            | "uia_poll_events"
            | "uia_hold_key"
            | "uia_app_launch"
    ) {
        return Err(strict_error(
            "unscopable_global_ability",
            format!(
                "'{name}' is disabled under a strict monitor fence; use a scoped read/action or hands_app_action open"
            ),
            Some(&monitor),
        ));
    }

    if matches!(name, "uia_type" | "uia_type_text")
        && (args.get("element_ref").is_some() || args.get("ref").is_some())
    {
        return Err(strict_error(
            "stale_ref_focus_risk",
            "UIA typing by cached ref is disabled under a strict fence; focus a uniquely titled scoped window and type through the foreground path",
            Some(&monitor),
        ));
    }

    if name == "uia_scroll" && (args.get("x").is_none() || args.get("y").is_none()) {
        return Err(strict_error(
            "cursor_global_scroll",
            "uia_scroll requires explicit in-scope x/y coordinates under a strict monitor fence",
            Some(&monitor),
        ));
    }

    if name == "uia_shortcut" {
        return Err(strict_error(
            "unscopable_global_input",
            "uia_shortcut is disabled under a strict monitor fence; focus a scoped window and use an explicit key operation",
            Some(&monitor),
        ));
    }

    if matches!(
        name,
        "uia_key_press" | "uia_hold_key" | "uia_type" | "uia_type_text" | "uia_scroll"
    ) && !foreground_in_monitor(&monitor)
    {
        return Err(strict_error(
            "foreground_out_of_scope",
            format!(
                "'{name}' requires the actual foreground window to be inside the monitor fence"
            ),
            Some(&monitor),
        ));
    }

    if name == "uia_click"
        && args.get("x").is_none()
        && args.get("element_ref").is_none()
        && args.get("ref").is_none()
        && !foreground_in_monitor(&monitor)
    {
        return Err(strict_error(
            "unscoped_click",
            "uia_click without coordinates or a scoped ref requires a foreground window inside the fence",
            Some(&monitor),
        ));
    }

    let rehome = name == "uia_window_move";
    if rehome
        && !window_selector_in_monitor(args, &monitor)
        && !args
            .get("allow_rehome")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Err(strict_error(
            "explicit_rehome_required",
            "Moving a window into the monitor fence requires allow_rehome=true and a unique title/handle",
            Some(&monitor),
        ));
    }
    let titled_window_action = name.starts_with("uia_window_")
        || matches!(
            name,
            "uia_focus_window"
                | "window_screenshot"
                | "vision_screenshot_hidden_window"
                | "type_into_window"
        );
    if titled_window_action
        && !rehome
        && (args.get("title").is_some()
            || args.get("window_title").is_some()
            || args.get("hwnd").is_some())
        && !window_selector_in_monitor(args, &monitor)
    {
        return Err(strict_error(
            "window_out_of_scope",
            format!("Target window for '{name}' is not owned by the scoped monitor"),
            Some(&monitor),
        ));
    }

    inject_monitor(args, &monitor)
}

pub fn prepare_vision_args(args: &Value) -> Result<Value, Value> {
    let Some(monitor) = active_monitor()? else {
        return Ok(args.clone());
    };
    inject_monitor(args, &monitor)
}

pub fn monitor_index(requested: usize) -> Result<usize, String> {
    match active_monitor() {
        Ok(Some(monitor)) => Ok(monitor.index),
        Ok(None) => Ok(requested),
        Err(error) => Err(error
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("monitor scope rejected capture")
            .to_string()),
    }
}

pub fn globalize_local_point(
    x: i64,
    y: i64,
    requested_monitor: usize,
) -> Result<(i64, i64), String> {
    let index = monitor_index(requested_monitor)?;
    let monitors =
        list_monitors().map_err(|error| format!("Could not enumerate monitors: {error}"))?;
    let monitor = monitors
        .get(index)
        .ok_or_else(|| format!("Monitor {index} is not available"))?;
    let scale = f64::from(monitor.scale_factor).max(0.01);
    let logical_x = (x as f64 / scale).round() as i64;
    let logical_y = (y as f64 / scale).round() as i64;
    Ok((logical_x + monitor.x as i64, logical_y + monitor.y as i64))
}

fn unique_capture_filename(
    prefix: &str,
    extension: &str,
    timestamp: &str,
    process_id: u32,
    counter: u64,
) -> String {
    format!(
        "{}_{}_{}_{}.{}",
        prefix,
        timestamp,
        process_id,
        counter,
        extension.trim_start_matches('.')
    )
}

pub fn unique_capture_path(prefix: &str, extension: &str, persistent: bool) -> String {
    let counter = PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%9f").to_string();
    let filename =
        unique_capture_filename(prefix, extension, &timestamp, std::process::id(), counter);
    let directory = if persistent {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Pictures")
            .join("Screenshots")
    } else {
        std::env::temp_dir().join("hands-captures")
    };
    let _ = std::fs::create_dir_all(&directory);
    directory.join(filename).to_string_lossy().into_owned()
}

fn filter_scoped_value(
    value: &mut Value,
    monitor: &MonitorDescriptor,
    refs: &mut HashSet<String>,
) -> bool {
    if let Some(rect) = value_rect(value) {
        if !rect_owned_by_monitor(rect, monitor) {
            return false;
        }
        if let Some(object) = value.as_object() {
            for key in ["element_ref", "ref", "a11y_ref"] {
                if let Some(reference) = object.get(key).and_then(Value::as_str) {
                    refs.insert(reference.to_string());
                }
            }
        }
    }

    match value {
        Value::Array(items) => {
            for item in items.iter_mut() {
                if !filter_scoped_value(item, monitor, refs) {
                    *item = Value::Null;
                }
            }
            items.retain(|item| !item.is_null());
        }
        Value::Object(object) => {
            for child in object.values_mut() {
                if matches!(child, Value::Array(_) | Value::Object(_))
                    && !filter_scoped_value(child, monitor, refs)
                {
                    *child = Value::Null;
                }
            }
        }
        _ => {}
    }
    true
}

pub fn filter_uia_result(name: &str, mut result: Value) -> Value {
    let monitor = match active_monitor() {
        Ok(Some(monitor)) => monitor,
        Ok(None) => return result,
        Err(error) => return error,
    };
    sync_safe_ref_monitor(&monitor);
    if matches!(name, "uia_get_state" | "uia_find" | "uia_find_element") {
        safe_refs()
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }
    let mut refs = HashSet::new();
    if !filter_scoped_value(&mut result, &monitor, &mut refs) {
        return strict_error(
            "uia_result_out_of_scope",
            "UIA result resolved outside the monitor fence",
            Some(&monitor),
        );
    }
    if let Some(object) = result.as_object_mut() {
        let filtered_count = ["windows", "elements", "items", "nodes"]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_array).map(Vec::len));
        if let Some(count) = filtered_count {
            object.insert("count".to_string(), json!(count));
        }
        object.insert("monitor_scope_applied".to_string(), json!(true));
        object.insert("monitor_index".to_string(), json!(monitor.index));
    }
    let mut known = safe_refs()
        .write()
        .unwrap_or_else(|error| error.into_inner());
    known.extend(refs);
    result
}

pub fn validate_scoped_ref(args: &Value) -> Result<(), Value> {
    let Some(monitor) = active_monitor()? else {
        return Ok(());
    };
    for key in ["element_ref", "ref"] {
        if let Some(reference) = args.get(key).and_then(Value::as_str) {
            if !safe_refs()
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .contains(reference)
            {
                return Err(strict_error(
                    "unknown_scoped_ref",
                    format!("UIA ref '{reference}' was not issued by a scoped discovery call"),
                    Some(&monitor),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn work_area_for_index(index: usize) -> Result<(i32, i32, i32, i32), Value> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    let monitors =
        list_monitors().map_err(|error| strict_error("enumeration_failed", error, None))?;
    let monitor = monitors
        .get(index)
        .ok_or_else(|| strict_error("monitor_missing", "Requested monitor was not found", None))?;
    let point = POINT {
        x: monitor.x + monitor.width as i32 / 2,
        y: monitor.y + monitor.height as i32 / 2,
    };
    let handle = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() {
        return Err(strict_error(
            "monitor_info_failed",
            "GetMonitorInfoW failed",
            Some(monitor),
        ));
    }
    Ok((
        info.rcWork.left,
        info.rcWork.top,
        info.rcWork.right,
        info.rcWork.bottom,
    ))
}

#[cfg(not(windows))]
pub fn work_area_for_index(_index: usize) -> Result<(i32, i32, i32, i32), Value> {
    Err(strict_error(
        "unsupported_platform",
        "Window placement is only available on Windows",
        None,
    ))
}

pub fn place_window(query: &str, index: usize) -> Value {
    if !window_query_is_unique(query) {
        return strict_error(
            "ambiguous_window",
            format!("Window query '{query}' must match exactly one top-level window"),
            None,
        );
    }
    let work = match work_area_for_index(index) {
        Ok(work) => work,
        Err(error) => return error,
    };
    let result = crate::handle_uia_window_move(&json!({
        "title": query,
        "x": work.0 + 16,
        "y": work.1 + 16
    }));
    if !result
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return result;
    }
    let monitor = match list_monitors()
        .ok()
        .and_then(|monitors| monitors.get(index).cloned())
    {
        Some(monitor) => monitor,
        None => {
            return strict_error(
                "monitor_missing",
                "Monitor disappeared during placement",
                None,
            )
        }
    };
    if !window_query_unique_in_monitor(query, &monitor) {
        return strict_error(
            "window_rehome_failed",
            format!("Window '{query}' did not land inside monitor {index}"),
            Some(&monitor),
        );
    }
    json!({"success": true, "monitor": index, "placement": result})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_paths_do_not_collide() {
        let first = unique_capture_path("screenshot", "png", false);
        let second = unique_capture_path("screenshot", "png", false);
        assert_ne!(first, second);
    }

    #[test]
    fn same_timestamp_same_process_uses_atomic_counter() {
        let timestamp = "20260714_205500_000000000";
        let first = unique_capture_filename("screenshot", "png", timestamp, 42, 7);
        let second = unique_capture_filename("screenshot", "png", timestamp, 42, 8);
        assert_ne!(first, second);
        assert!(first.ends_with("_42_7.png"));
        assert!(second.ends_with("_42_8.png"));
    }

    #[test]
    fn locked_without_explicit_scope_is_fail_closed() {
        for selector in ["", "off", " OFF "] {
            let state = inactive_selector_state(selector.trim(), true)
                .expect("inactive selectors must produce an explicit state");
            assert!(state.binding.is_none());
            assert!(state.config_error.is_some());
        }

        let unlocked = inactive_selector_state("off", false)
            .expect("inactive selectors must produce an explicit state");
        assert!(unlocked.binding.is_none());
        assert!(unlocked.config_error.is_none());
        assert!(inactive_selector_state("primary", true).is_none());
    }

    #[test]
    fn qr_browser_source_requires_browser_binding() {
        assert!(visible_browser_call(
            "hands_scan_qr",
            &json!({"source": "browser"})
        ));
        assert!(!visible_browser_call(
            "hands_scan_qr",
            &json!({"source": "screen"})
        ));
    }

    #[test]
    fn point_bounds_support_negative_monitor_origins() {
        let monitor = MonitorDescriptor {
            index: 3,
            display_id: 9,
            stable_id: "test-display-9".to_string(),
            stable_physical: true,
            x: -1427,
            y: -1080,
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
            rotation: 0.0,
            frequency: 60.0,
            is_primary: false,
        };
        assert!(point_inside(&monitor, -1400, -1000));
        assert!(!point_inside(&monitor, 600, 0));
    }
}
