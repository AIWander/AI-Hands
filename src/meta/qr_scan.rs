//! QR code scanning for TOTP registration.
//! Captures screen region, decodes QR via rqrr, returns otpauth:// URI.
//!
//! NOTE: The actual workflow:totp_register_from_uri call must be made by the
//! caller (Claude) since hands can't call workflow tools directly. This tool
//! returns the decoded URI for the caller to pass along.

use serde_json::{json, Value};
use std::time::Instant;

use super::response::{MetaToolResult, RungAttempt, Reversibility};
use super::error::MetaError;
use super::instrumentation;
use super::session::SharedSession;

/// Decode a QR code from raw image bytes (PNG/JPEG/BMP).
/// Returns the decoded string content.
///
/// Uses `prepare_from_greyscale` to avoid image crate version conflicts
/// (hands uses image 0.24, rqrr's `img` feature needs 0.25+).
pub fn decode_qr_from_bytes(image_bytes: &[u8]) -> Result<String, String> {
    // Try auto-detect first, then explicit format hints as fallback
    // (Phase C fix1: browser screenshots sometimes lack magic bytes header)
    let img = image::load_from_memory(image_bytes)
        .or_else(|_| image::load_from_memory_with_format(image_bytes, image::ImageFormat::Png))
        .or_else(|_| image::load_from_memory_with_format(image_bytes, image::ImageFormat::Jpeg))
        .or_else(|_| image::load_from_memory_with_format(image_bytes, image::ImageFormat::Bmp))
        .map_err(|e| format!("Failed to load image: {}", e))?;
    let gray = img.to_luma8();
    let (w, h) = gray.dimensions();
    let w = w as usize;
    let h = h as usize;
    let pixels = gray.into_raw();

    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| {
        pixels[y * w + x]
    });

    let grids = prepared.detect_grids();
    if grids.is_empty() {
        return Err("No QR code found in image".into());
    }

    let (_meta, content) = grids[0]
        .decode()
        .map_err(|e| format!("QR decode error: {}", e))?;

    Ok(content)
}

/// Validate that a string is an otpauth:// URI and extract key fields.
fn validate_otpauth_uri(uri: &str) -> Result<Value, String> {
    if !uri.starts_with("otpauth://") {
        return Err(format!("Decoded content is not an otpauth:// URI: {}", uri));
    }

    let rest = &uri[10..];
    let (otp_type, rest) = rest.split_once('/')
        .ok_or("Malformed otpauth URI: missing type")?;

    let label_part = if let Some((label, _query)) = rest.split_once('?') {
        url_decode(label)
    } else {
        return Err("Malformed otpauth URI: missing query parameters".into());
    };

    let (issuer_from_label, account) = if let Some((i, a)) = label_part.split_once(':') {
        (Some(i.to_string()), Some(a.trim().to_string()))
    } else {
        (None, Some(label_part))
    };

    let query = rest.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut issuer = issuer_from_label;
    for param in query.split('&') {
        if let Some((key, val)) = param.split_once('=') {
            if key.to_lowercase() == "issuer" {
                issuer = Some(url_decode(val));
            }
        }
    }

    Ok(json!({
        "type": otp_type,
        "issuer": issuer,
        "account": account,
    }))
}

fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h = chars.next().unwrap_or(0);
            let l = chars.next().unwrap_or(0);
            if let (Some(hv), Some(lv)) = (hex_val(h), hex_val(l)) {
                result.push((hv << 4 | lv) as char);
            } else {
                result.push('%');
                result.push(h as char);
                result.push(l as char);
            }
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decode base64 to raw bytes (for converting screenshot data).
/// Strips data URI prefix (e.g. "data:image/png;base64,") if present.
fn b64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let clean = if let Some(idx) = input.find(";base64,") {
        &input[idx + 8..]
    } else if let Some(idx) = input.find(",") {
        // Handle "data:image/png,..." without explicit ";base64,"
        if input[..idx].starts_with("data:") {
            &input[idx + 1..]
        } else {
            input
        }
    } else {
        input
    };
    base64::engine::general_purpose::STANDARD
        .decode(clean.trim())
        .map_err(|e| format!("Base64 decode failed: {}", e))
}

/// `hands_scan_qr` meta-tool.
///
/// Captures visible screen region (or browser page), decodes QR code,
/// validates otpauth:// URI format, and returns the URI for the caller
/// to pass to `workflow:totp_register_from_uri`.
pub async fn handle(
    args: &Value,
    browser: &browser_mcp::browser::SharedBrowser,
    session: &SharedSession,
) -> Value {
    let start = Instant::now();
    let call_id = {
        let mut s = session.write().unwrap_or_else(|e| e.into_inner());
        s.next_call_id()
    };

    let source = args.get("source").and_then(|v| v.as_str()).unwrap_or("screen");
    let region = args.get("region");

    let ctx = json!({"source": source, "region": region});
    let mut rungs: Vec<RungAttempt> = Vec::new();

    // Rung 1: Capture image (get base64 data, then decode to bytes)
    let rung_start = Instant::now();
    let image_bytes = match source {
        "browser" => {
            // Use browser_mcp tools to capture screenshot
            let shot_args = json!({"full_page": false});
            let result = browser_mcp::tools::handle_tool(browser, "screenshot", shot_args).await;
            let rung_ms = rung_start.elapsed().as_millis() as u64;

            if result.is_error {
                let err_msg = super::extract_browser_text(&result);
                let err_msg = if err_msg.is_empty() { "Browser screenshot failed".to_string() } else { err_msg };
                rungs.push(RungAttempt::failed("browser_screenshot", rung_ms, &err_msg));
                instrumentation::log_rung_attempt("hands_scan_qr", &call_id, "browser_screenshot", false, rung_ms, None, &ctx);
                let elapsed = start.elapsed().as_millis() as u64;
                instrumentation::log_aggregate("hands_scan_qr", &call_id, false, "browser_screenshot", rungs.len(), elapsed, None, Some(&err_msg));
                return MetaToolResult::failure(
                    rungs,
                    MetaError::other(err_msg),
                    elapsed,
                ).with_reversibility(Reversibility::Reversible).to_value();
            }

            // Phase C fix2: Extract image data from ToolResult.
            // browser_screenshot returns ToolContent::Image (base64 data + mimeType),
            // NOT ToolContent::Text. The old code used extract_browser_text() which
            // only handles Text, causing the Image data to be lost → empty buffer →
            // "failed to fill whole buffer" from the image crate.
            let mut image_data: Option<Vec<u8>> = None;

            for content in &result.content {
                match content {
                    // Primary path: Image content with base64 data
                    browser_mcp::types::ToolContent::Image { data, .. } => {
                        eprintln!("[qr_scan] got Image content: {} base64 chars", data.len());
                        match b64_decode(data) {
                            Ok(bytes) => {
                                eprintln!("[qr_scan] decoded to {} bytes, header: {:02x?}",
                                    bytes.len(), &bytes[..bytes.len().min(8)]);
                                image_data = Some(bytes);
                                break;
                            }
                            Err(e) => {
                                eprintln!("[qr_scan] Image content b64 decode failed: {}", e);
                            }
                        }
                    }
                    // Fallback: Text content that might contain JSON with screenshot field
                    browser_mcp::types::ToolContent::Text { text } => {
                        eprintln!("[qr_scan] got Text content: {} chars", text.len());
                        // Try parsing as JSON with screenshot/data field
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
                            let b64_str = val.get("screenshot").and_then(|v| v.as_str())
                                .or_else(|| val.get("data").and_then(|v| v.as_str()))
                                .or_else(|| val.get("result").and_then(|v| v.as_str()));
                            if let Some(b64) = b64_str {
                                if let Ok(bytes) = b64_decode(b64) {
                                    eprintln!("[qr_scan] decoded from Text JSON: {} bytes", bytes.len());
                                    image_data = Some(bytes);
                                    break;
                                }
                            }
                        }
                        // Try as raw base64
                        if let Ok(bytes) = b64_decode(text) {
                            if bytes.len() > 100 {
                                eprintln!("[qr_scan] decoded from raw Text base64: {} bytes", bytes.len());
                                image_data = Some(bytes);
                                break;
                            }
                        }
                    }
                }
            }

            match image_data {
                Some(bytes) if !bytes.is_empty() => {
                    rungs.push(RungAttempt::ok("browser_screenshot", rung_ms));
                    instrumentation::log_rung_attempt("hands_scan_qr", &call_id, "browser_screenshot", true, rung_ms, Some(1.0), &ctx);
                    bytes
                }
                _ => {
                    let err_msg = format!(
                        "Screenshot returned {} content items but no decodable image data",
                        result.content.len()
                    );
                    rungs.push(RungAttempt::failed("browser_screenshot", rung_ms, &err_msg));
                    let elapsed = start.elapsed().as_millis() as u64;
                    instrumentation::log_aggregate("hands_scan_qr", &call_id, false, "browser_screenshot", rungs.len(), elapsed, None, Some(&err_msg));
                    return MetaToolResult::failure(
                        rungs,
                        MetaError::other(err_msg),
                        elapsed,
                    ).with_reversibility(Reversibility::Reversible).to_value();
                }
            }
        }
        _ => {
            // Full-screen or region capture via vision_core
            let capture_result = if let Some(r) = region {
                let x = r.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = r.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let w = r.get("width").and_then(|v| v.as_u64()).unwrap_or(800) as u32;
                let h = r.get("height").and_then(|v| v.as_u64()).unwrap_or(600) as u32;
                vision_core::take_screenshot_region(None, 0, x, y, w, h, 100)
            } else {
                vision_core::take_screenshot(None, 0, 100)
            };

            let rung_ms = rung_start.elapsed().as_millis() as u64;
            match capture_result {
                Ok(data_or_path) => {
                    // Phase C fix1: vision_core may return a file path (e.g. "C:\temp\screenshot.png")
                    // OR base64-encoded image data. Detect which and handle accordingly.
                    // The colon at offset 1 ("C:") was causing "Base64 decode failed: Invalid symbol 58".
                    let bytes = if looks_like_file_path(&data_or_path) {
                        match std::fs::read(&data_or_path) {
                            Ok(b) => b,
                            Err(e) => {
                                let msg = format!("Failed to read screenshot file '{}': {}", data_or_path, e);
                                rungs.push(RungAttempt::failed("vision_screenshot", rung_ms, &msg));
                                let elapsed = start.elapsed().as_millis() as u64;
                                instrumentation::log_aggregate("hands_scan_qr", &call_id, false, "vision_screenshot", rungs.len(), elapsed, None, Some(&msg));
                                return MetaToolResult::failure(
                                    rungs, MetaError::other(msg), elapsed,
                                ).with_reversibility(Reversibility::Reversible).to_value();
                            }
                        }
                    } else {
                        match b64_decode(&data_or_path) {
                            Ok(b) => b,
                            Err(e) => {
                                rungs.push(RungAttempt::failed("vision_screenshot", rung_ms, &e));
                                let elapsed = start.elapsed().as_millis() as u64;
                                instrumentation::log_aggregate("hands_scan_qr", &call_id, false, "vision_screenshot", rungs.len(), elapsed, None, Some(&e));
                                return MetaToolResult::failure(
                                    rungs, MetaError::other(e), elapsed,
                                ).with_reversibility(Reversibility::Reversible).to_value();
                            }
                        }
                    };

                    rungs.push(RungAttempt::ok("vision_screenshot", rung_ms));
                    instrumentation::log_rung_attempt("hands_scan_qr", &call_id, "vision_screenshot", true, rung_ms, Some(1.0), &ctx);
                    bytes
                }
                Err(e) => {
                    rungs.push(RungAttempt::failed("vision_screenshot", rung_ms, &e));
                    let elapsed = start.elapsed().as_millis() as u64;
                    instrumentation::log_aggregate("hands_scan_qr", &call_id, false, "vision_screenshot", rungs.len(), elapsed, None, Some(&e));
                    return MetaToolResult::failure(
                        rungs,
                        MetaError::other(e),
                        elapsed,
                    ).with_reversibility(Reversibility::Reversible).to_value();
                }
            }
        }
    };

    // Rung 2: Decode QR
    let rung_start = Instant::now();
    let decoded = match decode_qr_from_bytes(&image_bytes) {
        Ok(content) => {
            let rung_ms = rung_start.elapsed().as_millis() as u64;
            rungs.push(RungAttempt::ok("qr_decode", rung_ms));
            instrumentation::log_rung_attempt("hands_scan_qr", &call_id, "qr_decode", true, rung_ms, Some(1.0), &ctx);
            content
        }
        Err(e) => {
            let rung_ms = rung_start.elapsed().as_millis() as u64;
            rungs.push(RungAttempt::failed("qr_decode", rung_ms, &e));
            let elapsed = start.elapsed().as_millis() as u64;
            instrumentation::log_aggregate("hands_scan_qr", &call_id, false, "qr_decode", rungs.len(), elapsed, None, Some(&e));
            return MetaToolResult::failure(
                rungs,
                MetaError::other(e),
                elapsed,
            ).with_reversibility(Reversibility::Reversible).to_value();
        }
    };

    // Rung 3: Validate otpauth:// URI
    let rung_start = Instant::now();
    match validate_otpauth_uri(&decoded) {
        Ok(parsed) => {
            let rung_ms = rung_start.elapsed().as_millis() as u64;
            rungs.push(RungAttempt::ok("validate_otpauth", rung_ms));
            let elapsed = start.elapsed().as_millis() as u64;
            instrumentation::log_aggregate("hands_scan_qr", &call_id, true, "qr_decode+validate", rungs.len(), elapsed, Some(1.0), None);

            MetaToolResult::success(
                "qr_decode+validate",
                rungs,
                json!({
                    "uri": decoded,
                    "is_otpauth": true,
                    "type": parsed.get("type"),
                    "issuer": parsed.get("issuer"),
                    "account": parsed.get("account"),
                    "next_step": "Call workflow:totp_register_from_uri with this URI to register the 2FA entry.",
                }),
                elapsed,
            ).with_reversibility(Reversibility::Reversible).to_value()
        }
        Err(e) => {
            let rung_ms = rung_start.elapsed().as_millis() as u64;
            rungs.push(RungAttempt::failed("validate_otpauth", rung_ms, &e));
            let elapsed = start.elapsed().as_millis() as u64;
            // Still a success (decoded QR content), just not otpauth
            instrumentation::log_aggregate("hands_scan_qr", &call_id, true, "qr_decode", rungs.len(), elapsed, Some(0.8), None);

            MetaToolResult::success(
                "qr_decode",
                rungs,
                json!({
                    "decoded_content": decoded,
                    "is_otpauth": false,
                    "warning": e,
                }),
                elapsed,
            ).with_reversibility(Reversibility::Reversible)
             .with_warning(format!("QR decoded but not an otpauth URI: {}", e))
             .to_value()
        }
    }
}

/// Detect if a string looks like a file path rather than base64 data.
/// Checks for Windows drive letter (C:\), UNC paths (\\), and Unix absolute paths (/).
fn looks_like_file_path(s: &str) -> bool {
    let s = s.trim();
    // Windows drive letter: "C:\..." or "C:/..."
    if s.len() >= 3 {
        let bytes = s.as_bytes();
        if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
            return true;
        }
    }
    // UNC path
    if s.starts_with("\\\\") {
        return true;
    }
    // File extension hint (common screenshot formats)
    if s.ends_with(".png") || s.ends_with(".jpg") || s.ends_with(".jpeg") || s.ends_with(".bmp") {
        return true;
    }
    false
}
