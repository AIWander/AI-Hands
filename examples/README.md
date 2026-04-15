# Examples

`hands` is an MCP server that communicates over JSON-RPC on stdin/stdout. To use it,
send JSON-RPC requests to the running process. Below are common usage patterns.

## 1. Navigate to a page and take a screenshot

```json
{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
  "name": "browser_navigate",
  "arguments": {"url": "https://example.com"}
}}
```

```json
{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
  "name": "browser_screenshot",
  "arguments": {"path": "C:/tmp/example.png"}
}}
```

## 2. Fill and submit a form

```json
{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
  "name": "browser_fill_form",
  "arguments": {
    "fields": [
      {"selector": "#username", "value": "testuser"},
      {"selector": "#password", "value": "hunter2"}
    ]
  }
}}
```

```json
{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
  "name": "browser_click",
  "arguments": {"selector": "button[type='submit']"}
}}
```

## 3. Find and click a Windows app button via UIA

```json
{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
  "name": "uia_find",
  "arguments": {"name": "Save", "control_type": "Button"}
}}
```

```json
{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
  "name": "uia_click",
  "arguments": {"name": "Save", "control_type": "Button"}
}}
```

## 4. OCR text from a screen region

```json
{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
  "name": "vision_screenshot_ocr",
  "arguments": {"region": {"x": 100, "y": 200, "width": 400, "height": 50}}
}}
```

## 5. Batch browser operations

Execute multiple browser actions in a single call to reduce round-trips:

```json
{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
  "name": "browser_batch",
  "arguments": {
    "actions": [
      {"tool": "browser_navigate", "arguments": {"url": "https://example.com/login"}},
      {"tool": "browser_fill_form", "arguments": {"fields": [
        {"selector": "#email", "value": "user@example.com"}
      ]}},
      {"tool": "browser_click", "arguments": {"selector": "#login-btn"}},
      {"tool": "browser_screenshot", "arguments": {"path": "C:/tmp/after_login.png"}}
    ]
  }
}}
```
