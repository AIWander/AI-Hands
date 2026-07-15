use serde_json::{json, Value};

pub use uia_lib_backend::{Point, Rect};

pub fn get_tool_definitions() -> Vec<Value> {
    uia_lib_backend::get_tool_definitions()
}

fn title_for_hwnd(hwnd: &str) -> Option<String> {
    let windows = uia_lib_backend::handle_tool_call("uia_list_window", &json!({}));
    windows
        .get("windows")
        .and_then(Value::as_array)?
        .iter()
        .find(|window| window.get("hwnd").and_then(Value::as_str) == Some(hwnd))
        .and_then(|window| window.get("title").and_then(Value::as_str))
        .map(str::to_string)
}

fn normalize_snap_args(args: &Value) -> Value {
    let mut normalized = args.clone();
    let position = args
        .get("position")
        .or_else(|| args.get("direction"))
        .and_then(Value::as_str)
        .unwrap_or("left");
    if let Some(object) = normalized.as_object_mut() {
        object.insert("position".to_string(), json!(position));
        if object.get("title").is_none() {
            if let Some(hwnd) = object.get("hwnd").and_then(Value::as_str) {
                if let Some(title) = title_for_hwnd(hwnd) {
                    object.insert("title".to_string(), json!(title));
                }
            }
        }
    }
    normalized
}

pub fn handle_tool_call(name: &str, args: &Value) -> Value {
    let scoped = match crate::monitor_scope::prepare_call(name, args) {
        Ok(scoped) => scoped,
        Err(error) => return error,
    };
    if let Err(error) = crate::monitor_scope::validate_scoped_ref(&scoped) {
        return error;
    }

    let result = match name {
        "uia_window_move" => crate::handle_uia_window_move(&scoped),
        "uia_window_resize" => crate::handle_uia_window_resize(&scoped),
        "uia_window_state" => crate::handle_uia_window_state(&scoped),
        "uia_window_snap" => crate::handle_uia_window_snap(&normalize_snap_args(&scoped)),
        "uia_app_launch" => crate::handle_uia_app_launch(&scoped),
        _ => uia_lib_backend::handle_tool_call(name, &scoped),
    };

    crate::monitor_scope::filter_uia_result(name, result)
}
