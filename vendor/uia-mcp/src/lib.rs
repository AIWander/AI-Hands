//! UIA Library — Windows UI Automation logic
//! Used by uia-mcp (standalone) and hands (unified server)

// Allow Windows API naming conventions (UIA_ButtonControlTypeId etc.)
#![allow(non_upper_case_globals)]

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use windows::{
    core::*, Win32::Foundation::*, Win32::System::Com::*, Win32::UI::Accessibility::*,
    Win32::UI::Input::KeyboardAndMouse::*, Win32::UI::WindowsAndMessaging::*,
};
// ============ DATA TYPES ============

#[derive(Serialize, Clone)]
pub struct UiElement {
    pub name: String,
    pub control_type: String,
    pub class_name: String,
    pub automation_id: String,
    pub bounding_rect: Rect,
    pub is_enabled: bool,
    pub is_visible: bool,
    pub center: Point,
}

#[derive(Serialize, Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Serialize, Clone, Copy)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Serialize)]
pub struct WindowInfo {
    pub hwnd: String,
    pub title: String,
    pub class_name: String,
    pub rect: Rect,
    pub is_visible: bool,
}

// ============ WINDOWS UIA IMPLEMENTATION ============

#[cfg(windows)]
fn get_ui_automation() -> Result<IUIAutomation> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
    }
}

#[cfg(windows)]
fn element_to_ui_element(element: &IUIAutomationElement) -> Option<UiElement> {
    unsafe {
        let name = element.CurrentName().ok()?.to_string();
        let control_type_id = element.CurrentControlType().ok()?;
        let class_name = element
            .CurrentClassName()
            .ok()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let automation_id = element
            .CurrentAutomationId()
            .ok()
            .map(|s| s.to_string())
            .unwrap_or_default();

        let rect_raw = element.CurrentBoundingRectangle().ok()?;
        let rect = Rect {
            x: rect_raw.left,
            y: rect_raw.top,
            width: rect_raw.right - rect_raw.left,
            height: rect_raw.bottom - rect_raw.top,
        };

        let is_enabled = element.CurrentIsEnabled().ok()?.as_bool();
        let is_offscreen = element.CurrentIsOffscreen().ok()?.as_bool();

        let control_type = control_type_to_string(control_type_id);

        Some(UiElement {
            name,
            control_type,
            class_name,
            automation_id,
            bounding_rect: rect,
            is_enabled,
            is_visible: !is_offscreen && rect.width > 0 && rect.height > 0,
            center: Point {
                x: rect.x + rect.width / 2,
                y: rect.y + rect.height / 2,
            },
        })
    }
}

#[cfg(windows)]
fn control_type_to_string(ct: UIA_CONTROLTYPE_ID) -> String {
    match ct {
        UIA_ButtonControlTypeId => "Button",
        UIA_CalendarControlTypeId => "Calendar",
        UIA_CheckBoxControlTypeId => "CheckBox",
        UIA_ComboBoxControlTypeId => "ComboBox",
        UIA_EditControlTypeId => "Edit",
        UIA_HyperlinkControlTypeId => "Hyperlink",
        UIA_ImageControlTypeId => "Image",
        UIA_ListItemControlTypeId => "ListItem",
        UIA_ListControlTypeId => "List",
        UIA_MenuControlTypeId => "Menu",
        UIA_MenuBarControlTypeId => "MenuBar",
        UIA_MenuItemControlTypeId => "MenuItem",
        UIA_ProgressBarControlTypeId => "ProgressBar",
        UIA_RadioButtonControlTypeId => "RadioButton",
        UIA_ScrollBarControlTypeId => "ScrollBar",
        UIA_SliderControlTypeId => "Slider",
        UIA_SpinnerControlTypeId => "Spinner",
        UIA_StatusBarControlTypeId => "StatusBar",
        UIA_TabControlTypeId => "Tab",
        UIA_TabItemControlTypeId => "TabItem",
        UIA_TextControlTypeId => "Text",
        UIA_ToolBarControlTypeId => "ToolBar",
        UIA_ToolTipControlTypeId => "ToolTip",
        UIA_TreeControlTypeId => "Tree",
        UIA_TreeItemControlTypeId => "TreeItem",
        UIA_WindowControlTypeId => "Window",
        UIA_PaneControlTypeId => "Pane",
        UIA_GroupControlTypeId => "Group",
        UIA_DocumentControlTypeId => "Document",
        UIA_DataGridControlTypeId => "DataGrid",
        UIA_DataItemControlTypeId => "DataItem",
        UIA_TitleBarControlTypeId => "TitleBar",
        UIA_HeaderControlTypeId => "Header",
        UIA_HeaderItemControlTypeId => "HeaderItem",
        UIA_TableControlTypeId => "Table",
        UIA_ThumbControlTypeId => "Thumb",
        UIA_SeparatorControlTypeId => "Separator",
        UIA_SemanticZoomControlTypeId => "SemanticZoom",
        UIA_AppBarControlTypeId => "AppBar",
        _ => "Unknown",
    }
    .to_string()
}

#[cfg(windows)]
fn collect_elements(
    element: &IUIAutomationElement,
    automation: &IUIAutomation,
    depth: u32,
    max_depth: u32,
    include_invisible: bool,
) -> Vec<UiElement> {
    let mut elements = Vec::new();

    if depth > max_depth {
        return elements;
    }

    if let Some(ui_elem) = element_to_ui_element(element) {
        if include_invisible || ui_elem.is_visible {
            elements.push(ui_elem);
        }
    }

    unsafe {
        if let Ok(condition) = automation.CreateTrueCondition() {
            if let Ok(children) = element.FindAll(TreeScope_Children, &condition) {
                if let Ok(length) = children.Length() {
                    for i in 0..length {
                        if let Ok(child) = children.GetElement(i) {
                            elements.extend(collect_elements(
                                &child,
                                automation,
                                depth + 1,
                                max_depth,
                                include_invisible,
                            ));
                        }
                    }
                }
            }
        }
    }

    elements
}

#[cfg(windows)]
fn get_visible_windows() -> Vec<WindowInfo> {
    let mut windows = Vec::new();

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut windows as *mut _ as isize),
        );
    }

    windows
}

#[cfg(windows)]
unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows = &mut *(lparam.0 as *mut Vec<WindowInfo>);

    if IsWindowVisible(hwnd).as_bool() {
        let mut title = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut title);
        let title = String::from_utf16_lossy(&title[..len as usize]);

        if !title.is_empty() {
            let mut class_name = [0u16; 256];
            let class_len = GetClassNameW(hwnd, &mut class_name);
            let class_name = String::from_utf16_lossy(&class_name[..class_len as usize]);

            let mut rect = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rect);

            windows.push(WindowInfo {
                hwnd: format!("{:?}", hwnd),
                title,
                class_name,
                rect: Rect {
                    x: rect.left,
                    y: rect.top,
                    width: rect.right - rect.left,
                    height: rect.bottom - rect.top,
                },
                is_visible: true,
            });
        }
    }

    TRUE
}

// ============ ACTION IMPLEMENTATIONS ============

#[cfg(windows)]
fn uia_click(args: &Value) -> Value {
    let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let button = args
        .get("button")
        .and_then(|v| v.as_str())
        .unwrap_or("left");
    let double_click = args
        .get("double_click")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;

        // Move cursor
        let _ = SetCursorPos(x, y);
        std::thread::sleep(std::time::Duration::from_millis(50));

        let (down_flag, up_flag) = match button {
            "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            _ => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        };

        let clicks = if double_click { 2 } else { 1 };
        for _ in 0..clicks {
            let inputs = [
                INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: 0,
                            dwFlags: down_flag,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: 0,
                            dwFlags: up_flag,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
            ];
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            if double_click {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }

    json!({
        "success": true,
        "clicked": {"x": x, "y": y},
        "button": button,
        "double_click": double_click
    })
}

#[cfg(windows)]
fn uia_type_text(args: &Value) -> Value {
    let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");

    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;

        for ch in text.encode_utf16() {
            let inputs = [
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: ch,
                            dwFlags: KEYEVENTF_UNICODE,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: ch,
                            dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
            ];
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    json!({
        "success": true,
        "typed": text,
        "length": text.len()
    })
}

#[cfg(windows)]
fn uia_focus_window(args: &Value) -> Value {
    let title_filter = args.get("title").and_then(|v| v.as_str());

    if title_filter.is_none() {
        return json!({"success": false, "error": "title parameter required"});
    }
    let filter = title_filter.unwrap().to_lowercase();

    let windows = get_visible_windows();
    let found = windows
        .iter()
        .find(|w| w.title.to_lowercase().contains(&filter));

    match found {
        Some(win) => {
            // Parse hwnd back from the debug string format "HWND(0x...)"
            let hwnd_str = &win.hwnd;
            let hwnd_val = if hwnd_str.contains("0x") {
                let hex = hwnd_str.trim_start_matches("HWND(").trim_end_matches(')');
                usize::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap_or(0)
            } else {
                0
            };

            if hwnd_val != 0 {
                unsafe {
                    let hwnd = HWND(hwnd_val as *mut _);
                    let _ = SetForegroundWindow(hwnd);
                    let _ = SetFocus(hwnd);
                }
                json!({
                    "success": true,
                    "focused": win.title,
                    "hwnd": win.hwnd
                })
            } else {
                json!({"success": false, "error": "Could not parse window handle"})
            }
        }
        None => json!({
            "success": false,
            "error": format!("No window found matching '{}'", filter),
            "available": windows.iter().map(|w| &w.title).collect::<Vec<_>>()
        }),
    }
}

#[cfg(not(windows))]
fn uia_click(_args: &Value) -> Value {
    json!({"success": false, "error": "Click only available on Windows"})
}

#[cfg(not(windows))]
fn uia_type_text(_args: &Value) -> Value {
    json!({"success": false, "error": "Type only available on Windows"})
}

#[cfg(not(windows))]
fn uia_focus_window(_args: &Value) -> Value {
    json!({"success": false, "error": "Focus only available on Windows"})
}

// ============ KEY PRESS / SHORTCUTS ============

#[cfg(windows)]
fn key_name_to_vk(name: &str) -> Option<VIRTUAL_KEY> {
    Some(match name.to_lowercase().as_str() {
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "escape" | "esc" => VK_ESCAPE,
        "space" => VK_SPACE,
        "backspace" => VK_BACK,
        "delete" | "del" => VK_DELETE,
        "insert" | "ins" => VK_INSERT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "pgup" => VK_PRIOR,
        "pagedown" | "pgdn" => VK_NEXT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "f1" => VK_F1,
        "f2" => VK_F2,
        "f3" => VK_F3,
        "f4" => VK_F4,
        "f5" => VK_F5,
        "f6" => VK_F6,
        "f7" => VK_F7,
        "f8" => VK_F8,
        "f9" => VK_F9,
        "f10" => VK_F10,
        "f11" => VK_F11,
        "f12" => VK_F12,
        "capslock" => VK_CAPITAL,
        "numlock" => VK_NUMLOCK,
        "scrolllock" => VK_SCROLL,
        "printscreen" | "prtsc" => VK_SNAPSHOT,
        "pause" => VK_PAUSE,
        "apps" | "menu" => VK_APPS,
        // Single character a-z, 0-9
        s if s.len() == 1 => {
            let c = s.chars().next().unwrap();
            match c {
                'a'..='z' => VIRTUAL_KEY(c.to_ascii_uppercase() as u16),
                '0'..='9' => VIRTUAL_KEY(c as u16),
                _ => return None,
            }
        }
        _ => return None,
    })
}

#[cfg(windows)]
fn modifier_to_vk(name: &str) -> Option<VIRTUAL_KEY> {
    Some(match name.to_lowercase().as_str() {
        "ctrl" | "control" => VK_CONTROL,
        "shift" => VK_SHIFT,
        "alt" => VK_MENU,
        "win" | "super" | "meta" => VK_LWIN,
        _ => return None,
    })
}

#[cfg(windows)]
fn send_key_combo(modifiers: &[VIRTUAL_KEY], key: VIRTUAL_KEY) {
    unsafe {
        let mut inputs: Vec<INPUT> = Vec::new();

        // Press modifiers down
        for m in modifiers {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: *m,
                        wScan: 0,
                        dwFlags: KEYBD_EVENT_FLAGS(0),
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }

        // Press main key down
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });

        // Release main key
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });

        // Release modifiers in reverse
        for m in modifiers.iter().rev() {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: *m,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }

        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn send_key_hold(modifiers: &[VIRTUAL_KEY], key: VIRTUAL_KEY, duration_ms: u64) {
    unsafe {
        // Phase 1: press modifiers then main key down
        let mut down_inputs: Vec<INPUT> = Vec::new();
        for m in modifiers {
            down_inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: *m,
                        wScan: 0,
                        dwFlags: KEYBD_EVENT_FLAGS(0),
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
        down_inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
        SendInput(&down_inputs, std::mem::size_of::<INPUT>() as i32);

        // Phase 2: hold
        std::thread::sleep(std::time::Duration::from_millis(duration_ms));

        // Phase 3: release main key then modifiers (reverse order)
        let mut up_inputs: Vec<INPUT> = Vec::new();
        up_inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
        for m in modifiers.iter().rev() {
            up_inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: *m,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
        SendInput(&up_inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn uia_key_press(args: &Value) -> Value {
    let keys_str = match args.get("keys").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => {
            return json!({"success": false, "error": "keys parameter required (e.g. 'ctrl+s', 'enter', 'alt+f4')"})
        }
    };

    let parts: Vec<&str> = keys_str.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return json!({"success": false, "error": "Empty key combination"});
    }

    let main_key_name = parts.last().unwrap();
    let modifier_names = &parts[..parts.len() - 1];

    // Resolve modifiers
    let mut modifiers = Vec::new();
    for m in modifier_names {
        match modifier_to_vk(m) {
            Some(vk) => modifiers.push(vk),
            None => {
                return json!({"success": false, "error": format!("Unknown modifier: '{}'. Use ctrl, shift, alt, win", m)})
            }
        }
    }

    // Resolve main key
    let main_vk = match key_name_to_vk(main_key_name) {
        Some(vk) => vk,
        None => {
            return json!({"success": false, "error": format!("Unknown key: '{}'. Use named keys (enter, tab, f1-f12, etc.) or single chars (a-z, 0-9)", main_key_name)})
        }
    };

    send_key_combo(&modifiers, main_vk);

    json!({
        "success": true,
        "keys": keys_str,
        "modifiers": modifier_names.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "main_key": main_key_name
    })
}

#[cfg(windows)]
fn uia_hold_key(args: &Value) -> Value {
    let keys_str = match args.get("keys").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => {
            return json!({"success": false, "error": "keys parameter required (e.g. 'shift', 'ctrl+a', 'space')"})
        }
    };

    let duration_ms = args
        .get("duration_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(1000);
    if duration_ms == 0 {
        return json!({"success": false, "error": "duration_ms must be > 0"});
    }
    if duration_ms > 60_000 {
        return json!({"success": false, "error": "duration_ms capped at 60000 (60s)"});
    }

    let parts: Vec<&str> = keys_str.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return json!({"success": false, "error": "Empty key combination"});
    }

    let main_key_name = parts.last().unwrap();
    let modifier_names = &parts[..parts.len() - 1];

    let mut modifiers = Vec::new();
    for m in modifier_names {
        match modifier_to_vk(m) {
            Some(vk) => modifiers.push(vk),
            None => {
                return json!({"success": false, "error": format!("Unknown modifier: '{}'. Use ctrl, shift, alt, win", m)})
            }
        }
    }

    let main_vk = match key_name_to_vk(main_key_name) {
        Some(vk) => vk,
        None => {
            return json!({"success": false, "error": format!("Unknown key: '{}'. Use named keys (enter, tab, f1-f12, etc.) or single chars (a-z, 0-9)", main_key_name)})
        }
    };

    send_key_hold(&modifiers, main_vk, duration_ms);

    json!({
        "success": true,
        "keys": keys_str,
        "duration_ms": duration_ms,
        "modifiers": modifier_names.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "main_key": main_key_name
    })
}

/// Built-in shortcut map for common apps
#[cfg(windows)]
fn get_app_shortcuts() -> Value {
    json!({
        "vscode": {
            "command_palette": "ctrl+shift+p",
            "quick_open": "ctrl+p",
            "terminal": "ctrl+`",
            "save": "ctrl+s",
            "save_all": "ctrl+k s",
            "close_tab": "ctrl+w",
            "find": "ctrl+f",
            "find_replace": "ctrl+h",
            "find_in_files": "ctrl+shift+f",
            "go_to_line": "ctrl+g",
            "toggle_sidebar": "ctrl+b",
            "new_file": "ctrl+n",
            "split_editor": "ctrl+\\",
            "zen_mode": "ctrl+k z",
            "settings": "ctrl+,",
            "explorer": "ctrl+shift+e",
            "source_control": "ctrl+shift+g",
            "extensions": "ctrl+shift+x",
            "debug": "ctrl+shift+d",
            "problems": "ctrl+shift+m",
            "output": "ctrl+shift+u"
        },
        "cursor": {
            "command_palette": "ctrl+shift+p",
            "ai_chat": "ctrl+l",
            "ai_edit": "ctrl+k",
            "quick_open": "ctrl+p",
            "terminal": "ctrl+`",
            "save": "ctrl+s",
            "find": "ctrl+f",
            "find_in_files": "ctrl+shift+f"
        },
        "chrome": {
            "new_tab": "ctrl+t",
            "close_tab": "ctrl+w",
            "reopen_tab": "ctrl+shift+t",
            "address_bar": "ctrl+l",
            "find": "ctrl+f",
            "refresh": "f5",
            "hard_refresh": "ctrl+shift+r",
            "devtools": "f12",
            "devtools_alt": "ctrl+shift+i",
            "next_tab": "ctrl+tab",
            "prev_tab": "ctrl+shift+tab",
            "bookmark": "ctrl+d",
            "history": "ctrl+h",
            "downloads": "ctrl+j",
            "zoom_in": "ctrl+=",
            "zoom_out": "ctrl+-",
            "zoom_reset": "ctrl+0"
        },
        "explorer": {
            "new_folder": "ctrl+shift+n",
            "rename": "f2",
            "delete": "delete",
            "properties": "alt+enter",
            "address_bar": "alt+d",
            "search": "ctrl+e",
            "select_all": "ctrl+a",
            "copy": "ctrl+c",
            "paste": "ctrl+v",
            "cut": "ctrl+x",
            "undo": "ctrl+z"
        },
        "windows": {
            "run": "win+r",
            "settings": "win+i",
            "lock": "win+l",
            "desktop": "win+d",
            "task_view": "win+tab",
            "snap_left": "win+left",
            "snap_right": "win+right",
            "maximize": "win+up",
            "minimize": "win+down",
            "screenshot": "win+shift+s",
            "clipboard": "win+v",
            "emoji": "win+.",
            "task_manager": "ctrl+shift+escape",
            "file_explorer": "win+e",
            "switch_app": "alt+tab",
            "close_window": "alt+f4",
            "system_info": "win+pause"
        },
        "notepad": {
            "new": "ctrl+n",
            "open": "ctrl+o",
            "save": "ctrl+s",
            "save_as": "ctrl+shift+s",
            "find": "ctrl+f",
            "replace": "ctrl+h",
            "go_to": "ctrl+g",
            "select_all": "ctrl+a",
            "zoom_in": "ctrl+=",
            "zoom_out": "ctrl+-"
        },
        "terminal": {
            "copy": "ctrl+c",
            "paste": "ctrl+v",
            "new_tab": "ctrl+shift+t",
            "close_tab": "ctrl+shift+w",
            "find": "ctrl+shift+f",
            "split_pane": "alt+shift+=",
            "next_tab": "ctrl+tab",
            "prev_tab": "ctrl+shift+tab",
            "settings": "ctrl+,"
        }
    })
}

#[cfg(windows)]
fn uia_shortcut(args: &Value) -> Value {
    let app = args.get("app").and_then(|v| v.as_str());
    let action = args.get("action").and_then(|v| v.as_str());

    let shortcuts = get_app_shortcuts();

    // If no app specified, list available apps
    if app.is_none() {
        let apps: Vec<&str> = shortcuts
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        return json!({
            "success": true,
            "mode": "list_apps",
            "available_apps": apps,
            "hint": "Pass app and action to execute a shortcut, or just app to list its shortcuts"
        });
    }

    let app_name = app.unwrap().to_lowercase();
    let app_shortcuts = match shortcuts.get(&app_name) {
        Some(s) => s,
        None => {
            return json!({
                "success": false,
                "error": format!("Unknown app '{}'. Available: {:?}", app_name,
                    shortcuts.as_object().unwrap().keys().collect::<Vec<_>>())
            })
        }
    };

    // If no action, list shortcuts for this app
    if action.is_none() {
        return json!({
            "success": true,
            "mode": "list_shortcuts",
            "app": app_name,
            "shortcuts": app_shortcuts
        });
    }

    let action_name = action.unwrap().to_lowercase();
    let key_combo = match app_shortcuts.get(&action_name).and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => {
            return json!({
                "success": false,
                "error": format!("Unknown action '{}' for app '{}'. Available: {:?}",
                    action_name, app_name,
                    app_shortcuts.as_object().unwrap().keys().collect::<Vec<_>>())
            })
        }
    };

    // Execute the shortcut via uia_key_press
    let result = uia_key_press(&json!({"keys": key_combo}));

    json!({
        "success": result.get("success").and_then(|v| v.as_bool()).unwrap_or(false),
        "app": app_name,
        "action": action_name,
        "keys": key_combo,
        "result": result
    })
}

#[cfg(windows)]
fn uia_read_value(args: &Value) -> Value {
    let name_filter = args.get("name").and_then(|v| v.as_str());
    let automation_id_filter = args.get("automation_id").and_then(|v| v.as_str());
    let control_type_filter = args.get("control_type").and_then(|v| v.as_str());

    if name_filter.is_none() && automation_id_filter.is_none() {
        return json!({"success": false, "error": "Provide 'name' or 'automation_id' to identify the element"});
    }

    match get_ui_automation() {
        Ok(automation) => {
            unsafe {
                match automation.GetRootElement() {
                    Ok(root) => {
                        // Collect deeper to find nested elements
                        let all_elements_raw =
                            collect_elements_with_values(&root, &automation, 0, 6);

                        let filtered: Vec<Value> = all_elements_raw
                            .into_iter()
                            .filter(|e| {
                                let name_match = name_filter
                                    .map(|n| {
                                        e.get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_lowercase()
                                            .contains(&n.to_lowercase())
                                    })
                                    .unwrap_or(true);
                                let aid_match = automation_id_filter
                                    .map(|a| {
                                        e.get("automation_id")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_lowercase()
                                            .contains(&a.to_lowercase())
                                    })
                                    .unwrap_or(true);
                                let type_match = control_type_filter
                                    .map(|t| {
                                        e.get("control_type")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_lowercase()
                                            == t.to_lowercase()
                                    })
                                    .unwrap_or(true);
                                name_match && aid_match && type_match
                            })
                            .collect();

                        json!({
                            "success": true,
                            "count": filtered.len(),
                            "elements": filtered
                        })
                    }
                    Err(e) => {
                        json!({"success": false, "error": format!("Root element error: {}", e)})
                    }
                }
            }
        }
        Err(e) => json!({"success": false, "error": format!("UIA init error: {}", e)}),
    }
}

/// Collect elements with their text values via UIA Value and Text patterns
#[cfg(windows)]
fn collect_elements_with_values(
    element: &IUIAutomationElement,
    automation: &IUIAutomation,
    depth: u32,
    max_depth: u32,
) -> Vec<Value> {
    let mut results = Vec::new();
    if depth > max_depth {
        return results;
    }

    unsafe {
        let name = element
            .CurrentName()
            .ok()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let control_type_id = element
            .CurrentControlType()
            .ok()
            .unwrap_or(UIA_CONTROLTYPE_ID(0));
        let class_name = element
            .CurrentClassName()
            .ok()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let automation_id = element
            .CurrentAutomationId()
            .ok()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let control_type = control_type_to_string(control_type_id);

        // Try to get value via ValuePattern
        let mut value_text: Option<String> = None;
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        {
            if let Ok(val) = pattern.CurrentValue() {
                let s = val.to_string();
                if !s.is_empty() {
                    value_text = Some(s);
                }
            }
        }

        // Try to get text via TextPattern
        let mut full_text: Option<String> = None;
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        {
            if let Ok(range) = pattern.DocumentRange() {
                if let Ok(text) = range.GetText(4096) {
                    let s = text.to_string();
                    if !s.is_empty() {
                        full_text = Some(s);
                    }
                }
            }
        }

        // Only include elements that have a name, automation_id, or some value
        if !name.is_empty()
            || !automation_id.is_empty()
            || value_text.is_some()
            || full_text.is_some()
        {
            let mut entry = json!({
                "name": name,
                "control_type": control_type,
                "class_name": class_name,
                "automation_id": automation_id,
            });

            if let Some(ref v) = value_text {
                entry
                    .as_object_mut()
                    .unwrap()
                    .insert("value".to_string(), json!(v));
            }
            if let Some(ref t) = full_text {
                entry
                    .as_object_mut()
                    .unwrap()
                    .insert("text".to_string(), json!(t));
            }
            results.push(entry);
        }

        // Recurse children
        if let Ok(condition) = automation.CreateTrueCondition() {
            if let Ok(children) = element.FindAll(TreeScope_Children, &condition) {
                if let Ok(length) = children.Length() {
                    for i in 0..length {
                        if let Ok(child) = children.GetElement(i) {
                            results.extend(collect_elements_with_values(
                                &child,
                                automation,
                                depth + 1,
                                max_depth,
                            ));
                        }
                    }
                }
            }
        }
    }

    results
}

#[cfg(windows)]
fn uia_scroll(args: &Value) -> Value {
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("down");
    let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(3) as i32;

    // Use mouse wheel via SendInput - works on whatever's under the cursor
    // or at the specified coordinates
    let x = args.get("x").and_then(|v| v.as_i64());
    let y = args.get("y").and_then(|v| v.as_i64());

    unsafe {
        // Move cursor to target if coordinates provided
        if let (Some(cx), Some(cy)) = (x, y) {
            let _ = SetCursorPos(cx as i32, cy as i32);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }

        let wheel_delta: i32 = match direction {
            "up" => 120 * amount,
            "down" => -120 * amount,
            "left" => -120 * amount, // horizontal
            "right" => 120 * amount, // horizontal
            _ => -120 * amount,
        };

        let is_horizontal = direction == "left" || direction == "right";

        let inputs = [INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: wheel_delta as u32,
                    dwFlags: if is_horizontal {
                        MOUSEEVENTF_HWHEEL
                    } else {
                        MOUSEEVENTF_WHEEL
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }];

        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }

    json!({
        "success": true,
        "direction": direction,
        "amount": amount,
        "at_cursor": x.is_none()
    })
}

// ============ WATCH / EVENT SYSTEM ============

#[derive(Clone, Serialize)]
struct WatchEvent {
    watch_id: String,
    event_type: String, // "focus_changed", "window_opened", "window_closed", "value_changed"
    timestamp: u64,
    details: Value,
}

#[derive(Clone)]
struct Watch {
    id: String,
    watch_type: String, // "focus", "window_list", "element_value"
    filter: String,     // title substring, element name, etc.
    last_state: String, // serialized last-seen state for diffing
}

struct WatchState {
    watches: Vec<Watch>,
    events: VecDeque<WatchEvent>,
    watcher_running: bool,
}

fn global_watch_state() -> &'static Mutex<WatchState> {
    static STATE: OnceLock<Mutex<WatchState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(WatchState {
            watches: Vec::new(),
            events: VecDeque::new(),
            watcher_running: false,
        })
    })
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(windows)]
fn start_watcher_thread() {
    let state = global_watch_state();
    {
        let mut s = state.lock().unwrap();
        if s.watcher_running {
            return;
        }
        s.watcher_running = true;
    }

    std::thread::spawn(move || {
        // Each iteration: check all watches, detect changes, queue events
        loop {
            std::thread::sleep(std::time::Duration::from_millis(750));

            let watches: Vec<Watch> = {
                let s = global_watch_state().lock().unwrap();
                if s.watches.is_empty() {
                    continue;
                }
                s.watches.clone()
            };

            for watch in &watches {
                match watch.watch_type.as_str() {
                    "focus" => check_focus_watch(watch),
                    "window_list" => check_window_list_watch(watch),
                    "element_value" => check_element_value_watch(watch),
                    _ => {}
                }
            }
        }
    });
}

#[cfg(windows)]
fn check_focus_watch(watch: &Watch) {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut title = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut title);
        let current_title = String::from_utf16_lossy(&title[..len as usize]);

        if current_title != watch.last_state {
            // Focus changed
            let event = WatchEvent {
                watch_id: watch.id.clone(),
                event_type: "focus_changed".to_string(),
                timestamp: now_millis(),
                details: json!({
                    "previous": watch.last_state,
                    "current": current_title,
                }),
            };

            let mut s = global_watch_state().lock().unwrap();
            s.events.push_back(event);
            // Update last_state
            if let Some(w) = s.watches.iter_mut().find(|w| w.id == watch.id) {
                w.last_state = current_title;
            }
        }
    }
}

#[cfg(windows)]
fn check_window_list_watch(watch: &Watch) {
    let windows = get_visible_windows();
    let current_titles: Vec<String> = windows.iter().map(|w| w.title.clone()).collect();
    let current_state = serde_json::to_string(&current_titles).unwrap_or_default();

    if current_state != watch.last_state {
        // Parse old titles for diff
        let old_titles: Vec<String> = serde_json::from_str(&watch.last_state).unwrap_or_default();

        let opened: Vec<&String> = current_titles
            .iter()
            .filter(|t| !old_titles.contains(t))
            .collect();
        let closed: Vec<&String> = old_titles
            .iter()
            .filter(|t| !current_titles.contains(t))
            .collect();

        // Only emit if there's an actual open/close (not just title text change)
        if !opened.is_empty() || !closed.is_empty() {
            let filter_lower = watch.filter.to_lowercase();
            let relevant = filter_lower.is_empty()
                || opened
                    .iter()
                    .any(|t| t.to_lowercase().contains(&filter_lower))
                || closed
                    .iter()
                    .any(|t| t.to_lowercase().contains(&filter_lower));

            if relevant {
                let event = WatchEvent {
                    watch_id: watch.id.clone(),
                    event_type: if !opened.is_empty() {
                        "window_opened"
                    } else {
                        "window_closed"
                    }
                    .to_string(),
                    timestamp: now_millis(),
                    details: json!({
                        "opened": opened,
                        "closed": closed,
                    }),
                };
                let mut s = global_watch_state().lock().unwrap();
                s.events.push_back(event);
            }
        }

        let mut s = global_watch_state().lock().unwrap();
        if let Some(w) = s.watches.iter_mut().find(|w| w.id == watch.id) {
            w.last_state = current_state;
        }
    }
}

#[cfg(windows)]
fn check_element_value_watch(watch: &Watch) {
    // Use UIA to find element and read its value
    if let Ok(automation) = get_ui_automation() {
        unsafe {
            if let Ok(root) = automation.GetRootElement() {
                let elements = collect_elements_with_values(&root, &automation, 0, 4);
                let filter_lower = watch.filter.to_lowercase();

                for elem in &elements {
                    let name = elem.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if !name.to_lowercase().contains(&filter_lower) {
                        continue;
                    }

                    let current_val = elem
                        .get("value")
                        .or_else(|| elem.get("text"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    if current_val != watch.last_state && !current_val.is_empty() {
                        let event = WatchEvent {
                            watch_id: watch.id.clone(),
                            event_type: "value_changed".to_string(),
                            timestamp: now_millis(),
                            details: json!({
                                "element": name,
                                "previous": watch.last_state,
                                "current": current_val,
                            }),
                        };
                        let mut s = global_watch_state().lock().unwrap();
                        s.events.push_back(event);
                        if let Some(w) = s.watches.iter_mut().find(|w| w.id == watch.id) {
                            w.last_state = current_val;
                        }
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn uia_watch(args: &Value) -> Value {
    let watch_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("");
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("add");

    match action {
        "add" => {
            if !["focus", "window_list", "element_value"].contains(&watch_type) {
                return json!({
                    "success": false,
                    "error": "type must be 'focus', 'window_list', or 'element_value'"
                });
            }

            // Get initial state
            let initial_state = match watch_type {
                "focus" => unsafe {
                    let hwnd = GetForegroundWindow();
                    let mut title = [0u16; 256];
                    let len = GetWindowTextW(hwnd, &mut title);
                    String::from_utf16_lossy(&title[..len as usize])
                },
                "window_list" => {
                    let windows = get_visible_windows();
                    let titles: Vec<String> = windows.iter().map(|w| w.title.clone()).collect();
                    serde_json::to_string(&titles).unwrap_or_default()
                }
                "element_value" => String::new(), // Will capture on first poll
                _ => String::new(),
            };

            let id = format!("{}_{}", watch_type, now_millis());

            let mut s = global_watch_state().lock().unwrap();
            s.watches.push(Watch {
                id: id.clone(),
                watch_type: watch_type.to_string(),
                filter: filter.to_string(),
                last_state: initial_state,
            });
            drop(s);

            // Ensure watcher thread is running
            start_watcher_thread();

            json!({
                "success": true,
                "watch_id": id,
                "type": watch_type,
                "filter": filter,
                "hint": "Call uia_poll_events to check for triggered events"
            })
        }
        "remove" => {
            let watch_id = args.get("watch_id").and_then(|v| v.as_str()).unwrap_or("");
            let mut s = global_watch_state().lock().unwrap();
            let before = s.watches.len();
            s.watches.retain(|w| w.id != watch_id);
            let removed = before - s.watches.len();
            json!({
                "success": removed > 0,
                "removed": removed,
                "remaining_watches": s.watches.len()
            })
        }
        "list" => {
            let s = global_watch_state().lock().unwrap();
            let watches: Vec<Value> = s
                .watches
                .iter()
                .map(|w| {
                    json!({
                        "id": w.id,
                        "type": w.watch_type,
                        "filter": w.filter,
                    })
                })
                .collect();
            json!({
                "success": true,
                "count": watches.len(),
                "watches": watches
            })
        }
        "clear" => {
            let mut s = global_watch_state().lock().unwrap();
            let count = s.watches.len();
            s.watches.clear();
            s.events.clear();
            json!({
                "success": true,
                "cleared": count
            })
        }
        _ => {
            json!({"success": false, "error": "action must be 'add', 'remove', 'list', or 'clear'"})
        }
    }
}

#[cfg(windows)]
fn uia_poll_events(args: &Value) -> Value {
    let max_events = args.get("max").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
    let watch_id_filter = args.get("watch_id").and_then(|v| v.as_str());

    let mut s = global_watch_state().lock().unwrap();

    let mut events = Vec::new();
    let mut remaining = VecDeque::new();

    while let Some(event) = s.events.pop_front() {
        let matches = watch_id_filter.map(|f| event.watch_id == f).unwrap_or(true);

        if matches && events.len() < max_events {
            events.push(event);
        } else {
            remaining.push_back(event);
        }
    }

    s.events = remaining;
    let queue_remaining = s.events.len();

    json!({
        "success": true,
        "count": events.len(),
        "events": events.iter().map(|e| json!({
            "watch_id": e.watch_id,
            "event_type": e.event_type,
            "timestamp": e.timestamp,
            "details": e.details,
        })).collect::<Vec<_>>(),
        "queue_remaining": queue_remaining
    })
}

#[cfg(not(windows))]
fn uia_watch(_args: &Value) -> Value {
    json!({"success": false, "error": "Watch only available on Windows"})
}

#[cfg(not(windows))]
fn uia_poll_events(_args: &Value) -> Value {
    json!({"success": false, "error": "Poll events only available on Windows"})
}

#[cfg(not(windows))]
fn uia_read_value(_args: &Value) -> Value {
    json!({"success": false, "error": "Read value only available on Windows"})
}

#[cfg(not(windows))]
fn uia_scroll(_args: &Value) -> Value {
    json!({"success": false, "error": "Scroll only available on Windows"})
}

#[cfg(not(windows))]
fn uia_key_press(_args: &Value) -> Value {
    json!({"success": false, "error": "Key press only available on Windows"})
}

#[cfg(not(windows))]
fn uia_hold_key(_args: &Value) -> Value {
    json!({"success": false, "error": "Hold key only available on Windows"})
}

#[cfg(not(windows))]
fn uia_shortcut(_args: &Value) -> Value {
    json!({"success": false, "error": "Shortcuts only available on Windows"})
}

// ============ TOOL IMPLEMENTATIONS ============

#[cfg(windows)]
fn uia_get_state(args: &Value) -> Value {
    let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let include_invisible = args
        .get("include_invisible")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match get_ui_automation() {
        Ok(automation) => unsafe {
            match automation.GetRootElement() {
                Ok(root) => {
                    let elements =
                        collect_elements(&root, &automation, 0, max_depth, include_invisible);
                    json!({
                        "success": true,
                        "count": elements.len(),
                        "elements": elements
                    })
                }
                Err(e) => json!({"success": false, "error": format!("Root element error: {}", e)}),
            }
        },
        Err(e) => json!({"success": false, "error": format!("UIA init error: {}", e)}),
    }
}

#[cfg(windows)]
fn uia_list_windows(_args: &Value) -> Value {
    let windows = get_visible_windows();
    json!({
        "success": true,
        "count": windows.len(),
        "windows": windows
    })
}

#[cfg(windows)]
fn uia_find_element(args: &Value) -> Value {
    let name_filter = args.get("name").and_then(|v| v.as_str());
    let type_filter = args.get("control_type").and_then(|v| v.as_str());

    match get_ui_automation() {
        Ok(automation) => unsafe {
            match automation.GetRootElement() {
                Ok(root) => {
                    let all_elements = collect_elements(&root, &automation, 0, 5, false);

                    let filtered: Vec<UiElement> = all_elements
                        .into_iter()
                        .filter(|e| {
                            let name_match = name_filter
                                .map(|n| e.name.to_lowercase().contains(&n.to_lowercase()))
                                .unwrap_or(true);
                            let type_match = type_filter
                                .map(|t| e.control_type.to_lowercase() == t.to_lowercase())
                                .unwrap_or(true);
                            name_match && type_match
                        })
                        .collect();

                    json!({
                        "success": true,
                        "count": filtered.len(),
                        "elements": filtered
                    })
                }
                Err(e) => json!({"success": false, "error": format!("Root element error: {}", e)}),
            }
        },
        Err(e) => json!({"success": false, "error": format!("UIA init error: {}", e)}),
    }
}

#[cfg(not(windows))]
fn uia_get_state(_args: &Value) -> Value {
    json!({"success": false, "error": "UI automation only available on Windows"})
}

#[cfg(not(windows))]
fn uia_list_windows(_args: &Value) -> Value {
    json!({"success": false, "error": "Window listing only available on Windows"})
}

#[cfg(not(windows))]
fn uia_find_element(_args: &Value) -> Value {
    json!({"success": false, "error": "Element finding only available on Windows"})
}

// ============ MCP HANDLERS ============

pub fn get_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "uia_get_state",
            "description": "Get UI elements from foreground window with exact [x,y] coordinates for clicking. Use for desktop automation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_depth": {
                        "type": "integer",
                        "description": "Max depth to traverse UI tree (default: 3)",
                        "default": 3
                    },
                    "include_invisible": {
                        "type": "boolean",
                        "description": "Include invisible elements (default: false)",
                        "default": false
                    }
                }
            }
        }),
        json!({
            "name": "uia_list_window",
            "description": "List all visible windows with their positions and handles.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "uia_find",
            "description": "Find UI elements by name or control type. Returns exact coordinates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Filter by name (case-insensitive substring match)"
                    },
                    "control_type": {
                        "type": "string",
                        "description": "Filter by control type (Button, Edit, List, etc.)"
                    }
                }
            }
        }),
        json!({
            "name": "uia_click",
            "description": "Click at exact screen coordinates. Use with uia_find_element to get coordinates first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": { "type": "integer", "description": "Screen X coordinate" },
                    "y": { "type": "integer", "description": "Screen Y coordinate" },
                    "button": { "type": "string", "description": "Mouse button: left, right, middle (default: left)" },
                    "double_click": { "type": "boolean", "description": "Double-click (default: false)" }
                },
                "required": ["x", "y"]
            }
        }),
        json!({
            "name": "uia_type",
            "description": "Type text into the currently focused element using keyboard input simulation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Text to type" }
                },
                "required": ["text"]
            }
        }),
        json!({
            "name": "uia_focus_window",
            "description": "Bring a window to the foreground by title substring match.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Window title to search for (case-insensitive substring)" }
                },
                "required": ["title"]
            }
        }),
        json!({
            "name": "uia_key_press",
            "description": "Press a key or key combination. Supports modifiers (ctrl, shift, alt, win) and named keys (enter, tab, f1-f12, etc). Examples: 'ctrl+s', 'alt+f4', 'ctrl+shift+p', 'enter', 'f5'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keys": { "type": "string", "description": "Key combo like 'ctrl+s', 'alt+tab', 'ctrl+shift+p', 'enter', 'f1'" }
                },
                "required": ["keys"]
            }
        }),
        json!({
            "name": "uia_hold_key",
            "description": "Press a key or key combination and hold it for N milliseconds before releasing. Useful for games, accessibility tools, or any app that distinguishes tap from hold. Same key syntax as uia_key_press. Duration capped at 60000ms (60s).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keys": { "type": "string", "description": "Key or combo to hold (e.g. 'shift', 'ctrl+a', 'space', 'w')" },
                    "duration_ms": { "type": "integer", "description": "Hold duration in milliseconds (default 1000, max 60000)", "default": 1000 }
                },
                "required": ["keys"]
            }
        }),
        json!({
            "name": "uia_shortcut",
            "description": "Execute a named shortcut for a known app. Pass no args to list apps. Pass app only to list its shortcuts. Pass app+action to execute. Supports: vscode, cursor, chrome, explorer, windows, notepad, terminal.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "App name: vscode, cursor, chrome, explorer, windows, notepad, terminal" },
                    "action": { "type": "string", "description": "Action name (e.g. 'command_palette', 'save', 'new_tab')" }
                }
            }
        }),
        json!({
            "name": "uia_read_value",
            "description": "Read the text/value content of UI elements. Uses UIA ValuePattern and TextPattern to extract actual text from Edit fields, text boxes, documents, etc. Filter by name, automation_id, or control_type.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Filter by element name (case-insensitive substring)" },
                    "automation_id": { "type": "string", "description": "Filter by automation ID (case-insensitive substring)" },
                    "control_type": { "type": "string", "description": "Filter by control type (Edit, Document, Text, etc.)" }
                }
            }
        }),
        json!({
            "name": "uia_scroll",
            "description": "Scroll via mouse wheel at current cursor position or at specific coordinates. Supports vertical (up/down) and horizontal (left/right).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "direction": { "type": "string", "description": "Scroll direction: up, down, left, right (default: down)" },
                    "amount": { "type": "integer", "description": "Number of scroll notches (default: 3)" },
                    "x": { "type": "integer", "description": "Optional: move cursor to this X before scrolling" },
                    "y": { "type": "integer", "description": "Optional: move cursor to this Y before scrolling" }
                }
            }
        }),
        json!({
            "name": "uia_watch",
            "description": "Register a watch for desktop events. Background thread polls every 750ms and queues events. Use uia_poll_events to drain. Types: 'focus' (window focus changes), 'window_list' (windows open/close), 'element_value' (UI element text changes). Actions: 'add', 'remove', 'list', 'clear'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "description": "add, remove, list, or clear (default: add)" },
                    "type": { "type": "string", "description": "Watch type: focus, window_list, element_value" },
                    "filter": { "type": "string", "description": "Optional filter: title/name substring to match" },
                    "watch_id": { "type": "string", "description": "For remove: the watch ID to remove" }
                }
            }
        }),
        json!({
            "name": "uia_poll_event",
            "description": "Drain queued events from active watches. Returns events and removes them from the queue. Call periodically or on-demand to check what happened.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max": { "type": "integer", "description": "Max events to return (default: 50)" },
                    "watch_id": { "type": "string", "description": "Optional: only return events from this watch" }
                }
            }
        }),
    ]
}

pub fn handle_tool_call(name: &str, args: &Value) -> Value {
    match name {
        "uia_get_state" => uia_get_state(args),
        "uia_list_window" | "uia_list_windows" => uia_list_windows(args),
        "uia_find" | "uia_find_element" => uia_find_element(args),
        "uia_click" => uia_click(args),
        "uia_type" | "uia_type_text" => uia_type_text(args),
        "uia_focus_window" => uia_focus_window(args),
        "uia_key_press" => uia_key_press(args),
        "uia_hold_key" => uia_hold_key(args),
        "uia_shortcut" => uia_shortcut(args),
        "uia_read_value" => uia_read_value(args),
        "uia_scroll" => uia_scroll(args),
        "uia_watch" => uia_watch(args),
        "uia_poll_event" | "uia_poll_events" => uia_poll_events(args),
        _ => json!({"error": format!("Unknown tool: {}", name)}),
    }
}
