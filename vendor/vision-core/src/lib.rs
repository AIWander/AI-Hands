//! Vision Core - shared screenshot, OCR, and image analysis library
//! Used by: local (mcp-windows), programmer (antigravity), browser
//! All tools exposed with vision_ prefix to show lineage
// NAV: TOC at end of file | 2026-05-31

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use windows::Graphics::Imaging::BitmapDecoder;
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn default_capture_path(prefix: &str) -> String {
    let directory = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join("Pictures").join("Screenshots"))
        .unwrap_or_else(|| std::env::temp_dir().join("hands-captures"));
    let _ = std::fs::create_dir_all(&directory);
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%9f");
    let counter = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    directory
        .join(format!(
            "{}_{}_{}_{}.png",
            prefix,
            timestamp,
            std::process::id(),
            counter
        ))
        .to_string_lossy()
        .into_owned()
}

// Experimental cross-platform OCR backend. Entirely behind the `onnx` feature so
// the default build carries zero extra weight (no ort / ndarray / ONNX Runtime).
#[cfg(feature = "onnx")]
mod paddle_onnx;

// === OCR BACKEND SELECTION ===

/// Which OCR engine the public `ocr_image*` fns dispatch to.
/// Windows.Media.Ocr is the default and only verified backend. Paddle is
/// experimental and requires the crate to be built with `--features onnx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OcrBackend {
    Windows,
    Paddle,
}

/// Error returned when the Paddle backend is requested but the crate was built
/// without the `onnx` feature. Only referenced on default (non-onnx) builds.
#[cfg(not(feature = "onnx"))]
const PADDLE_NOT_COMPILED: &str = "VISION_CORE_OCR_BACKEND=paddle requested but vision-core was built without the `onnx` feature. Rebuild with --features onnx, or unset the env var to use Windows OCR.";

/// Pick the OCR backend from `VISION_CORE_OCR_BACKEND`.
/// Recognized opt-in values: `paddle`, `onnx`, `paddleocr` -> Paddle.
/// Anything else (or unset) -> Windows (default, unchanged behavior).
fn selected_backend() -> OcrBackend {
    match std::env::var("VISION_CORE_OCR_BACKEND").as_deref() {
        Ok("paddle") | Ok("onnx") | Ok("paddleocr") => OcrBackend::Paddle,
        _ => OcrBackend::Windows,
    }
}

// === TOOL DEFINITIONS ===

/// Local-only vision tools (no API calls, no tokens)
pub fn get_local_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "vision_screenshot",
            "description": "[Vision] Take a screenshot of the screen or a specific region. Returns the image path and optionally base64 data.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "save_path": { "type": "string", "description": "Path to save screenshot (optional, uses temp if not provided)" },
                    "return_base64": { "type": "boolean", "description": "Return base64 encoded image data (default: false)", "default": false },
                    "monitor": { "type": "integer", "description": "Monitor index (0 = primary)", "default": 0 },
                    "region": {
                        "type": "object",
                        "description": "Capture specific region: {x, y, width, height}",
                        "properties": {
                            "x": {"type": "integer"}, "y": {"type": "integer"},
                            "width": {"type": "integer"}, "height": {"type": "integer"}
                        }
                    },
                    "quality": { "type": "integer", "description": "JPEG quality 1-100 (default: 80)", "default": 80, "minimum": 1, "maximum": 100 }
                }
            }
        }),
        json!({
            "name": "vision_ocr",
            "description": "[Vision] Extract text from an image using Windows OCR. Local processing, no tokens used.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "image_path": { "type": "string", "description": "Path to image file" },
                    "language": { "type": "string", "description": "OCR language (default: en-US)", "default": "en-US" }
                },
                "required": ["image_path"]
            }
        }),
        json!({
            "name": "vision_screenshot_ocr",
            "description": "[Vision] Take screenshot and immediately OCR it. Returns extracted text. Local processing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "monitor": { "type": "integer", "description": "Monitor index (0 = primary)", "default": 0 },
                    "save_screenshot": { "type": "string", "description": "Optional path to save the screenshot" },
                    "region": {
                        "type": "object",
                        "description": "Capture specific region: {x, y, width, height}",
                        "properties": {
                            "x": {"type": "integer"}, "y": {"type": "integer"},
                            "width": {"type": "integer"}, "height": {"type": "integer"}
                        }
                    }
                }
            }
        }),
        json!({
            "name": "vision_check_user_input",
            "description": "[Vision] Screenshot and OCR bottom of screen to detect if user typed during tool execution.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "height": { "type": "integer", "description": "Height from bottom to capture (default: 200px)", "default": 200 },
                    "monitor": { "type": "integer", "description": "Monitor index (default: 0)", "default": 0 }
                }
            }
        }),
        // vision_analyze excluded — API tool, see get_api_definitions()
        json!({
            "name": "vision_load_image",
            "description": "[Vision] Load image as base64 for direct vision analysis in chat.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "image_path": { "type": "string", "description": "Path to image file" }
                },
                "required": ["image_path"]
            }
        }),
        json!({
            "name": "vision_diff",
            "description": "[Vision] Compare two images and return difference percentage. Use 'screen' as image_b to compare against current screen.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "image_a": { "type": "string", "description": "Path to first image (reference/before)" },
                    "image_b": { "type": "string", "description": "Path to second image, or 'screen' for current screen" },
                    "threshold": { "type": "number", "description": "Pixel difference threshold 0-255 (default: 30)", "default": 30 },
                    "monitor": { "type": "integer", "description": "Monitor index if image_b is 'screen' (default: 0)", "default": 0 }
                },
                "required": ["image_a", "image_b"]
            }
        }),
        json!({
            "name": "vision_zoom",
            "description": "Crop and magnify a screen region for closer inspection. Takes a screenshot, crops to the specified rectangle, and scales up.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": {"type": "integer", "description": "Left edge of region"},
                    "y": {"type": "integer", "description": "Top edge of region"},
                    "width": {"type": "integer", "description": "Width of region"},
                    "height": {"type": "integer", "description": "Height of region"},
                    "scale": {"type": "number", "description": "Magnification factor (default 2.0)", "default": 2.0}
                },
                "required": ["x", "y", "width", "height"]
            }
        }),
        json!({
            "name": "vision_find_template",
            "description": "[Vision] Find a template image within the screen or another image. Returns best match location and confidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "template": { "type": "string", "description": "Path to template image to find" },
                    "search_in": { "type": "string", "description": "Path to image or 'screen' (default: screen)" },
                    "threshold": { "type": "number", "description": "Match confidence 0.0-1.0 (default: 0.8)", "default": 0.8 },
                    "monitor": { "type": "integer", "description": "Monitor index if search_in is 'screen' (default: 0)", "default": 0 }
                },
                "required": ["template"]
            }
        }),
    ]
}

/// API-calling vision tools (uses Claude tokens). Programmer-only.
pub fn get_api_definitions() -> Vec<Value> {
    vec![json!({
        "name": "vision_analyze",
        "description": "[Vision] Analyze image with Claude Vision API. Uses tokens but provides intelligent analysis.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "image_path": { "type": "string", "description": "Path to image file" },
                "query": { "type": "string", "description": "What to analyze/look for", "default": "Describe this image in detail" }
            },
            "required": ["image_path"]
        }
    })]
}

/// All vision tools (local + API). For programmer server.
pub fn get_all_definitions() -> Vec<Value> {
    let mut defs = get_local_definitions();
    defs.extend(get_api_definitions());
    defs
}

// === HELPER FUNCTIONS ===

pub fn save_image(img: &image::RgbaImage, path: &str, quality: u8) -> Result<(), String> {
    let extension = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();

    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    match extension.as_str() {
        "jpg" | "jpeg" => {
            let rgb_img: image::RgbImage = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            let mut output =
                std::fs::File::create(path).map_err(|e| format!("Failed to create file: {}", e))?;
            let mut encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality);
            encoder
                .encode_image(&rgb_img)
                .map_err(|e| format!("JPEG encode failed: {}", e))?;
            Ok(())
        }
        _ => img.save(path).map_err(|e| format!("Failed to save: {}", e)),
    }
}

pub fn take_screenshot(
    save_path: Option<&str>,
    monitor: usize,
    quality: u8,
) -> Result<String, String> {
    use screenshots::Screen;

    let screens = Screen::all().map_err(|e| format!("Failed to get screens: {}", e))?;
    if monitor >= screens.len() {
        return Err(format!(
            "Monitor {} not found. Available: 0-{}",
            monitor,
            screens.len() - 1
        ));
    }

    let screen = &screens[monitor];
    let image = screen
        .capture()
        .map_err(|e| format!("Screenshot failed: {}", e))?;

    let path = save_path
        .map(|p| p.to_string())
        .unwrap_or_else(|| default_capture_path("screenshot"));

    save_image(&image, &path, quality)?;
    Ok(path)
}

pub fn take_screenshot_region(
    save_path: Option<&str>,
    monitor: usize,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    quality: u8,
) -> Result<String, String> {
    use screenshots::Screen;

    let screens = Screen::all().map_err(|e| format!("Failed to get screens: {}", e))?;
    if monitor >= screens.len() {
        return Err(format!(
            "Monitor {} not found. Available: 0-{}",
            monitor,
            screens.len() - 1
        ));
    }

    let screen = &screens[monitor];
    let full_image = screen
        .capture()
        .map_err(|e| format!("Screenshot failed: {}", e))?;

    let cropped = image::imageops::crop_imm(
        &full_image,
        x.max(0) as u32,
        y.max(0) as u32,
        width.min(full_image.width()),
        height.min(full_image.height()),
    )
    .to_image();

    let path = save_path
        .map(|p| p.to_string())
        .unwrap_or_else(|| default_capture_path("region"));

    save_image(&cropped, &path, quality)?;
    Ok(path)
}

pub fn get_screen_dimensions(monitor: usize) -> Result<(u32, u32), String> {
    use screenshots::Screen;
    let screens = Screen::all().map_err(|e| format!("Failed to get screens: {}", e))?;
    if monitor >= screens.len() {
        return Err(format!("Monitor {} not found", monitor));
    }
    let info = screens[monitor].display_info;
    Ok((info.width, info.height))
}

pub fn image_to_base64(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read image: {}", e))?;
    Ok(BASE64.encode(&bytes))
}

/// OCR an image to plain text. PUBLIC API — signature is stable.
///
/// Dispatches to the backend chosen by `VISION_CORE_OCR_BACKEND` (default:
/// Windows.Media.Ocr). The `language` hint is forwarded to the Windows path
/// (currently ignored there, preserved for API compatibility).
pub async fn ocr_image(image_path: &str, language: &str) -> Result<String, String> {
    match selected_backend() {
        #[cfg(feature = "onnx")]
        OcrBackend::Paddle => paddle_onnx::ocr_image(image_path).await,
        #[cfg(not(feature = "onnx"))]
        OcrBackend::Paddle => Err(PADDLE_NOT_COMPILED.to_string()),
        OcrBackend::Windows => windows_ocr_image(image_path, language).await,
    }
}

/// Windows.Media.Ocr text extraction (default backend). Body moved verbatim
/// from the previous public `ocr_image`; behavior is unchanged.
async fn windows_ocr_image(image_path: &str, _language: &str) -> Result<String, String> {
    let path = std::path::Path::new(image_path);
    if !path.exists() {
        return Err(format!("Image not found: {}", image_path));
    }

    let image_bytes =
        std::fs::read(image_path).map_err(|e| format!("Failed to read image: {}", e))?;

    let stream =
        InMemoryRandomAccessStream::new().map_err(|e| format!("Failed to create stream: {}", e))?;

    let writer = DataWriter::CreateDataWriter(&stream)
        .map_err(|e| format!("Failed to create writer: {}", e))?;

    writer
        .WriteBytes(&image_bytes)
        .map_err(|e| format!("Failed to write bytes: {}", e))?;

    writer
        .StoreAsync()
        .map_err(|e| format!("Failed to store: {}", e))?
        .get()
        .map_err(|e| format!("Failed to get store result: {}", e))?;

    writer
        .FlushAsync()
        .map_err(|e| format!("Failed to flush: {}", e))?
        .get()
        .map_err(|e| format!("Failed to get flush result: {}", e))?;

    stream
        .Seek(0)
        .map_err(|e| format!("Failed to seek: {}", e))?;

    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|e| format!("Failed to create decoder: {}", e))?
        .get()
        .map_err(|e| format!("Failed to get decoder: {}", e))?;

    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|e| format!("Failed to get bitmap: {}", e))?
        .get()
        .map_err(|e| format!("Failed to get bitmap result: {}", e))?;

    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|e| format!("Failed to create OCR engine: {}", e))?;

    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| format!("OCR failed: {}", e))?
        .get()
        .map_err(|e| format!("Failed to get OCR result: {}", e))?;

    let text = result
        .Text()
        .map_err(|e| format!("Failed to get text: {}", e))?;

    Ok(text.to_string())
}

/// OCR an image and return word-level bounding boxes: Vec<(word, x, y, width, height)>.
/// PUBLIC API — signature is stable. Dispatches per `VISION_CORE_OCR_BACKEND`.
pub async fn ocr_image_with_positions(
    image_path: &str,
) -> Result<Vec<(String, f64, f64, f64, f64)>, String> {
    match selected_backend() {
        #[cfg(feature = "onnx")]
        OcrBackend::Paddle => paddle_onnx::ocr_image_with_positions(image_path).await,
        #[cfg(not(feature = "onnx"))]
        OcrBackend::Paddle => Err(PADDLE_NOT_COMPILED.to_string()),
        OcrBackend::Windows => windows_ocr_image_with_positions(image_path).await,
    }
}

/// Windows.Media.Ocr word-box extraction (default backend). Body moved verbatim
/// from the previous public `ocr_image_with_positions`; behavior is unchanged.
async fn windows_ocr_image_with_positions(
    image_path: &str,
) -> Result<Vec<(String, f64, f64, f64, f64)>, String> {
    let path = std::path::Path::new(image_path);
    if !path.exists() {
        return Err(format!("Image not found: {}", image_path));
    }
    let image_bytes =
        std::fs::read(image_path).map_err(|e| format!("Failed to read image: {}", e))?;
    let stream =
        InMemoryRandomAccessStream::new().map_err(|e| format!("Failed to create stream: {}", e))?;
    let writer = DataWriter::CreateDataWriter(&stream)
        .map_err(|e| format!("Failed to create writer: {}", e))?;
    writer
        .WriteBytes(&image_bytes)
        .map_err(|e| format!("Failed to write bytes: {}", e))?;
    writer
        .StoreAsync()
        .map_err(|e| format!("Failed to store: {}", e))?
        .get()
        .map_err(|e| format!("Failed to get store result: {}", e))?;
    writer
        .FlushAsync()
        .map_err(|e| format!("Failed to flush: {}", e))?
        .get()
        .map_err(|e| format!("Failed to get flush result: {}", e))?;
    stream
        .Seek(0)
        .map_err(|e| format!("Failed to seek: {}", e))?;
    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|e| format!("Failed to create decoder: {}", e))?
        .get()
        .map_err(|e| format!("Failed to get decoder: {}", e))?;
    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|e| format!("Failed to get bitmap: {}", e))?
        .get()
        .map_err(|e| format!("Failed to get bitmap result: {}", e))?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|e| format!("Failed to create OCR engine: {}", e))?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| format!("OCR failed: {}", e))?
        .get()
        .map_err(|e| format!("Failed to get OCR result: {}", e))?;

    let mut words = Vec::new();
    let lines = result
        .Lines()
        .map_err(|e| format!("Failed to get lines: {}", e))?;
    for line in &lines {
        let line_words = line
            .Words()
            .map_err(|e| format!("Failed to get words: {}", e))?;
        for word in &line_words {
            let text = word
                .Text()
                .map_err(|e| format!("Failed to get word text: {}", e))?
                .to_string();
            let rect = word
                .BoundingRect()
                .map_err(|e| format!("Failed to get bounding rect: {}", e))?;
            words.push((
                text,
                rect.X as f64,
                rect.Y as f64,
                rect.Width as f64,
                rect.Height as f64,
            ));
        }
    }
    Ok(words)
}

pub async fn analyze_with_claude(image_path: &str, query: &str) -> Result<String, String> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| "ANTHROPIC_API_KEY not set. Set it to use Claude Vision.".to_string())?;

    let base64_data = image_to_base64(image_path)?;

    let extension = Path::new(image_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();

    let media_type = match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    };

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "image", "source": { "type": "base64", "media_type": media_type, "data": base64_data } },
                    { "type": "text", "text": query }
                ]
            }]
        }))
        .send()
        .await
        .map_err(|e| format!("API request failed: {}", e))?;

    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if let Some(content) = json["content"].as_array() {
        if let Some(first) = content.first() {
            if let Some(text) = first["text"].as_str() {
                return Ok(text.to_string());
            }
        }
    }

    Err(format!("Unexpected API response: {}", json))
}

fn load_image_rgba(path: &str) -> Result<image::RgbaImage, String> {
    Ok(image::open(path)
        .map_err(|e| format!("Failed to load image {}: {}", path, e))?
        .to_rgba8())
}

fn diff_images(
    img_a: &image::RgbaImage,
    img_b: &image::RgbaImage,
    threshold: u8,
) -> (f64, u32, u32) {
    let (w, h) = img_a.dimensions();
    let img_b = if img_b.dimensions() != (w, h) {
        image::imageops::resize(img_b, w, h, image::imageops::FilterType::Nearest)
    } else {
        img_b.clone()
    };

    let mut diff_pixels = 0u64;
    let total_pixels = (w as u64) * (h as u64);

    for (pa, pb) in img_a.pixels().zip(img_b.pixels()) {
        let dr = (pa[0] as i32 - pb[0] as i32).abs();
        let dg = (pa[1] as i32 - pb[1] as i32).abs();
        let db = (pa[2] as i32 - pb[2] as i32).abs();
        if dr > threshold as i32 || dg > threshold as i32 || db > threshold as i32 {
            diff_pixels += 1;
        }
    }

    let diff_percent = (diff_pixels as f64 / total_pixels as f64) * 100.0;
    (diff_percent, w, h)
}

fn find_template_match(
    haystack: &image::RgbaImage,
    needle: &image::RgbaImage,
    threshold: f64,
) -> Option<(u32, u32, f64)> {
    let (hw, hh) = haystack.dimensions();
    let (nw, nh) = needle.dimensions();

    if nw > hw || nh > hh {
        return None;
    }

    let mut best_match: Option<(u32, u32, f64)> = None;

    for y in 0..=(hh - nh) {
        for x in 0..=(hw - nw) {
            let mut match_sum = 0u64;
            let mut total = 0u64;

            for ny in 0..nh {
                for nx in 0..nw {
                    let hp = haystack.get_pixel(x + nx, y + ny);
                    let np = needle.get_pixel(nx, ny);
                    if np[3] < 128 {
                        continue;
                    }
                    total += 1;
                    let diff = (hp[0] as i32 - np[0] as i32).abs()
                        + (hp[1] as i32 - np[1] as i32).abs()
                        + (hp[2] as i32 - np[2] as i32).abs();
                    match_sum += (765 - diff) as u64;
                }
            }

            if total == 0 {
                continue;
            }
            let confidence = (match_sum as f64) / (total as f64 * 765.0);
            if confidence >= threshold {
                if best_match.is_none() || confidence > best_match.unwrap().2 {
                    best_match = Some((x, y, confidence));
                }
            }
        }
    }

    best_match
}

// === CAPABILITY REPORTER ===

/// Report which OCR backends this build exposes and which one is active.
///
/// `onnx_compiled` reflects whether the crate was built with `--features onnx`.
/// `paddle_models_present` is true only if all three Paddle env paths
/// (`VISION_CORE_PADDLE_DET_MODEL`, `_REC_MODEL`, `_DICT`) are set AND exist.
pub fn vision_ocr_backends() -> Value {
    let onnx_compiled = cfg!(feature = "onnx");

    let mut available = vec!["windows_media_ocr"];
    if onnx_compiled {
        available.push("paddleocr_onnx");
    }

    let active = match selected_backend() {
        OcrBackend::Windows => "windows_media_ocr",
        OcrBackend::Paddle => "paddleocr_onnx",
    };

    let paddle_models_present = [
        "VISION_CORE_PADDLE_DET_MODEL",
        "VISION_CORE_PADDLE_REC_MODEL",
        "VISION_CORE_PADDLE_DICT",
    ]
    .iter()
    .all(|var| {
        std::env::var(var)
            .ok()
            .filter(|p| !p.is_empty())
            .map(|p| Path::new(&p).exists())
            .unwrap_or(false)
    });

    json!({
        "default": "windows_media_ocr",
        "available": available,
        "onnx_compiled": onnx_compiled,
        "active": active,
        "paddle_models_present": paddle_models_present,
        "note": "PaddleOCR-ONNX is experimental; set VISION_CORE_OCR_BACKEND=paddle + the 3 model env vars to enable. Windows.Media.Ocr is the default and only verified backend."
    })
}

// === MAIN DISPATCH ===

/// Execute a vision tool. Call with tokio block_on if in sync context.
pub async fn execute(name: &str, args: &Value) -> Value {
    match execute_inner(name, args).await {
        Ok(v) => v,
        Err(e) => json!({"error": e}),
    }
}

async fn execute_inner(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "vision_screenshot" => {
            let save_path = args["save_path"].as_str();
            let return_base64 = args["return_base64"].as_bool().unwrap_or(false);
            let monitor = args["monitor"].as_u64().unwrap_or(0) as usize;
            let quality = args["quality"].as_u64().unwrap_or(80) as u8;

            let path = if let Some(region) = args.get("region").filter(|r| r.is_object()) {
                let x = region["x"].as_i64().unwrap_or(0) as i32;
                let y = region["y"].as_i64().unwrap_or(0) as i32;
                let width = region["width"].as_u64().unwrap_or(400) as u32;
                let height = region["height"].as_u64().unwrap_or(200) as u32;
                take_screenshot_region(save_path, monitor, x, y, width, height, quality)?
            } else {
                take_screenshot(save_path, monitor, quality)?
            };

            let mut result = json!({ "success": true, "path": path, "monitor": monitor });
            if return_base64 {
                result["base64"] = json!(image_to_base64(&path)?);
            }
            Ok(result)
        }

        "vision_ocr" => {
            let image_path = args["image_path"]
                .as_str()
                .ok_or("image_path is required")?;
            let language = args["language"].as_str().unwrap_or("en-US");
            let text = ocr_image(image_path, language).await?;
            Ok(
                json!({ "success": true, "text": text, "image_path": image_path, "chars": text.len() }),
            )
        }

        "vision_screenshot_ocr" => {
            let monitor = args["monitor"].as_u64().unwrap_or(0) as usize;
            let save_path = args["save_screenshot"].as_str();

            let path = if let Some(region) = args.get("region").filter(|r| r.is_object()) {
                let x = region["x"].as_i64().unwrap_or(0) as i32;
                let y = region["y"].as_i64().unwrap_or(0) as i32;
                let width = region["width"].as_u64().unwrap_or(400) as u32;
                let height = region["height"].as_u64().unwrap_or(200) as u32;
                take_screenshot_region(save_path, monitor, x, y, width, height, 80)?
            } else {
                take_screenshot(save_path, monitor, 80)?
            };

            let text = ocr_image(&path, "en-US").await?;
            if save_path.is_none() {
                std::fs::remove_file(&path).ok();
            }
            Ok(
                json!({ "success": true, "text": text, "chars": text.len(), "screenshot_saved": save_path.is_some() }),
            )
        }

        "vision_check_user_input" => {
            let height = args["height"].as_u64().unwrap_or(200) as u32;
            let monitor = args["monitor"].as_u64().unwrap_or(0) as usize;
            let (screen_width, screen_height) = get_screen_dimensions(monitor)?;
            let y = (screen_height - height) as i32;
            let path = take_screenshot_region(None, monitor, 0, y, screen_width, height, 80)?;
            let text = ocr_image(&path, "en-US").await?;
            std::fs::remove_file(&path).ok();
            let trimmed = text.trim();
            let has_input = !trimmed.is_empty()
                && trimmed.len() > 5
                && !trimmed.chars().all(|c| c.is_whitespace() || c == '|');
            Ok(json!({
                "success": true,
                "has_user_input": has_input,
                "text": if has_input { trimmed } else { "" },
                "chars": if has_input { trimmed.len() } else { 0 },
                "hint": if has_input { "User may have typed input" } else { "No user input detected" }
            }))
        }

        "vision_zoom" => {
            let x = args["x"].as_i64().ok_or("x is required")? as i32;
            let y = args["y"].as_i64().ok_or("y is required")? as i32;
            let width = args["width"].as_u64().ok_or("width is required")? as u32;
            let height = args["height"].as_u64().ok_or("height is required")? as u32;
            let scale = args["scale"].as_f64().unwrap_or(2.0);

            if width == 0 || height == 0 {
                return Err("width and height must be > 0".into());
            }
            if scale <= 0.0 || scale > 10.0 {
                return Err("scale must be between 0 and 10".into());
            }

            // Take a full screenshot and crop to the region
            let temp_path = take_screenshot_region(None, 0, x, y, width, height, 100)?;
            let img = image::open(&temp_path)
                .map_err(|e| format!("Failed to open cropped image: {}", e))?;
            std::fs::remove_file(&temp_path).ok();

            // Scale up
            let new_width = (width as f64 * scale) as u32;
            let new_height = (height as f64 * scale) as u32;
            let scaled = image::imageops::resize(
                &img,
                new_width,
                new_height,
                image::imageops::FilterType::Lanczos3,
            );

            // Save to temp file
            let out_path = default_capture_path("zoom");
            let rgba_img: image::RgbaImage = image::DynamicImage::ImageRgba8(scaled).to_rgba8();
            save_image(&rgba_img, &out_path, 100)?;

            Ok(json!({
                "success": true,
                "path": out_path,
                "region": {"x": x, "y": y, "width": width, "height": height},
                "scale": scale,
                "output_width": new_width,
                "output_height": new_height
            }))
        }

        "vision_analyze" => {
            let image_path = args["image_path"]
                .as_str()
                .ok_or("image_path is required")?;
            let query = args["query"]
                .as_str()
                .unwrap_or("Describe this image in detail");
            let analysis = analyze_with_claude(image_path, query).await?;
            Ok(
                json!({ "success": true, "analysis": analysis, "image_path": image_path, "note": "Used Claude Vision API (tokens consumed)" }),
            )
        }

        "vision_load_image" => {
            let image_path = args["image_path"]
                .as_str()
                .ok_or("image_path is required")?;
            if !Path::new(image_path).exists() {
                return Err(format!("Image not found: {}", image_path));
            }
            let base64_data = image_to_base64(image_path)?;
            let extension = Path::new(image_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png")
                .to_lowercase();
            let media_type = match extension.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "png" => "image/png",
                "gif" => "image/gif",
                "webp" => "image/webp",
                _ => "image/png",
            };
            Ok(
                json!({ "type": "image", "source": { "type": "base64", "media_type": media_type, "data": base64_data } }),
            )
        }

        "vision_diff" => {
            let image_a_path = args["image_a"].as_str().ok_or("image_a is required")?;
            let image_b_input = args["image_b"].as_str().ok_or("image_b is required")?;
            let threshold = args["threshold"].as_u64().unwrap_or(30) as u8;
            let monitor = args["monitor"].as_u64().unwrap_or(0) as usize;

            let img_a = load_image_rgba(image_a_path)?;
            let (img_b, image_b_path) = if image_b_input == "screen" {
                let temp_path = take_screenshot(None, monitor, 80)?;
                let img = load_image_rgba(&temp_path)?;
                (img, temp_path)
            } else {
                (load_image_rgba(image_b_input)?, image_b_input.to_string())
            };

            let (diff_percent, width, height) = diff_images(&img_a, &img_b, threshold);
            let changed = diff_percent > 0.1;
            if image_b_input == "screen" {
                std::fs::remove_file(&image_b_path).ok();
            }

            Ok(json!({
                "success": true, "changed": changed,
                "diff_percent": (diff_percent * 100.0).round() / 100.0,
                "threshold": threshold, "dimensions": {"width": width, "height": height}
            }))
        }

        "vision_find_template" => {
            let template_path = args["template"].as_str().ok_or("template is required")?;
            let search_in = args["search_in"].as_str().unwrap_or("screen");
            let threshold = args["threshold"].as_f64().unwrap_or(0.8);
            let monitor = args["monitor"].as_u64().unwrap_or(0) as usize;

            let needle = load_image_rgba(template_path)?;
            let (nw, nh) = needle.dimensions();

            let (haystack, haystack_path) = if search_in == "screen" {
                let temp_path = take_screenshot(None, monitor, 80)?;
                let img = load_image_rgba(&temp_path)?;
                (img, temp_path)
            } else {
                (load_image_rgba(search_in)?, search_in.to_string())
            };

            let result = find_template_match(&haystack, &needle, threshold);
            if search_in == "screen" {
                std::fs::remove_file(&haystack_path).ok();
            }

            match result {
                Some((x, y, confidence)) => Ok(json!({
                    "success": true, "found": true,
                    "x": x, "y": y,
                    "center_x": x + nw / 2, "center_y": y + nh / 2,
                    "confidence": (confidence * 100.0).round() / 100.0,
                    "template_size": {"width": nw, "height": nh}
                })),
                None => Ok(json!({ "success": true, "found": false, "threshold": threshold })),
            }
        }

        "vision_ocr_backends" => Ok(vision_ocr_backends()),

        _ => Err(format!("Unknown vision tool: {}", name)),
    }
}

// === FILE NAVIGATION ===
// Generated: 2026-05-31
// 21 functions | 1 enum | onnx submodule: src/paddle_onnx.rs (feature-gated)
//
// IMPORTS: base64, screenshots, serde_json, std, windows
//
// OCR BACKEND:
//   enum OcrBackend: 24
//   selected_backend (env VISION_CORE_OCR_BACKEND): 37
//   pub +ocr_image (dispatcher): 321
//   windows_ocr_image (default path): 333
//   pub +ocr_image_with_positions (dispatcher): 397
//   windows_ocr_image_with_positions (default path): 411
//   pub +vision_ocr_backends (capability reporter): 632
//
// FUNCTIONS:
//   pub +get_local_definitions: 47 [LARGE]
//   pub +get_api_definitions: 171
//   pub +get_all_definitions: 187
//   pub +save_image: 195
//   pub +take_screenshot: 222
//   pub +take_screenshot_region: 255
//   pub +get_screen_dimensions: 301
//   pub +image_to_base64: 311
//   pub +analyze_with_claude: 485 [med]
//   load_image_rgba: 542
//   diff_images: 548
//   find_template_match: 576
//   pub +execute: 672
//   execute_inner: 679 [LARGE]
//
// === END FILE NAVIGATION ===
