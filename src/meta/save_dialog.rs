//! Save dialog detection + auto-handling for window close operations.
//! Detects unsaved-changes dialogs and handles via Auto/Save/Discard/Ask behavior.
//!
//! Known dialog patterns:
//!   - Notepad: "Do you want to save changes to {file}?" → [Save, Don't Save, Cancel]
//!   - VS Code: "Do you want to save the changes...?" → [Save, Don't Save, Cancel]
//!   - Word/Excel: "Want to save your changes to {file}?" → [Save, Don't Save, Cancel]
//!   - Generic: Dialog class #32770 with Save/Don't Save/Yes/No buttons

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How to handle a detected save dialog on close.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SaveDialogAction {
    /// Save in place if file has a path; autosave to Documents if Untitled.
    #[default]
    Auto,
    /// Always click "Save".
    Save,
    /// Click "Don't Save" / "No" / "Discard" to discard changes.
    Discard,
    /// Return dialog info to caller, take no action.
    Ask,
}

/// Information about a detected save dialog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveDialogInfo {
    pub detected: bool,
    pub dialog_text: Option<String>,
    pub buttons: Vec<String>,
    pub suggested_action: String,
}

/// Resolution for how to handle a detected dialog.
pub struct DialogResolution {
    /// Which button to click (None for Ask mode — no action taken).
    pub button_text: Option<String>,
    /// For Auto mode with Untitled files — path to save to.
    pub save_path: Option<String>,
    /// Human-readable description of the action taken.
    pub description: String,
}

/// Parse a SaveDialogAction from an optional string value.
pub fn parse_save_dialog_action(s: Option<&str>) -> SaveDialogAction {
    match s {
        Some("auto") => SaveDialogAction::Auto,
        Some("save") => SaveDialogAction::Save,
        Some("discard") => SaveDialogAction::Discard,
        Some("ask") => SaveDialogAction::Ask,
        _ => SaveDialogAction::default(),
    }
}

/// Known button labels for "save" actions.
const SAVE_BUTTONS: &[&str] = &["Save", "Yes", "&Save", "&Yes"];

/// Known button labels for "discard" actions.
const DISCARD_BUTTONS: &[&str] = &[
    "Don't Save",
    "Don't Save",
    "No",
    "Discard",
    "&Don't Save",
    "&No",
    "Do&n't Save",
];

/// Known button labels for "cancel" actions.
const CANCEL_BUTTONS: &[&str] = &["Cancel", "&Cancel"];

/// Detect if a save dialog appeared after a close attempt.
///
/// Checks UIA window list entries for dialog indicators:
/// - Window class `#32770` (standard Win32 dialog)
/// - `role` = "dialog" or "alert"
/// - Title containing "save", "unsaved", or "changes"
pub fn detect_save_dialog(uia_windows: &[Value]) -> Option<SaveDialogInfo> {
    for window in uia_windows {
        let class = window
            .get("class")
            .or_else(|| window.get("class_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let role = window.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let title = window
            .get("title")
            .or_else(|| window.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let title_lower = title.to_lowercase();

        let is_dialog = class == "#32770"
            || role == "dialog"
            || role == "alert"
            || title_lower.contains("save")
            || title_lower.contains("unsaved")
            || title_lower.contains("changes");

        if !is_dialog {
            continue;
        }

        // Extract dialog text from content or children
        let dialog_text = extract_dialog_text(window);

        // Extract button labels from children
        let buttons = extract_button_labels(window);

        // Only treat as a save dialog if it has recognizable save/discard buttons
        let has_save_pattern = buttons.iter().any(|b| {
            let bl = b.to_lowercase();
            bl.contains("save")
                || bl == "yes"
                || bl.contains("don't save")
                || bl == "no"
                || bl == "discard"
        });

        if !has_save_pattern && dialog_text.is_none() {
            continue;
        }

        let suggested = if has_discard_button(&buttons) {
            "discard"
        } else if has_save_button(&buttons) {
            "save"
        } else {
            "ask"
        };

        return Some(SaveDialogInfo {
            detected: true,
            dialog_text,
            buttons,
            suggested_action: suggested.to_string(),
        });
    }

    None
}

/// Extract readable text from a dialog window's content or children.
fn extract_dialog_text(window: &Value) -> Option<String> {
    // Try direct text/content fields
    if let Some(text) = window.get("text").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    if let Some(content) = window.get("content").and_then(|v| v.as_str()) {
        if !content.is_empty() {
            return Some(content.to_string());
        }
    }

    // Walk children for static text elements
    if let Some(children) = window.get("children").and_then(|v| v.as_array()) {
        let mut texts = Vec::new();
        for child in children {
            let child_role = child.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let child_class = child.get("class").and_then(|v| v.as_str()).unwrap_or("");

            if child_role == "text" || child_role == "static_text" || child_class == "Static" {
                if let Some(t) = child
                    .get("name")
                    .or_else(|| child.get("text"))
                    .and_then(|v| v.as_str())
                {
                    if !t.is_empty() {
                        texts.push(t.to_string());
                    }
                }
            }
        }
        if !texts.is_empty() {
            return Some(texts.join(" "));
        }
    }

    None
}

/// Extract button labels from a dialog window's children.
fn extract_button_labels(window: &Value) -> Vec<String> {
    let mut buttons = Vec::new();

    if let Some(children) = window.get("children").and_then(|v| v.as_array()) {
        for child in children {
            let child_role = child.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let child_class = child.get("class").and_then(|v| v.as_str()).unwrap_or("");

            if child_role == "button" || child_class == "Button" {
                if let Some(label) = child
                    .get("name")
                    .or_else(|| child.get("text"))
                    .and_then(|v| v.as_str())
                {
                    if !label.is_empty() {
                        buttons.push(label.to_string());
                    }
                }
            }
        }
    }

    // Fallback: check top-level buttons array
    if buttons.is_empty() {
        if let Some(btn_array) = window.get("buttons").and_then(|v| v.as_array()) {
            for b in btn_array {
                if let Some(label) = b.as_str() {
                    buttons.push(label.to_string());
                } else if let Some(label) = b
                    .get("name")
                    .or_else(|| b.get("text"))
                    .and_then(|v| v.as_str())
                {
                    buttons.push(label.to_string());
                }
            }
        }
    }

    buttons
}

fn has_save_button(buttons: &[String]) -> bool {
    buttons
        .iter()
        .any(|b| SAVE_BUTTONS.iter().any(|sb| b.eq_ignore_ascii_case(sb)))
}

fn has_discard_button(buttons: &[String]) -> bool {
    buttons
        .iter()
        .any(|b| DISCARD_BUTTONS.iter().any(|db| b.eq_ignore_ascii_case(db)))
}

/// Find the best matching button label from the dialog's available buttons.
fn find_button<'a>(buttons: &'a [String], candidates: &[&str]) -> Option<&'a str> {
    for candidate in candidates {
        for button in buttons {
            if button.eq_ignore_ascii_case(candidate) {
                return Some(button.as_str());
            }
        }
    }
    // Fuzzy fallback: contains match
    for candidate in candidates {
        let cl = candidate.to_lowercase();
        for button in buttons {
            if button.to_lowercase().contains(&cl) {
                return Some(button.as_str());
            }
        }
    }
    None
}

/// Determine if the dialog text indicates an Untitled/unsaved-new file.
fn is_untitled(dialog_text: Option<&str>, app_name: &str) -> bool {
    if let Some(text) = dialog_text {
        let tl = text.to_lowercase();
        return tl.contains("untitled")
            || tl.contains("new document")
            || tl.contains("new file")
            || tl.contains("document1")
            || tl.contains("book1");
    }
    // Some apps don't name untitled files in the dialog
    let _ = app_name;
    false
}

/// Generate an autosave path for Untitled documents.
fn autosave_path(app_name: &str, ext: &str) -> String {
    let user_profile =
        std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\Default".to_string());
    let dir = format!("{}\\Documents\\hands-autosave", user_profile);
    let _ = std::fs::create_dir_all(&dir);

    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let safe_app = app_name.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");
    format!("{}\\{}_{}.{}", dir, ts, safe_app, ext)
}

/// Guess a file extension from the app name.
fn guess_extension(app_name: &str) -> &'static str {
    let al = app_name.to_lowercase();
    if al.contains("notepad") || al.contains("text") {
        "txt"
    } else if al.contains("word") || al.contains("winword") {
        "docx"
    } else if al.contains("excel") {
        "xlsx"
    } else if al.contains("powerpoint") {
        "pptx"
    } else {
        // covers vscode, code, and unknown apps
        "txt"
    }
}

/// Build the resolution for how to handle the detected save dialog.
///
/// Returns the button to click and a description of the action.
///
/// # Modes
/// - `Auto`: Save if file has a path; autosave to `%USERPROFILE%\Documents\hands-autosave\` if Untitled;
///   fall through to Ask for unrecognized patterns.
/// - `Save`: Click "Save" button.
/// - `Discard`: Click "Don't Save" / "No" / "Discard".
/// - `Ask`: Return dialog info with no action.
pub fn resolve_dialog_action(
    dialog: &SaveDialogInfo,
    action: &SaveDialogAction,
    app_name: &str,
) -> Result<DialogResolution, String> {
    match action {
        SaveDialogAction::Ask => Ok(DialogResolution {
            button_text: None,
            save_path: None,
            description: format!(
                "Save dialog detected (buttons: [{}]). Returning to caller for decision.",
                dialog.buttons.join(", ")
            ),
        }),

        SaveDialogAction::Save => {
            let btn = find_button(&dialog.buttons, SAVE_BUTTONS).ok_or_else(|| {
                format!(
                    "No Save button found among: [{}]",
                    dialog.buttons.join(", ")
                )
            })?;
            Ok(DialogResolution {
                button_text: Some(btn.to_string()),
                save_path: None,
                description: format!("Clicking '{}' to save.", btn),
            })
        }

        SaveDialogAction::Discard => {
            let btn = find_button(&dialog.buttons, DISCARD_BUTTONS).ok_or_else(|| {
                format!(
                    "No Discard/Don't Save button found among: [{}]",
                    dialog.buttons.join(", ")
                )
            })?;
            Ok(DialogResolution {
                button_text: Some(btn.to_string()),
                save_path: None,
                description: format!("Clicking '{}' to discard unsaved changes.", btn),
            })
        }

        SaveDialogAction::Auto => {
            if is_untitled(dialog.dialog_text.as_deref(), app_name) {
                // Untitled document — autosave to Documents
                let ext = guess_extension(app_name);
                let path = autosave_path(app_name, ext);

                // For Untitled: click Save, then we'd need to handle the Save As dialog.
                // For simplicity, click Save and note the autosave intent.
                let btn = find_button(&dialog.buttons, SAVE_BUTTONS);
                match btn {
                    Some(b) => Ok(DialogResolution {
                        button_text: Some(b.to_string()),
                        save_path: Some(path.clone()),
                        description: format!(
                            "Untitled document — clicking '{}'. Autosave target: {}",
                            b, path
                        ),
                    }),
                    None => {
                        // Can't save — fall through to Ask
                        Ok(DialogResolution {
                            button_text: None,
                            save_path: None,
                            description: format!(
                                "Untitled document but no Save button found. Returning to caller. Buttons: [{}]",
                                dialog.buttons.join(", ")
                            ),
                        })
                    }
                }
            } else {
                // Named file — save in place
                let btn = find_button(&dialog.buttons, SAVE_BUTTONS);
                match btn {
                    Some(b) => Ok(DialogResolution {
                        button_text: Some(b.to_string()),
                        save_path: None,
                        description: format!("Named file — clicking '{}' to save in place.", b),
                    }),
                    None => {
                        // Weird case — no recognizable Save button
                        Ok(DialogResolution {
                            button_text: None,
                            save_path: None,
                            description: format!(
                                "Cannot determine save action. Returning to caller. Buttons: [{}]",
                                dialog.buttons.join(", ")
                            ),
                        })
                    }
                }
            }
        }
    }
}
