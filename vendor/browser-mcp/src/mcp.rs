//! MCP Protocol Implementation for Browser-MCP
// NAV: TOC at line 375 | 4 fn | 3 struct | 2026-03-26

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use tracing::{info, warn, error};

use browser_mcp;

// ============ MCP PROTOCOL TYPES ============

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

// ============ TOOL DEFINITIONS ============

fn get_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "navigate",
            "description": "Navigate browser to URL with auto-wait",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to navigate to" },
                    "wait_until": { "type": "string", "description": "Wait condition: load, networkidle", "default": "load" }
                },
                "required": ["url"]
            }
        }),
        json!({
            "name": "click",
            "description": "Click element by selector with retry",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector" },
                    "timeout_ms": { "type": "integer", "description": "Timeout in ms", "default": 10000 }
                },
                "required": ["selector"]
            }
        }),
        json!({
            "name": "fill",
            "description": "Type text into input field",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector" },
                    "value": { "type": "string", "description": "Text to type" },
                    "clear": { "type": "boolean", "description": "Clear field first", "default": true }
                },
                "required": ["selector", "value"]
            }
        }),
        json!({
            "name": "screenshot",
            "description": "Take screenshot, returns base64 or saves to path",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Save path (optional)" },
                    "full_page": { "type": "boolean", "description": "Full page capture", "default": false },
                    "quality": { "type": "integer", "description": "JPEG quality 1-100", "default": 80 }
                }
            }
        }),
        json!({
            "name": "get_html",
            "description": "Get page HTML content",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector (optional, full page if omitted)" }
                }
            }
        }),
        json!({
            "name": "get_text",
            "description": "Get text content of element",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector" }
                },
                "required": ["selector"]
            }
        }),
        json!({
            "name": "eval",
            "description": "Execute JavaScript in page context",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "script": { "type": "string", "description": "JavaScript code" }
                },
                "required": ["script"]
            }
        }),
        json!({
            "name": "wait_for",
            "description": "Wait for element to appear",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector" },
                    "state": { "type": "string", "description": "visible, hidden, attached", "default": "visible" },
                    "timeout_ms": { "type": "integer", "description": "Timeout in ms", "default": 10000 }
                },
                "required": ["selector"]
            }
        }),
        json!({
            "name": "press",
            "description": "Press keyboard key",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Key name: Enter, Tab, Escape, etc" }
                },
                "required": ["key"]
            }
        }),
        json!({
            "name": "select",
            "description": "Select dropdown option",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector" },
                    "value": { "type": "string", "description": "Option value" }
                },
                "required": ["selector", "value"]
            }
        }),
        json!({
            "name": "scroll",
            "description": "Scroll page or element",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "direction": { "type": "string", "description": "up, down, left, right", "default": "down" },
                    "amount": { "type": "integer", "description": "Pixels to scroll", "default": 500 },
                    "selector": { "type": "string", "description": "Element to scroll (optional)" }
                }
            }
        }),
        json!({
            "name": "cookies",
            "description": "Manage cookies: get, set, clear",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "description": "get, set, clear", "default": "get" },
                    "name": { "type": "string", "description": "Cookie name (for set)" },
                    "value": { "type": "string", "description": "Cookie value (for set)" },
                    "domain": { "type": "string", "description": "Cookie domain (for set)" }
                }
            }
        }),
        json!({
            "name": "status",
            "description": "Get browser status and current URL",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "configure",
            "description": "Configure browser settings",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "auto_wait": { "type": "boolean", "description": "Auto-wait for elements" },
                    "timeout_ms": { "type": "integer", "description": "Default timeout" },
                    "headless": { "type": "boolean", "description": "Headless mode" }
                }
            }
        }),
        json!({
            "name": "close",
            "description": "Close browser and cleanup",
            "inputSchema": { "type": "object", "properties": {} }
        }),
    ]
}

// ============ TOOL EXECUTION ============

async fn execute_tool(name: &str, args: Value) -> Result<Value> {
    match name {
        "navigate" => browser_mcp::navigate(args).await,
        "click" => browser_mcp::click(args).await,
        "fill" => browser_mcp::fill(args).await,
        "screenshot" => browser_mcp::screenshot(args).await,
        "get_html" => browser_mcp::get_html(args).await,
        "get_text" => browser_mcp::get_text(args).await,
        "eval" | "evaluate" => browser_mcp::evaluate(args).await,
        "wait_for" => browser_mcp::wait_for(args).await,
        "press" => browser_mcp::press(args).await,
        "select" => browser_mcp::select(args).await,
        "scroll" => browser_mcp::scroll(args).await,
        "cookies" => browser_mcp::cookies(args).await,
        "status" => browser_mcp::status(args).await,
        "configure" => browser_mcp::configure(args).await,
        "close" => browser_mcp::close().await,
        _ => anyhow::bail!("Unknown tool: {}", name),
    }
}

// ============ MCP SERVER ============

pub async fn run_stdio_server() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    info!("Browser-MCP ready, listening on stdio");

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                error!("Read error: {}", e);
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                        data: None,
                    }),
                };
                writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
                stdout.flush()?;
                continue;
            }
        };

        let method = match &request.method {
            Some(m) => m.clone(),
            None => {
                warn!("Request missing method");
                continue;
            }
        };

        // Skip notifications
        if request.id.is_none() || method.starts_with("notifications/") {
            info!("Notification: {} (no response)", method);
            continue;
        }

        let response = handle_request(&method, request.id, request.params).await;
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }

    Ok(())
}

async fn handle_request(method: &str, id: Option<Value>, params: Option<Value>) -> JsonRpcResponse {
    let id = id.unwrap_or(Value::Null);

    match method {
        "initialize" => {
            info!("Initialize request");
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "browser-mcp",
                        "version": "2.0.0"
                    }
                })),
                error: None,
            }
        }

        "tools/list" => {
            info!("Tools list requested");
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(json!({ "tools": get_tool_definitions() })),
                error: None,
            }
        }

        "tools/call" => {
            let params = params.unwrap_or(json!({}));
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let tool_args = params.get("arguments").cloned().unwrap_or(json!({}));

            info!("Tool call: {}", tool_name);

            match execute_tool(tool_name, tool_args).await {
                Ok(result) => JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
                        }]
                    })),
                    error: None,
                },
                Err(e) => JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32000,
                        message: e.to_string(),
                        data: None,
                    }),
                },
            }
        }

        _ => {
            warn!("Unknown method: {}", method);
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", method),
                    data: None,
                }),
            }
        }
    }
}

// === FILE NAVIGATION ===
// Generated: 2026-03-26T16:19:22
// Total: 372 lines | 4 functions | 3 structs | 0 constants
//
// IMPORTS: anyhow, browser_mcp, serde, serde_json, std, tracing
//
// STRUCTS:
//   JsonRpcRequest: 14-19
//   JsonRpcResponse: 22-29
//   JsonRpcError: 32-37
//
// FUNCTIONS:
//   get_tool_definitions: 41-208 [LARGE]
//   execute_tool: 212-231
//   pub +run_stdio_server: 235-293 [med]
//   handle_request: 295-372 [med]
//
// === END FILE NAVIGATION ===
