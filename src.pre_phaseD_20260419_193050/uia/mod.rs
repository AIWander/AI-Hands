#![allow(non_upper_case_globals)]
#![allow(clippy::too_many_arguments)] // UIA automation functions require many params

use std::{collections::HashMap, thread, time::Duration};

use serde::Serialize;
use serde_json::{json, Value};
use uia_lib::{Point, Rect};

#[cfg(windows)]
mod ref_cache;

#[cfg(windows)]
use ref_cache::{CachedElement, CachedElementMeta};

#[cfg(windows)]
use windows::Win32::{
    Foundation::HWND,
    System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED},
    UI::{
        Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Children,
            UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId, UIA_ComboBoxControlTypeId,
            UIA_DataGridControlTypeId, UIA_DataItemControlTypeId, UIA_DocumentControlTypeId,
            UIA_EditControlTypeId, UIA_HyperlinkControlTypeId, UIA_ListControlTypeId,
            UIA_ListItemControlTypeId, UIA_MenuBarControlTypeId, UIA_MenuControlTypeId,
            UIA_MenuItemControlTypeId, UIA_RadioButtonControlTypeId, UIA_ScrollBarControlTypeId,
            UIA_SliderControlTypeId, UIA_SpinnerControlTypeId, UIA_TabControlTypeId,
            UIA_TabItemControlTypeId, UIA_TextControlTypeId, UIA_TreeControlTypeId,
            UIA_TreeItemControlTypeId, UIA_CONTROLTYPE_ID,
        },
        WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW},
    },
};

#[derive(Clone, Serialize)]
struct SnapshotNode {
    #[serde(rename = "ref")]
    ref_id: String,
    name: String,
    control_type: String,
    class_name: String,
    automation_id: String,
    bounding_rect: Rect,
    is_enabled: bool,
    is_visible: bool,
    center: Point,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<SnapshotNode>,
}

#[cfg(windows)]
struct ElementDetails {
    name: String,
    control_type_id: UIA_CONTROLTYPE_ID,
    control_type: String,
    class_name: String,
    automation_id: String,
    bounding_rect: Rect,
    is_enabled: bool,
    is_visible: bool,
    is_keyboard_focusable: bool,
    center: Point,
}

pub fn augment_tool_definition(tool: &mut Value) {
    let Some(name) = tool
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_string)
    else {
        return;
    };

    match name.as_str() {
        "uia_get_state" => {
            if let Some(tool_object) = tool.as_object_mut() {
                tool_object.insert(
                    "description".into(),
                    json!("Get the foreground window's desktop accessibility tree. Each returned node gets a stable ref ID (for example 'ref_12') that can be passed to uia_click or uia_type as element_ref. By default this only returns interactive elements to reduce token usage."),
                );
            }
            if let Some(properties) = tool
                .get_mut("inputSchema")
                .and_then(|value| value.get_mut("properties"))
                .and_then(|value| value.as_object_mut())
            {
                properties.insert(
                    "interactive_only".into(),
                    json!({
                        "type": "boolean",
                        "default": true,
                        "description": "If true, only return elements that can be clicked, typed into, expanded, or toggled. Static text and decorative nodes are skipped."
                    }),
                );
            }
        }
        "uia_click" => {
            if let Some(tool_object) = tool.as_object_mut() {
                tool_object.insert(
                    "description".into(),
                    json!("Click by cached element_ref from the last uia_get_state snapshot, or fall back to exact screen coordinates."),
                );
            }
            if let Some(properties) = tool
                .get_mut("inputSchema")
                .and_then(|value| value.get_mut("properties"))
                .and_then(|value| value.as_object_mut())
            {
                properties.insert(
                    "element_ref".into(),
                    json!({
                        "type": "string",
                        "description": "Ref ID from the last uia_get_state snapshot (for example 'ref_12'). Preferred over raw coordinates."
                    }),
                );
            }
            if let Some(schema) = tool
                .get_mut("inputSchema")
                .and_then(|value| value.as_object_mut())
            {
                schema.remove("required");
            }
        }
        "uia_type" => {
            if let Some(tool_object) = tool.as_object_mut() {
                tool_object.insert(
                    "description".into(),
                    json!("Type text into a cached element_ref from the last uia_get_state snapshot, or into the currently focused element."),
                );
            }
            if let Some(properties) = tool
                .get_mut("inputSchema")
                .and_then(|value| value.get_mut("properties"))
                .and_then(|value| value.as_object_mut())
            {
                properties.insert(
                    "element_ref".into(),
                    json!({
                        "type": "string",
                        "description": "Ref ID from the last uia_get_state snapshot (for example 'ref_12'). When present, Hands focuses that element before typing."
                    }),
                );
            }
        }
        _ => {}
    }
}

#[cfg(windows)]
pub fn handle_get_state(args: &Value) -> Value {
    let max_depth = args
        .get("max_depth")
        .and_then(|value| value.as_u64())
        .unwrap_or(3) as u32;
    let include_invisible = args
        .get("include_invisible")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let interactive_only = args
        .get("interactive_only")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);

    match collect_snapshot(max_depth, include_invisible, interactive_only) {
        Ok(snapshot) => {
            let ref_count = snapshot.elements.len();
            json!({
                "success": true,
                "window": {
                    "title": snapshot.window_title,
                    "hwnd": snapshot.hwnd_label,
                },
                "count": ref_count,
                "ref_count": ref_count,
                "interactive_only": interactive_only,
                "tree": snapshot.tree,
                "elements": snapshot.elements,
                "hint": "Pass a node's ref value as element_ref to uia_click or uia_type. Refs are refreshed on every new uia_get_state snapshot."
            })
        }
        Err(error) => json!({
            "success": false,
            "error": error,
        }),
    }
}

#[cfg(not(windows))]
pub fn handle_get_state(_args: &Value) -> Value {
    json!({
        "success": false,
        "error": "UI automation only available on Windows"
    })
}

#[cfg(windows)]
pub fn handle_click(args: &Value) -> Value {
    let Some(ref_id) = args.get("element_ref").and_then(|value| value.as_str()) else {
        return uia_lib::handle_tool_call("uia_click", args);
    };

    let button = args
        .get("button")
        .and_then(|value| value.as_str())
        .unwrap_or("left");
    let double_click = args
        .get("double_click")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    match ref_cache::resolve_ref(ref_id) {
        Ok(cached) => {
            let target = current_center(&cached.element).unwrap_or(cached.meta.center);
            let mut result = uia_lib::handle_tool_call(
                "uia_click",
                &json!({
                    "x": target.x,
                    "y": target.y,
                    "button": button,
                    "double_click": double_click,
                }),
            );
            if let Some(object) = result.as_object_mut() {
                object.insert("element_ref".into(), json!(ref_id));
                object.insert("resolved_element".into(), json!(cached.meta));
            }
            result
        }
        Err(error) => json!({
            "success": false,
            "error": error,
        }),
    }
}

#[cfg(not(windows))]
pub fn handle_click(args: &Value) -> Value {
    uia_lib::handle_tool_call("uia_click", args)
}

#[cfg(windows)]
pub fn handle_type(args: &Value) -> Value {
    let Some(ref_id) = args.get("element_ref").and_then(|value| value.as_str()) else {
        return uia_lib::handle_tool_call("uia_type_text", args);
    };

    let text = match args.get("text").and_then(|value| value.as_str()) {
        Some(value) => value,
        None => {
            return json!({
                "success": false,
                "error": "text parameter required"
            });
        }
    };

    match ref_cache::resolve_ref(ref_id) {
        Ok(cached) => {
            let focus_result = focus_element(&cached.element).unwrap_or_else(|error| {
                let click_fallback = uia_lib::handle_tool_call(
                    "uia_click",
                    &json!({
                        "x": cached.meta.center.x,
                        "y": cached.meta.center.y,
                    }),
                );
                json!({
                    "success": click_fallback.get("success").and_then(|value| value.as_bool()).unwrap_or(false),
                    "mode": "click_fallback",
                    "error": error,
                    "click_result": click_fallback,
                })
            });

            thread::sleep(Duration::from_millis(50));

            let mut result = uia_lib::handle_tool_call("uia_type_text", &json!({ "text": text }));
            if let Some(object) = result.as_object_mut() {
                object.insert("element_ref".into(), json!(ref_id));
                object.insert("focus_result".into(), focus_result);
                object.insert("resolved_element".into(), json!(cached.meta));
            }
            result
        }
        Err(error) => json!({
            "success": false,
            "error": error,
        }),
    }
}

#[cfg(not(windows))]
pub fn handle_type(args: &Value) -> Value {
    uia_lib::handle_tool_call("uia_type_text", args)
}

#[cfg(windows)]
struct SnapshotResult {
    window_title: String,
    hwnd_label: String,
    tree: Vec<SnapshotNode>,
    elements: Vec<CachedElementMeta>,
}

#[cfg(windows)]
fn collect_snapshot(
    max_depth: u32,
    include_invisible: bool,
    interactive_only: bool,
) -> Result<SnapshotResult, String> {
    let automation = get_ui_automation()?;
    let (hwnd, window_title, hwnd_label) = foreground_window_info()?;
    let root = unsafe {
        automation
            .ElementFromHandle(hwnd)
            .map_err(|error| format!("Failed to bind UIA to the foreground window: {}", error))?
    };

    let mut refs = HashMap::new();
    let mut flat_elements = Vec::new();
    let mut counter = 0usize;
    let tree = collect_nodes(
        &root,
        &automation,
        0,
        max_depth,
        include_invisible,
        interactive_only,
        &mut counter,
        &mut refs,
        &mut flat_elements,
    );

    ref_cache::store_snapshot(window_title.clone(), hwnd_label.clone(), refs);

    Ok(SnapshotResult {
        window_title,
        hwnd_label,
        tree,
        elements: flat_elements,
    })
}

#[cfg(windows)]
fn collect_nodes(
    element: &IUIAutomationElement,
    automation: &IUIAutomation,
    depth: u32,
    max_depth: u32,
    include_invisible: bool,
    interactive_only: bool,
    counter: &mut usize,
    refs: &mut HashMap<String, CachedElement>,
    flat_elements: &mut Vec<CachedElementMeta>,
) -> Vec<SnapshotNode> {
    if depth > max_depth {
        return Vec::new();
    }

    let Some(details) = element_details(element) else {
        return Vec::new();
    };

    let mut children = Vec::new();
    if depth < max_depth {
        children = collect_children(
            element,
            automation,
            depth + 1,
            max_depth,
            include_invisible,
            interactive_only,
            counter,
            refs,
            flat_elements,
        );
    }

    let include_self = (include_invisible || details.is_visible)
        && if interactive_only {
            is_interactive(&details)
        } else {
            true
        };

    if !include_self {
        return children;
    }

    let ref_id = format!("ref_{}", *counter);
    *counter += 1;

    let meta = CachedElementMeta {
        ref_id: ref_id.clone(),
        name: details.name,
        control_type: details.control_type,
        class_name: details.class_name,
        automation_id: details.automation_id,
        bounding_rect: details.bounding_rect,
        is_enabled: details.is_enabled,
        is_visible: details.is_visible,
        center: details.center,
    };

    refs.insert(
        ref_id.clone(),
        CachedElement {
            element: element.clone(),
            meta: meta.clone(),
        },
    );
    flat_elements.push(meta.clone());

    vec![SnapshotNode {
        ref_id,
        name: meta.name.clone(),
        control_type: meta.control_type.clone(),
        class_name: meta.class_name.clone(),
        automation_id: meta.automation_id.clone(),
        bounding_rect: meta.bounding_rect,
        is_enabled: meta.is_enabled,
        is_visible: meta.is_visible,
        center: meta.center,
        children,
    }]
}

#[cfg(windows)]
fn collect_children(
    element: &IUIAutomationElement,
    automation: &IUIAutomation,
    depth: u32,
    max_depth: u32,
    include_invisible: bool,
    interactive_only: bool,
    counter: &mut usize,
    refs: &mut HashMap<String, CachedElement>,
    flat_elements: &mut Vec<CachedElementMeta>,
) -> Vec<SnapshotNode> {
    let mut nodes = Vec::new();

    unsafe {
        let Ok(condition) = automation.CreateTrueCondition() else {
            return nodes;
        };
        let Ok(children) = element.FindAll(TreeScope_Children, &condition) else {
            return nodes;
        };
        let Ok(length) = children.Length() else {
            return nodes;
        };

        for index in 0..length {
            if let Ok(child) = children.GetElement(index) {
                nodes.extend(collect_nodes(
                    &child,
                    automation,
                    depth,
                    max_depth,
                    include_invisible,
                    interactive_only,
                    counter,
                    refs,
                    flat_elements,
                ));
            }
        }
    }

    nodes
}

#[cfg(windows)]
fn element_details(element: &IUIAutomationElement) -> Option<ElementDetails> {
    unsafe {
        let control_type_id = element.CurrentControlType().ok()?;
        let rect_raw = element.CurrentBoundingRectangle().ok()?;
        let width = (rect_raw.right - rect_raw.left).max(0);
        let height = (rect_raw.bottom - rect_raw.top).max(0);
        let bounding_rect = Rect {
            x: rect_raw.left,
            y: rect_raw.top,
            width,
            height,
        };
        let is_offscreen = element
            .CurrentIsOffscreen()
            .ok()
            .map(|value| value.as_bool())
            .unwrap_or(false);
        let is_enabled = element
            .CurrentIsEnabled()
            .ok()
            .map(|value| value.as_bool())
            .unwrap_or(false);
        let is_keyboard_focusable = element
            .CurrentIsKeyboardFocusable()
            .ok()
            .map(|value| value.as_bool())
            .unwrap_or(false);

        Some(ElementDetails {
            name: element
                .CurrentName()
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_default(),
            control_type_id,
            control_type: control_type_to_string(control_type_id),
            class_name: element
                .CurrentClassName()
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_default(),
            automation_id: element
                .CurrentAutomationId()
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_default(),
            bounding_rect,
            is_enabled,
            is_visible: !is_offscreen && width > 0 && height > 0,
            is_keyboard_focusable,
            center: Point {
                x: rect_raw.left + width / 2,
                y: rect_raw.top + height / 2,
            },
        })
    }
}

#[cfg(windows)]
fn is_interactive(details: &ElementDetails) -> bool {
    matches!(
        details.control_type_id,
        UIA_ButtonControlTypeId
            | UIA_CheckBoxControlTypeId
            | UIA_ComboBoxControlTypeId
            | UIA_DataGridControlTypeId
            | UIA_DataItemControlTypeId
            | UIA_DocumentControlTypeId
            | UIA_EditControlTypeId
            | UIA_HyperlinkControlTypeId
            | UIA_ListControlTypeId
            | UIA_ListItemControlTypeId
            | UIA_MenuControlTypeId
            | UIA_MenuBarControlTypeId
            | UIA_MenuItemControlTypeId
            | UIA_RadioButtonControlTypeId
            | UIA_ScrollBarControlTypeId
            | UIA_SliderControlTypeId
            | UIA_SpinnerControlTypeId
            | UIA_TabControlTypeId
            | UIA_TabItemControlTypeId
            | UIA_TreeControlTypeId
            | UIA_TreeItemControlTypeId
    ) || details.is_keyboard_focusable
}

#[cfg(windows)]
fn control_type_to_string(control_type_id: UIA_CONTROLTYPE_ID) -> String {
    match control_type_id {
        UIA_ButtonControlTypeId => "Button",
        UIA_CheckBoxControlTypeId => "CheckBox",
        UIA_ComboBoxControlTypeId => "ComboBox",
        UIA_DataGridControlTypeId => "DataGrid",
        UIA_DataItemControlTypeId => "DataItem",
        UIA_DocumentControlTypeId => "Document",
        UIA_EditControlTypeId => "Edit",
        UIA_HyperlinkControlTypeId => "Link",
        UIA_ListControlTypeId => "List",
        UIA_ListItemControlTypeId => "ListItem",
        UIA_MenuControlTypeId => "Menu",
        UIA_MenuBarControlTypeId => "MenuBar",
        UIA_MenuItemControlTypeId => "MenuItem",
        UIA_RadioButtonControlTypeId => "RadioButton",
        UIA_ScrollBarControlTypeId => "ScrollBar",
        UIA_SliderControlTypeId => "Slider",
        UIA_SpinnerControlTypeId => "Spinner",
        UIA_TabControlTypeId => "Tab",
        UIA_TabItemControlTypeId => "TabItem",
        UIA_TextControlTypeId => "Text",
        UIA_TreeControlTypeId => "Tree",
        UIA_TreeItemControlTypeId => "TreeItem",
        _ => "Unknown",
    }
    .to_string()
}

#[cfg(windows)]
fn foreground_window_info() -> Result<(HWND, String, String), String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return Err("No foreground window is active.".to_string());
        }

        let mut title = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut title);
        let window_title = String::from_utf16_lossy(&title[..len as usize]);
        let hwnd_label = format!("0x{:X}", hwnd.0 as usize);

        Ok((hwnd, window_title, hwnd_label))
    }
}

#[cfg(windows)]
fn get_ui_automation() -> Result<IUIAutomation, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            .map_err(|error| format!("Failed to create UI Automation client: {}", error))
    }
}

#[cfg(windows)]
fn current_center(element: &IUIAutomationElement) -> Option<Point> {
    unsafe {
        let rect = element.CurrentBoundingRectangle().ok()?;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return None;
        }

        Some(Point {
            x: rect.left + width / 2,
            y: rect.top + height / 2,
        })
    }
}

#[cfg(windows)]
fn focus_element(element: &IUIAutomationElement) -> Result<Value, String> {
    unsafe {
        element
            .SetFocus()
            .map_err(|error| format!("SetFocus failed: {}", error))?;
    }

    Ok(json!({
        "success": true,
        "mode": "uia_set_focus"
    }))
}
