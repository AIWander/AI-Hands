# Example: Automate a Windows Desktop App

UIA tools control native Windows apps through the accessibility tree.
No browser involved — this talks directly to the OS.

## Open Notepad, type, and save

```
uia_app_launch(name: "notepad.exe")
uia_list_window()
→ Shows all open windows. Find the exact title string.

uia_focus_window(title: "Untitled - Notepad")
uia_type(text: "Meeting notes for Monday\nAction items:\n- Review PR\n- Deploy fix")
uia_shortcut(keys: "Ctrl+S")

wait_for_visual(text: "Save As")
uia_type(text: "meeting_notes.txt")
uia_key_press(key: "Enter")
```

## Multi-window layout

```
uia_app_launch(name: "excel.exe")
uia_app_launch(name: "notepad.exe")

uia_window_snap(title: "Excel", position: "left")
uia_window_snap(title: "Notepad", position: "right")

uia_focus_window(title: "Excel")
uia_read_value(name: "A1")
→ Get the cell value

uia_focus_window(title: "Notepad")
uia_type(text: "Value from Excel: ...")
```

## Batch UIA actions

```json
uia_batch(actions: [
  {"type": "click", "name": "File", "control_type": "MenuItem"},
  {"type": "click", "name": "Save As", "control_type": "MenuItem"},
  {"type": "type_text", "text": "report.pdf"},
  {"type": "key_press", "key": "Enter"}
])
```

One round-trip instead of four.
