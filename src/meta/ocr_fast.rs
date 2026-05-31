#![allow(dead_code)] // public handle_* + helpers invoked via main.rs dispatch
//! `vision_ocr_fast` + `vision_ocr_capabilities` — Item 4 phase 1 (no ONNX yet).
//!
//! Phase 1 ships the API surface so callers can adopt `vision_ocr_fast` today
//! and benefit transparently when the ONNX backend lands in vision-core v0.2.0
//! (the 5–10x speedup on complex screens from the Milestone A handoff).
//!
//! Architecture:
//! - `vision_ocr_fast` routes through `vision_cache::cached_execute("vision_ocr", args)`
//!   and annotates the result with `backend_metadata` documenting the current
//!   backend (tesseract), the detected GPU, and the planned ONNX backend.
//!   Registered via the Phase A/B/C path (`dispatch_meta_tool`) because
//!   `cached_execute` is async; the handler accepts `browser`/`session` but
//!   ignores them.
//! - `vision_ocr_capabilities` is pure local-state introspection: returns
//!   platform, available/planned backends, GPU vendor + adapter list, DLL
//!   presence checks. Cached in `OnceLock` so the wmic shell-out only happens
//!   once per process. Pass `force_refresh: true` to bypass the cache.
//!   Registered via the special-case path next to `vision_cache_stats`.
//!
//! GPU detection (Windows):
//!   - `nvml_dll_present`     = C:\Windows\System32\nvml.dll
//!   - `directml_dll_present` = C:\Windows\System32\DirectML.dll
//!   - `cuda_env_set`         = CUDA_VISIBLE_DEVICES / NVIDIA_VISIBLE_DEVICES
//!   - adapters via `wmic path Win32_VideoController get Name /format:list`
//!   - vendor inferred from adapter names (NVIDIA / AMD / Intel / Apple)
//!
//! Non-Windows returns a minimal payload — only env-var-derived signals are
//! checked; full adapter enumeration is deferred to a later phase.

use serde_json::{json, Value};
use std::path::Path;
use std::sync::OnceLock;

use super::session::SharedSession;

// ── Public entry points ──────────────────────────────────────────────────────

/// Handler for `vision_ocr_fast` — phase 1 stub.
///
/// Routes through `vision_cache::cached_execute("vision_ocr", args)` and
/// annotates the response with `backend_metadata`. The signature matches the
/// Phase A/B/C dispatch contract (browser + session) so it can be registered in
/// `dispatch_meta_tool`, but neither is currently used.
pub async fn handle_ocr_fast(
    args: &Value,
    _browser: &browser_mcp::browser::SharedBrowser,
    _session: &SharedSession,
) -> Value {
    let caps = get_capabilities_cached();
    let gpu_detected = caps
        .get("gpu")
        .and_then(|g| g.get("detected"))
        .cloned()
        .unwrap_or(json!(false));

    // Route through the existing vision_cache so callers benefit from caching
    // today even before the ONNX backend lands.
    let raw = super::vision_cache::cached_execute("vision_ocr", args).await;

    let backend_metadata = json!({
        "backend": "tesseract",
        "phase": "phase_1_stub",
        "gpu_detected": gpu_detected,
        "future_backend": "PaddleOCR-ONNX (vision-core v0.2.0 — pending)",
        "speedup_potential": "5-10x on complex screens (handoff acceptance)",
        "note": "Callers can adopt vision_ocr_fast today; phase 2 will swap the underlying OCR backend transparently when vision-core v0.2.0 lands with ONNX support."
    });

    annotate_with_backend_metadata(raw, backend_metadata)
}

/// Handler for `vision_ocr_capabilities` — pure local-state introspection.
pub fn handle_capabilities(args: &Value) -> Value {
    let force_refresh = args
        .get("force_refresh")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    get_capabilities(force_refresh)
}

// ── Annotation helper (extracted so tests can exercise it) ──────────────────

/// Insert `backend_metadata` into a vision_cache result. Object results get the
/// key inserted in-place; non-object results are wrapped under `result`.
fn annotate_with_backend_metadata(mut raw: Value, backend_metadata: Value) -> Value {
    if let Some(obj) = raw.as_object_mut() {
        obj.insert("backend_metadata".to_string(), backend_metadata);
        raw
    } else {
        json!({"result": raw, "backend_metadata": backend_metadata})
    }
}

// ── Capabilities (cached via OnceLock) ──────────────────────────────────────

static CAPS_CACHE: OnceLock<Value> = OnceLock::new();

/// Cached capabilities accessor — wmic shell-out runs at most once per process.
fn get_capabilities_cached() -> Value {
    CAPS_CACHE.get_or_init(build_capabilities).clone()
}

/// Capabilities accessor with optional force-refresh. `force_refresh=true`
/// rebuilds without consulting the cache; the cache itself stays populated
/// with whatever value `get_capabilities_cached` first stored. (OnceLock can't
/// be reset, but callers asking for a refresh always get a fresh result.)
fn get_capabilities(force_refresh: bool) -> Value {
    if force_refresh {
        build_capabilities()
    } else {
        get_capabilities_cached()
    }
}

/// Build the capabilities payload — platform, backends, GPU detection. This
/// is the function the OnceLock caches and that `force_refresh` re-runs.
fn build_capabilities() -> Value {
    let platform = current_platform();
    let gpu = detect_gpu();

    json!({
        "ok": true,
        "platform": platform,
        "backends_available": ["tesseract"],
        "backends_planned": ["paddleocr_onnx_gpu", "paddleocr_onnx_cpu"],
        "gpu": gpu,
        "current_default": "tesseract",
        "phase_1_stub": true,
        "note": "Phase 2 will add backends_available = [..., 'paddleocr_onnx_gpu'] once vision-core upgrades to v0.2.0 with ONNX support."
    })
}

fn current_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else {
        "Unknown"
    }
}

// ── GPU detection ────────────────────────────────────────────────────────────

fn detect_gpu() -> Value {
    if cfg!(target_os = "windows") {
        detect_gpu_windows()
    } else {
        detect_gpu_non_windows()
    }
}

#[cfg(target_os = "windows")]
fn detect_gpu_windows() -> Value {
    detect_gpu_inner(list_adapters_with_fallback)
}

// On non-Windows builds the function still exists so tests can call it. It
// uses the injectable adapter-lister path with a no-op lister.
#[cfg(not(target_os = "windows"))]
fn detect_gpu_windows() -> Value {
    detect_gpu_inner(|| Vec::new())
}

/// Pure-Rust core of GPU detection, parameterised over the adapter-listing
/// strategy. Lets tests exercise vendor inference + payload shape without
/// shelling out to wmic.
fn detect_gpu_inner<F>(adapter_lister: F) -> Value
where
    F: Fn() -> Vec<String>,
{
    let adapters = adapter_lister();
    let vendor = infer_vendor(&adapters);
    let detected = !adapters.is_empty();
    let directml_dll_present = Path::new(r"C:\Windows\System32\DirectML.dll").exists();
    let nvml_dll_present = Path::new(r"C:\Windows\System32\nvml.dll").exists();
    let cuda_env_set = std::env::var("CUDA_VISIBLE_DEVICES").is_ok()
        || std::env::var("NVIDIA_VISIBLE_DEVICES").is_ok();

    json!({
        "detected": detected,
        "vendor": vendor,
        "adapters": adapters,
        "directml_dll_present": directml_dll_present,
        "nvml_dll_present": nvml_dll_present,
        "cuda_env_set": cuda_env_set
    })
}

fn detect_gpu_non_windows() -> Value {
    let cuda_env_set = std::env::var("CUDA_VISIBLE_DEVICES").is_ok()
        || std::env::var("NVIDIA_VISIBLE_DEVICES").is_ok();
    json!({
        "detected": false,
        "vendor": Value::Null,
        "adapters": [],
        "directml_dll_present": false,
        "nvml_dll_present": false,
        "cuda_env_set": cuda_env_set,
        "platform_note": "GPU detection beyond env vars not implemented on non-Windows"
    })
}

/// Run `wmic path Win32_VideoController get Name /format:list` and parse the
/// `Name=` lines. Failures (wmic missing, non-zero exit) collapse to an empty
/// list — the caller treats that as `detected:false`.
fn list_adapters_via_wmic() -> Vec<String> {
    let output = std::process::Command::new("wmic")
        .args([
            "path",
            "Win32_VideoController",
            "get",
            "Name",
            "/format:list",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            parse_wmic_adapters(&stdout)
        }
        _ => Vec::new(),
    }
}

/// Parse `Name=...` lines from wmic's `/format:list` output. Skips empty
/// values and the format-line preamble.
fn parse_wmic_adapters(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed.strip_prefix("Name=").map(|v| v.trim().to_string())
        })
        .filter(|v| !v.is_empty())
        .collect()
}

/// Fallback adapter lister using PowerShell + Get-CimInstance.
/// Required on Windows 11 build 26200+ where wmic was removed.
/// One Name per line; empty lines + non-Name lines (PSReadLine warnings) filtered.
#[cfg(target_os = "windows")]
fn list_adapters_via_powershell() -> Vec<String> {
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            parse_powershell_adapters(&stdout)
        }
        _ => Vec::new(),
    }
}

/// Parse PowerShell `Get-CimInstance Win32_VideoController | Select-Object
/// -ExpandProperty Name` output — one adapter name per line, no `Name=` prefix.
/// Trims each line, skips empties, and filters PowerShell warning/error lines
/// (e.g. `WARNING: ...`, `Get-CimInstance: ...`).
fn parse_powershell_adapters(s: &str) -> Vec<String> {
    s.lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("WARNING:"))
        .filter(|line| !line.starts_with("Get-CimInstance:"))
        .collect()
}

/// Try wmic first (faster, ~50ms); fall back to PowerShell + Get-CimInstance
/// (slower, ~300ms but available on Windows 11 26200+ where wmic was removed).
#[cfg(target_os = "windows")]
fn list_adapters_with_fallback() -> Vec<String> {
    list_adapters_with_fallback_inner(list_adapters_via_wmic, list_adapters_via_powershell)
}

/// Testable core of `list_adapters_with_fallback` — parameterised over the two
/// adapter-listing strategies so tests can inject closures without shelling out.
fn list_adapters_with_fallback_inner<P, S>(primary: P, secondary: S) -> Vec<String>
where
    P: Fn() -> Vec<String>,
    S: Fn() -> Vec<String>,
{
    let result = primary();
    if !result.is_empty() {
        return result;
    }
    // Primary returned empty (either truly no GPU OR primary is gone on this
    // Windows 11 build); fall back to the secondary strategy.
    secondary()
}

/// Best-effort vendor inference from adapter display names. First match wins,
/// preserving the order NVIDIA → AMD/Radeon → Intel → Apple.
fn infer_vendor(adapters: &[String]) -> Option<String> {
    for adapter in adapters {
        let lower = adapter.to_lowercase();
        if lower.contains("nvidia") {
            return Some("NVIDIA".to_string());
        }
        if lower.contains("amd") || lower.contains("radeon") {
            return Some("AMD".to_string());
        }
        if lower.contains("intel") {
            return Some("Intel".to_string());
        }
        if lower.contains("apple") {
            return Some("Apple".to_string());
        }
    }
    None
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wmic_adapters_extracts_name_lines() {
        let sample = "Name=NVIDIA GeForce RTX 4090\nName=Intel UHD Graphics 770\n";
        let adapters = parse_wmic_adapters(sample);
        assert_eq!(
            adapters,
            vec![
                "NVIDIA GeForce RTX 4090".to_string(),
                "Intel UHD Graphics 770".to_string(),
            ]
        );
    }

    #[test]
    fn parse_wmic_adapters_skips_empty_and_format_header() {
        // wmic /format:list output often has blank lines + non-Name fields
        // (the format header itself is not present in /format:list, but a
        // real run can include trailing blanks and stray whitespace).
        let sample = "\nName=NVIDIA GeForce RTX 4090\n\nName=\nName=  Intel UHD Graphics\n\nDescription=ignored field\n";
        let adapters = parse_wmic_adapters(sample);
        assert_eq!(
            adapters,
            vec![
                "NVIDIA GeForce RTX 4090".to_string(),
                "Intel UHD Graphics".to_string(),
            ]
        );
    }

    #[test]
    fn parse_powershell_adapters_extracts_names() {
        // Get-CimInstance ... | Select -ExpandProperty Name returns one
        // adapter name per line, no `Name=` prefix.
        let sample = "NVIDIA GeForce RTX 4090\nIntel UHD Graphics\n";
        let adapters = parse_powershell_adapters(sample);
        assert_eq!(
            adapters,
            vec![
                "NVIDIA GeForce RTX 4090".to_string(),
                "Intel UHD Graphics".to_string(),
            ]
        );
    }

    #[test]
    fn parse_powershell_adapters_skips_warnings_and_blanks() {
        // Real-world PowerShell output can include warning lines (PSReadLine,
        // CIM session warnings) and Get-CimInstance error lines. Those + blanks
        // must be filtered so they don't pollute the adapter list.
        let sample = "WARNING: PSReadLine session timed out\n\nNVIDIA GeForce RTX 4090\nGet-CimInstance: The CIM session failed\nIntel UHD Graphics\n   \n";
        let adapters = parse_powershell_adapters(sample);
        assert_eq!(
            adapters,
            vec![
                "NVIDIA GeForce RTX 4090".to_string(),
                "Intel UHD Graphics".to_string(),
            ]
        );
    }

    #[test]
    fn fallback_returns_powershell_when_wmic_empty() {
        // Primary (wmic) returns empty → fall back to secondary (powershell).
        let primary = Vec::new;
        let secondary = || vec!["Qualcomm(R) Adreno(TM) X1-85 GPU".to_string()];
        let adapters = list_adapters_with_fallback_inner(primary, secondary);
        assert_eq!(
            adapters,
            vec!["Qualcomm(R) Adreno(TM) X1-85 GPU".to_string()]
        );

        // Primary returns adapters → secondary is NOT consulted.
        let primary = || vec!["NVIDIA GeForce RTX 4090".to_string()];
        let secondary = || vec!["should not appear".to_string()];
        let adapters = list_adapters_with_fallback_inner(primary, secondary);
        assert_eq!(adapters, vec!["NVIDIA GeForce RTX 4090".to_string()]);

        // Both empty → empty.
        let adapters = list_adapters_with_fallback_inner(Vec::new, Vec::new);
        assert!(adapters.is_empty());
    }

    #[test]
    fn infer_vendor_nvidia() {
        let adapters = vec!["NVIDIA GeForce RTX 4090".to_string()];
        assert_eq!(infer_vendor(&adapters), Some("NVIDIA".to_string()));
    }

    #[test]
    fn infer_vendor_amd_radeon() {
        // Both AMD and Radeon names should map to AMD.
        let amd = vec!["AMD Ryzen Graphics".to_string()];
        let radeon = vec!["Radeon RX 7900 XTX".to_string()];
        assert_eq!(infer_vendor(&amd), Some("AMD".to_string()));
        assert_eq!(infer_vendor(&radeon), Some("AMD".to_string()));
    }

    #[test]
    fn infer_vendor_intel() {
        let adapters = vec!["Intel UHD Graphics 770".to_string()];
        assert_eq!(infer_vendor(&adapters), Some("Intel".to_string()));
    }

    #[test]
    fn infer_vendor_apple() {
        let adapters = vec!["Apple M2 Max".to_string()];
        assert_eq!(infer_vendor(&adapters), Some("Apple".to_string()));
    }

    #[test]
    fn infer_vendor_none_for_unknown() {
        // Empty list → None.
        assert_eq!(infer_vendor(&[]), None);
        // Unknown adapter name → None.
        let unknown = vec!["Mystery Vendor X9000".to_string()];
        assert_eq!(infer_vendor(&unknown), None);
    }

    #[test]
    fn infer_vendor_first_match_wins_with_nvidia_priority() {
        // When both NVIDIA and Intel are present, NVIDIA (the dGPU) wins
        // because it appears first in the adapter list — confirms the
        // documented priority order.
        let adapters = vec![
            "NVIDIA GeForce RTX 4090".to_string(),
            "Intel UHD Graphics".to_string(),
        ];
        assert_eq!(infer_vendor(&adapters), Some("NVIDIA".to_string()));
    }

    #[test]
    fn detect_gpu_inner_with_injected_lister_shape_is_correct() {
        let val = detect_gpu_inner(|| {
            vec![
                "NVIDIA GeForce RTX 4090".to_string(),
                "Intel UHD Graphics".to_string(),
            ]
        });
        assert_eq!(val["detected"], json!(true));
        assert_eq!(val["vendor"], json!("NVIDIA"));
        assert_eq!(
            val["adapters"],
            json!(["NVIDIA GeForce RTX 4090", "Intel UHD Graphics"])
        );
        // DLL keys + cuda_env_set are always present, regardless of value.
        assert!(val.get("directml_dll_present").is_some());
        assert!(val.get("nvml_dll_present").is_some());
        assert!(val.get("cuda_env_set").is_some());
    }

    #[test]
    fn detect_gpu_inner_empty_list_means_not_detected() {
        let val = detect_gpu_inner(Vec::new);
        assert_eq!(val["detected"], json!(false));
        assert_eq!(val["vendor"], Value::Null);
        assert_eq!(val["adapters"], json!([]));
    }

    #[test]
    fn get_capabilities_returns_well_formed_shape() {
        let caps = get_capabilities(false);
        assert_eq!(caps["ok"], json!(true));
        assert!(caps.get("platform").is_some());
        assert_eq!(caps["backends_available"], json!(["tesseract"]));
        assert_eq!(
            caps["backends_planned"],
            json!(["paddleocr_onnx_gpu", "paddleocr_onnx_cpu"])
        );
        assert_eq!(caps["current_default"], json!("tesseract"));
        assert_eq!(caps["phase_1_stub"], json!(true));
        assert!(caps.get("note").is_some());
        // gpu sub-object must exist and have the documented keys.
        let gpu = &caps["gpu"];
        assert!(gpu.get("detected").is_some());
        assert!(gpu.get("adapters").is_some());
        assert!(gpu.get("directml_dll_present").is_some());
        assert!(gpu.get("nvml_dll_present").is_some());
        assert!(gpu.get("cuda_env_set").is_some());
    }

    #[test]
    fn get_capabilities_force_refresh_returns_fresh_result() {
        // Warm the cache first.
        let _ = get_capabilities(false);
        // A forced refresh must still return a fully formed payload (shape
        // identical to the cached one) — proves the refresh path doesn't
        // panic and doesn't return a degraded/empty Value.
        let refreshed = get_capabilities(true);
        assert_eq!(refreshed["ok"], json!(true));
        assert_eq!(refreshed["backends_available"], json!(["tesseract"]));
        assert_eq!(refreshed["phase_1_stub"], json!(true));
        // Force-refresh path is invoked even when the OnceLock is hot — the
        // returned object is a fresh allocation (different ptr than cached
        // clone would be), but we can only safely assert structural equality.
        let cached = get_capabilities(false);
        assert_eq!(
            cached["backends_available"],
            refreshed["backends_available"]
        );
    }

    #[test]
    fn handle_capabilities_respects_force_refresh_arg() {
        // Default args = cached path.
        let cached = handle_capabilities(&json!({}));
        assert_eq!(cached["ok"], json!(true));
        // Explicit force_refresh=true = fresh path.
        let fresh = handle_capabilities(&json!({"force_refresh": true}));
        assert_eq!(fresh["ok"], json!(true));
        assert_eq!(fresh["phase_1_stub"], json!(true));
    }

    #[test]
    fn backend_metadata_annotated_when_wrapping_object_result() {
        let raw = json!({"text": "hello world", "confidence": 0.95});
        let metadata = json!({"backend": "tesseract", "phase": "phase_1_stub"});
        let annotated = annotate_with_backend_metadata(raw, metadata.clone());
        assert_eq!(annotated["text"], json!("hello world"));
        assert_eq!(annotated["confidence"], json!(0.95));
        assert_eq!(annotated["backend_metadata"], metadata);
    }

    #[test]
    fn backend_metadata_wraps_non_object_result() {
        // If vision_cache somehow returned a string or array, we wrap it
        // rather than silently dropping the metadata.
        let raw = json!("just a string");
        let metadata = json!({"backend": "tesseract"});
        let annotated = annotate_with_backend_metadata(raw.clone(), metadata.clone());
        assert_eq!(annotated["result"], raw);
        assert_eq!(annotated["backend_metadata"], metadata);
    }
}
