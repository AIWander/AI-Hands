use serde_json::{json, Value};
use std::sync::OnceLock;

pub use vision_core_lib::{ocr_image_with_positions, save_image};

static VISION_EXECUTION: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn execution_lock() -> &'static tokio::sync::Mutex<()> {
    VISION_EXECUTION.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub fn get_all_definitions() -> Vec<Value> {
    let mut definitions = vision_core_lib::get_all_definitions();
    for definition in &mut definitions {
        let name = definition.get("name").and_then(Value::as_str).unwrap_or("");
        if matches!(name, "vision_screenshot" | "vision_screenshot_ocr") {
            if let Some(object) = definition.as_object_mut() {
                let description = object
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                object.insert(
                    "description".to_string(),
                    Value::String(format!(
                        "{description} Automatic paths are collision-safe across concurrent calls. Active hands_monitor_scope bindings override the monitor and fail closed on topology drift."
                    )),
                );
            }
        }
    }
    definitions
}

pub async fn execute(name: &str, args: &Value) -> Value {
    let mut prepared = match crate::monitor_scope::prepare_vision_args(args) {
        Ok(value) => value,
        Err(error) => return error,
    };

    let _serial = execution_lock().lock().await;
    match name {
        "vision_screenshot" => execute_screenshot(&mut prepared).await,
        "vision_screenshot_ocr" => execute_screenshot_ocr(&mut prepared).await,
        "vision_zoom" => execute_zoom(&prepared).await,
        _ => vision_core_lib::execute(name, &prepared).await,
    }
}

fn requested_path(args: &Value, key: &str) -> Result<Option<String>, Value> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(path)) if path.trim().is_empty() => Ok(None),
        Some(Value::String(path)) => Ok(Some(path.clone())),
        Some(_) => Err(json!({
            "success": false,
            "error": format!("{key} must be a non-empty string path or null")
        })),
    }
}

fn capture_selected(
    save_path: &str,
    monitor: usize,
    region: Option<&Value>,
    quality: u8,
) -> Result<(String, crate::monitor_scope::MonitorDescriptor, u32, u32), String> {
    let (screen, descriptor) = crate::monitor_scope::selected_screen(monitor)?;
    let full = screen
        .capture()
        .map_err(|error| format!("Screenshot failed: {error}"))?;
    let image = if let Some(region) = region.filter(|value| value.is_object()) {
        let x = region.get("x").and_then(Value::as_i64).unwrap_or(0).max(0) as u32;
        let y = region.get("y").and_then(Value::as_i64).unwrap_or(0).max(0) as u32;
        if x >= full.width() || y >= full.height() {
            return Err("Capture region starts outside the selected monitor".to_string());
        }
        let width = region
            .get("width")
            .and_then(Value::as_u64)
            .unwrap_or(400)
            .max(1) as u32;
        let height = region
            .get("height")
            .and_then(Value::as_u64)
            .unwrap_or(200)
            .max(1) as u32;
        image::imageops::crop_imm(
            &full,
            x,
            y,
            width.min(full.width() - x),
            height.min(full.height() - y),
        )
        .to_image()
    } else {
        full
    };
    crate::monitor_scope::validate_monitor_snapshot(&descriptor)?;
    let capture_width = image.width();
    let capture_height = image.height();
    vision_core_lib::save_image(&image, save_path, quality)?;
    Ok((
        save_path.to_string(),
        descriptor,
        capture_width,
        capture_height,
    ))
}

async fn execute_screenshot(args: &mut Value) -> Value {
    let path = match requested_path(args, "save_path") {
        Ok(Some(path)) => path,
        Ok(None) => crate::monitor_scope::unique_capture_path("screenshot", "png", true),
        Err(error) => return error,
    };
    let monitor = args.get("monitor").and_then(Value::as_u64).unwrap_or(0) as usize;
    let quality = args.get("quality").and_then(Value::as_u64).unwrap_or(80) as u8;
    let (path, descriptor, width, height) =
        match capture_selected(&path, monitor, args.get("region"), quality) {
            Ok(capture) => capture,
            Err(error) => return json!({"success": false, "error": error}),
        };
    let mut result = json!({
        "success": true,
        "path": path,
        "monitor": descriptor.index,
        "display_id": descriptor.display_id,
        "stable_id": descriptor.stable_id,
        "capture_width": width,
        "capture_height": height
    });
    if args
        .get("return_base64")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        match vision_core_lib::image_to_base64(
            result
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ) {
            Ok(encoded) => result["base64"] = json!(encoded),
            Err(error) => return json!({"success": false, "error": error}),
        }
    }
    result
}

async fn execute_screenshot_ocr(args: &mut Value) -> Value {
    let explicit_path = match requested_path(args, "save_screenshot") {
        Ok(path) => path,
        Err(error) => return error,
    };
    let path = explicit_path.clone().unwrap_or_else(|| {
        crate::monitor_scope::unique_capture_path("screenshot_ocr", "png", false)
    });
    let monitor = args.get("monitor").and_then(Value::as_u64).unwrap_or(0) as usize;
    let (path, descriptor, width, height) =
        match capture_selected(&path, monitor, args.get("region"), 80) {
            Ok(capture) => capture,
            Err(error) => return json!({"success": false, "error": error}),
        };
    let text = match vision_core_lib::ocr_image(&path, "en-US").await {
        Ok(text) => text,
        Err(error) => {
            if explicit_path.is_none() {
                let _ = std::fs::remove_file(&path);
            }
            return json!({"success": false, "error": error});
        }
    };
    if explicit_path.is_none() {
        let _ = std::fs::remove_file(&path);
    }
    let chars = text.len();
    json!({
        "success": true,
        "text": text,
        "chars": chars,
        "screenshot_saved": explicit_path.is_some(),
        "monitor": descriptor.index,
        "display_id": descriptor.display_id,
        "stable_id": descriptor.stable_id,
        "capture_width": width,
        "capture_height": height
    })
}

async fn execute_zoom(args: &Value) -> Value {
    let Some(x) = args.get("x").and_then(Value::as_i64) else {
        return json!({"error": "x is required"});
    };
    let Some(y) = args.get("y").and_then(Value::as_i64) else {
        return json!({"error": "y is required"});
    };
    let Some(width) = args.get("width").and_then(Value::as_u64) else {
        return json!({"error": "width is required"});
    };
    let Some(height) = args.get("height").and_then(Value::as_u64) else {
        return json!({"error": "height is required"});
    };
    let scale = args.get("scale").and_then(Value::as_f64).unwrap_or(2.0);
    if width == 0 || height == 0 {
        return json!({"error": "width and height must be > 0"});
    }
    if !(0.0..=10.0).contains(&scale) || scale == 0.0 {
        return json!({"error": "scale must be between 0 and 10"});
    }
    let monitor = args.get("monitor").and_then(Value::as_u64).unwrap_or(0) as usize;
    let temp = match take_screenshot_region(
        None,
        monitor,
        x as i32,
        y as i32,
        width as u32,
        height as u32,
        100,
    ) {
        Ok(path) => path,
        Err(error) => return json!({"error": error}),
    };
    let image = match image::open(&temp) {
        Ok(image) => image,
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            return json!({"error": format!("Failed to open cropped image: {error}")});
        }
    };
    let _ = std::fs::remove_file(&temp);
    let output_width = (width as f64 * scale) as u32;
    let output_height = (height as f64 * scale) as u32;
    let scaled = image::imageops::resize(
        &image,
        output_width,
        output_height,
        image::imageops::FilterType::Lanczos3,
    );
    let output = crate::monitor_scope::unique_capture_path("zoom", "png", true);
    let rgba = image::DynamicImage::ImageRgba8(scaled).to_rgba8();
    if let Err(error) = vision_core_lib::save_image(&rgba, &output, 100) {
        return json!({"error": format!("Failed to save zoom image: {error}")});
    }
    json!({
        "success": true,
        "path": output,
        "monitor": monitor,
        "region": {"x": x, "y": y, "width": width, "height": height},
        "scale": scale,
        "output_width": output_width,
        "output_height": output_height
    })
}

pub fn take_screenshot(
    save_path: Option<&str>,
    monitor: usize,
    quality: u8,
) -> Result<String, String> {
    let generated;
    let path = match save_path {
        Some(path) => path,
        None => {
            generated = crate::monitor_scope::unique_capture_path("screenshot_tmp", "png", false);
            &generated
        }
    };
    capture_selected(path, monitor, None, quality).map(|capture| capture.0)
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
    let generated;
    let path = match save_path {
        Some(path) => path,
        None => {
            generated = crate::monitor_scope::unique_capture_path("region_tmp", "png", false);
            &generated
        }
    };
    let region = json!({"x": x, "y": y, "width": width, "height": height});
    capture_selected(path, monitor, Some(&region), quality).map(|capture| capture.0)
}
