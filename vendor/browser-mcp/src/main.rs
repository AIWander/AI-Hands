//! Browser MCP Server - Pure Rust Playwright replacement
use browser_mcp::browser::create_shared;
use browser_mcp::tools::{handle_tool, list_tools};
use browser_mcp::types::*;
use serde_json::json;
use std::io::{BufRead, Write};

#[tokio::main]
async fn main() {
    let browser = create_shared();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            _ => continue,
        };

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                writeln!(
                    stdout,
                    "{}",
                    error_response(None, -32700, format!("Parse error: {}", e))
                )
                .ok();
                stdout.flush().ok();
                continue;
            }
        };

        // Validate JSON-RPC 2.0 version
        if req.jsonrpc != "2.0" {
            eprintln!("Invalid JSON-RPC version: {}", req.jsonrpc);
            writeln!(
                stdout,
                "{}",
                error_response(
                    req.id,
                    -32600,
                    format!(
                        "Invalid JSON-RPC version: expected '2.0', got '{}'",
                        req.jsonrpc
                    )
                )
            )
            .ok();
            stdout.flush().ok();
            continue;
        }

        let response = match req.method.as_str() {
            "initialize" => success_response(
                req.id,
                json!(InitializeResult {
                    protocol_version: "2024-11-05".into(),
                    capabilities: Capabilities {
                        tools: ToolsCapability {
                            list_changed: false
                        },
                    },
                    server_info: ServerInfo {
                        name: "browser-mcp".into(),
                        version: "0.1.0".into(),
                    },
                }),
            ),

            "notifications/initialized" | "initialized" => continue, // Notification, no response

            "tools/list" => success_response(req.id, json!({"tools": list_tools()})),

            "tools/call" => {
                let name = req
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
                let result = handle_tool(&browser, name, args).await;
                success_response(req.id, json!(result))
            }

            "ping" => success_response(req.id, json!({})),

            _ => error_response(req.id, -32601, format!("Unknown method: {}", req.method)),
        };

        writeln!(stdout, "{}", response).ok();
        stdout.flush().ok();
    }
}
