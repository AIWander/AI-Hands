//! Rust UI Automation MCP Server (stdio)
//! Thin MCP protocol wrapper that delegates to uia_lib.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

// ============ MCP PROTOCOL ============

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

// ============ MCP HANDLERS ============

fn handle_request(request: &JsonRpcRequest) -> JsonRpcResponse {
    let id = request.id.clone().unwrap_or(Value::Null);

    match request.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "uia-mcp",
                    "version": "1.0.0"
                }
            })),
            error: None,
        },

        "notifications/initialized" => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(json!({})),
            error: None,
        },

        "tools/list" => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(json!({
                "tools": uia_lib::get_tool_definitions()
            })),
            error: None,
        },

        "tools/call" => {
            let params = request.params.as_ref();
            let tool_name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let tool_args = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or(json!({}));

            let result = uia_lib::handle_tool_call(tool_name, &tool_args);

            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                    }]
                })),
                error: None,
            }
        }

        _ => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(json!({
                "code": -32601,
                "message": format!("Method not found: {}", request.method)
            })),
        },
    }
}

// ============ MAIN ============

fn main() {
    // Debug: log startup to temp file so we can verify Claude Code launches this
    let _ = std::fs::write(
        std::env::temp_dir().join("uia_mcp_started.txt"),
        format!(
            "UIA MCP started at {:?}\nPID: {}\nArgs: {:?}\n",
            std::time::SystemTime::now(),
            std::process::id(),
            std::env::args().collect::<Vec<_>>()
        ),
    );

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Parse error: {}", e);
                continue;
            }
        };

        // Validate JSON-RPC 2.0 version
        if request.jsonrpc != "2.0" {
            eprintln!("Invalid JSON-RPC version: {}", request.jsonrpc);
            let response = JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone().unwrap_or(Value::Null),
                result: None,
                error: Some(json!({
                    "code": -32600,
                    "message": format!("Invalid JSON-RPC version: expected '2.0', got '{}'", request.jsonrpc)
                })),
            };
            writeln!(stdout, "{}", serde_json::to_string(&response).unwrap()).unwrap();
            stdout.flush().unwrap();
            continue;
        }

        // Handle notifications (no id) - just acknowledge
        if request.id.is_none() {
            continue;
        }

        let response = handle_request(&request);
        let response_str = serde_json::to_string(&response).unwrap();
        writeln!(stdout, "{}", response_str).unwrap();
        stdout.flush().unwrap();
    }
}
