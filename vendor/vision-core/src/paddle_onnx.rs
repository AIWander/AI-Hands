//! Experimental PaddleOCR (PP-OCRv4) backend via ONNX Runtime.
//!
//! ENTIRELY behind the `onnx` Cargo feature (the `mod paddle_onnx;` declaration
//! in lib.rs is `#[cfg(feature = "onnx")]`). The default vision-core build does
//! NOT compile this file and pulls in neither `ort` nor `ndarray`.
//!
//! Status: EXPERIMENTAL / UNVERIFIED. The two-stage pipeline (DBNet text
//! detection -> CRNN/SVTR text recognition + CTC greedy decode) is implemented
//! to PP-OCRv4 standard conventions, but it has NOT been runtime-validated
//! against real model files. Tensor I/O names and a few shape conventions vary
//! by how the model was exported (paddle2onnx flags, PaddleOCR version); those
//! spots are marked `// ASSUMPTION (PP-OCRv4 default): ...`.
//!
//! ## Configuration (all via env)
//! - `VISION_CORE_PADDLE_DET_MODEL` — path to the detection `.onnx` (DBNet).
//! - `VISION_CORE_PADDLE_REC_MODEL` — path to the recognition `.onnx` (CRNN/SVTR).
//! - `VISION_CORE_PADDLE_DICT`      — path to the character dictionary `.txt`
//!   (one token per line, in the order the rec model emits class indices).
//!
//! Enable at runtime with `VISION_CORE_OCR_BACKEND=paddle`. If any of the three
//! paths is unset or missing, every entry point returns a clear `Err(..)` and
//! never panics.
//!
//! ## ort 2.x API used (for future maintainers)
//! - Session load:  `ort::session::Session::builder()?.commit_from_file(path)?`
//! - Inference:     `session.run(ort::inputs![name => tensor])?`
//! - Input tensor:  `ort::value::Tensor::from_array((shape, vec))?`
//! - Output read:   `outputs[i_or_name].try_extract_array::<f32>()? -> ArrayViewD<f32>`
//!   (`ndarray` is an enabled ort feature, so `try_extract_array` is available.)

use ndarray::{Array, ArrayD, IxDyn};
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

// === PP-OCRv4 constants (standard export defaults) ===

/// Detection input is resized so each side is a multiple of 32 (DBNet stride).
const DET_SIDE_MULTIPLE: u32 = 32;
/// Upper bound on the longest detection side, to cap memory/latency.
const DET_MAX_SIDE: u32 = 960;
/// Recognition models are trained at a fixed input height of 48px (PP-OCRv4).
const REC_IMG_HEIGHT: u32 = 48;
/// Max recognition width before the strip is squeezed (PP-OCRv4 default 320).
const REC_MAX_WIDTH: u32 = 320;
// ASSUMPTION (PP-OCRv4 default): ImageNet mean/std normalization, RGB order.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];
// ASSUMPTION (PP-OCRv4 default): DBNet probability-map binarization threshold
// and the minimum mean confidence for a connected region to be kept.
const DET_BIN_THRESH: f32 = 0.3;
const DET_BOX_THRESH: f32 = 0.5;

// === Session cache ===
//
// Loaded sessions are cached in a process-global map keyed by the model path so
// repeated `ocr_image*` calls don't re-parse the ONNX graph. ort `Session::run`
// takes `&mut self`, so each session lives behind its own `Mutex`.

type SessionCache = OnceLock<Mutex<HashMap<String, &'static Mutex<Session>>>>;
static DET_SESSIONS: SessionCache = OnceLock::new();
static REC_SESSIONS: SessionCache = OnceLock::new();

/// Get (or build + cache) a `Session` for `model_path`.
///
/// The returned `&'static Mutex<Session>` is leaked intentionally: there is at
/// most one session per distinct model path for the life of the process, which
/// is the desired cache behavior and avoids self-referential lifetimes.
fn get_session(cache: &SessionCache, model_path: &str) -> Result<&'static Mutex<Session>, String> {
    let map = cache.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map
        .lock()
        .map_err(|_| "ONNX session cache mutex poisoned".to_string())?;

    if let Some(sess) = guard.get(model_path) {
        return Ok(*sess);
    }

    let session = Session::builder()
        .map_err(|e| format!("ort: failed to create session builder: {e}"))?
        .commit_from_file(model_path)
        .map_err(|e| format!("ort: failed to load ONNX model '{model_path}': {e}"))?;

    // Leak to obtain a 'static handle for the cache (one per model path).
    let leaked: &'static Mutex<Session> = Box::leak(Box::new(Mutex::new(session)));
    guard.insert(model_path.to_string(), leaked);
    Ok(leaked)
}

// === Config resolution ===

struct PaddleConfig {
    det_model: String,
    rec_model: String,
    dict: Vec<String>,
}

/// Read + validate the three env paths. Returns a single clear error listing the
/// missing pieces rather than panicking.
fn resolve_config() -> Result<PaddleConfig, String> {
    let det = std::env::var("VISION_CORE_PADDLE_DET_MODEL")
        .ok()
        .filter(|s| !s.is_empty());
    let rec = std::env::var("VISION_CORE_PADDLE_REC_MODEL")
        .ok()
        .filter(|s| !s.is_empty());
    let dict = std::env::var("VISION_CORE_PADDLE_DICT")
        .ok()
        .filter(|s| !s.is_empty());

    let mut missing: Vec<&str> = Vec::new();
    match &det {
        Some(p) if Path::new(p).exists() => {}
        Some(_) => missing.push("VISION_CORE_PADDLE_DET_MODEL (path does not exist)"),
        None => missing.push("VISION_CORE_PADDLE_DET_MODEL"),
    }
    match &rec {
        Some(p) if Path::new(p).exists() => {}
        Some(_) => missing.push("VISION_CORE_PADDLE_REC_MODEL (path does not exist)"),
        None => missing.push("VISION_CORE_PADDLE_REC_MODEL"),
    }
    match &dict {
        Some(p) if Path::new(p).exists() => {}
        Some(_) => missing.push("VISION_CORE_PADDLE_DICT (path does not exist)"),
        None => missing.push("VISION_CORE_PADDLE_DICT"),
    }

    if !missing.is_empty() {
        return Err(format!(
            "PaddleOCR-ONNX backend is missing model configuration. Set VISION_CORE_PADDLE_DET_MODEL / VISION_CORE_PADDLE_REC_MODEL / VISION_CORE_PADDLE_DICT to valid files. Missing/invalid: {}. See docs/paddle-onnx.md for how to fetch PP-OCRv4 ONNX models.",
            missing.join(", ")
        ));
    }

    let dict_path = dict.unwrap();
    let dict_text = std::fs::read_to_string(&dict_path)
        .map_err(|e| format!("Failed to read PaddleOCR dictionary '{dict_path}': {e}"))?;
    // PP-OCRv4 dicts are one token per line; we keep them verbatim (no trim of
    // internal content) but strip the trailing newline only.
    let dict: Vec<String> = dict_text.lines().map(|l| l.to_string()).collect();
    if dict.is_empty() {
        return Err(format!("PaddleOCR dictionary '{dict_path}' is empty"));
    }

    Ok(PaddleConfig {
        det_model: det.unwrap(),
        rec_model: rec.unwrap(),
        dict,
    })
}

// === Public entry points ===

/// One recognized text region: `(text, x, y, width, height)` in source pixels.
/// Mirrors the public `ocr_image_with_positions` element type in lib.rs.
type DetResult = (String, f64, f64, f64, f64);

/// PP-OCRv4 image -> plain text. Runs detection + recognition and joins the
/// recognized strings (reading order: top-to-bottom by box top edge).
pub async fn ocr_image(image_path: &str) -> Result<String, String> {
    let boxes = run_pipeline(image_path)?;
    let mut ordered = boxes;
    ordered.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let text = ordered
        .into_iter()
        .map(|(t, _, _, _, _)| t)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text)
}

/// PP-OCRv4 image -> per-region `(text, x, y, w, h)` in source-image pixels.
pub async fn ocr_image_with_positions(image_path: &str) -> Result<Vec<DetResult>, String> {
    run_pipeline(image_path)
}

// === Pipeline ===

/// A detected text region in source-image pixel coordinates.
struct DetBox {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// Full 2-stage pipeline: load image -> detect boxes -> crop+recognize each.
fn run_pipeline(image_path: &str) -> Result<Vec<DetResult>, String> {
    if !Path::new(image_path).exists() {
        return Err(format!("Image not found: {image_path}"));
    }
    let config = resolve_config()?;

    let rgb = image::open(image_path)
        .map_err(|e| format!("Failed to open image '{image_path}': {e}"))?
        .to_rgb8();
    let (src_w, src_h) = rgb.dimensions();
    if src_w == 0 || src_h == 0 {
        return Err("Image has zero dimension".to_string());
    }

    // --- Stage 1: detection ---
    let boxes = detect_boxes(&config, &rgb, src_w, src_h)?;

    // --- Stage 2: recognition (per box) ---
    let mut results = Vec::with_capacity(boxes.len());
    for b in boxes {
        // Crop with clamping to image bounds.
        let cx = b.x.min(src_w.saturating_sub(1));
        let cy = b.y.min(src_h.saturating_sub(1));
        let cw = b.w.min(src_w - cx).max(1);
        let ch = b.h.min(src_h - cy).max(1);
        let crop = image::imageops::crop_imm(&rgb, cx, cy, cw, ch).to_image();

        let text = recognize_crop(&config, &crop)?;
        if !text.trim().is_empty() {
            results.push((text, cx as f64, cy as f64, cw as f64, ch as f64));
        }
    }

    Ok(results)
}

/// Stage 1 — DBNet text detection. Returns axis-aligned bounding boxes.
fn detect_boxes(
    config: &PaddleConfig,
    rgb: &image::RgbImage,
    src_w: u32,
    src_h: u32,
) -> Result<Vec<DetBox>, String> {
    // Resize so the longest side <= DET_MAX_SIDE and both sides are multiples of
    // DET_SIDE_MULTIPLE (DBNet requirement). Track scale factors to map the
    // probability map back to source pixels.
    let (net_w, net_h) = det_target_size(src_w, src_h);
    let resized = image::imageops::resize(rgb, net_w, net_h, image::imageops::FilterType::Triangle);

    // NCHW float input, normalized. // ASSUMPTION (PP-OCRv4 default): shape
    // [1, 3, net_h, net_w], RGB channel order, ImageNet mean/std.
    let input = to_nchw_normalized(&resized, net_w, net_h);
    let shape = [1usize, 3, net_h as usize, net_w as usize];
    let flat: Vec<f32> = input.into_raw_vec_and_offset().0;
    let tensor = Tensor::from_array((shape, flat))
        .map_err(|e| format!("ort: failed to build detection input tensor: {e}"))?;

    let det_mutex = get_session(&DET_SESSIONS, &config.det_model)?;
    let mut session = det_mutex
        .lock()
        .map_err(|_| "detection session mutex poisoned".to_string())?;

    // ASSUMPTION (PP-OCRv4 default): single input named "x". paddle2onnx exports
    // PP-OCR det with input name "x" and one output (the probability map).
    let outputs = session
        .run(ort::inputs!["x" => tensor])
        .map_err(|e| format!("ort: detection inference failed: {e}"))?;

    // Output is the segmentation probability map, shape [1, 1, net_h, net_w].
    let prob = first_output_array(&outputs)?;
    let prob_view = prob.view();

    // Map probability map -> binary mask -> connected-component boxes.
    let boxes = boxes_from_prob_map(&prob_view, net_w, net_h, src_w, src_h);
    Ok(boxes)
}

/// Stage 2 — CRNN/SVTR recognition on a single cropped text strip. CTC-greedy
/// decodes the per-timestep class logits against the dictionary.
fn recognize_crop(config: &PaddleConfig, crop: &image::RgbImage) -> Result<String, String> {
    // Resize to fixed height REC_IMG_HEIGHT, preserve aspect ratio, cap width.
    let (cw, ch) = crop.dimensions();
    if cw == 0 || ch == 0 {
        return Ok(String::new());
    }
    let scaled_w = ((cw as f32) * (REC_IMG_HEIGHT as f32) / (ch as f32)).ceil() as u32;
    let target_w = scaled_w.clamp(1, REC_MAX_WIDTH);
    let resized = image::imageops::resize(
        crop,
        target_w,
        REC_IMG_HEIGHT,
        image::imageops::FilterType::Triangle,
    );

    // ASSUMPTION (PP-OCRv4 default): rec input is NCHW [1, 3, 48, W], normalized
    // to [-1, 1] via (x/255 - 0.5) / 0.5 (PP-OCR rec uses 0.5 mean/std, not the
    // ImageNet stats the detector uses).
    let input = to_nchw_rec_normalized(&resized, target_w, REC_IMG_HEIGHT);
    let shape = [1usize, 3, REC_IMG_HEIGHT as usize, target_w as usize];
    let flat: Vec<f32> = input.into_raw_vec_and_offset().0;
    let tensor = Tensor::from_array((shape, flat))
        .map_err(|e| format!("ort: failed to build recognition input tensor: {e}"))?;

    let rec_mutex = get_session(&REC_SESSIONS, &config.rec_model)?;
    let mut session = rec_mutex
        .lock()
        .map_err(|_| "recognition session mutex poisoned".to_string())?;

    // ASSUMPTION (PP-OCRv4 default): single input named "x".
    let outputs = session
        .run(ort::inputs!["x" => tensor])
        .map_err(|e| format!("ort: recognition inference failed: {e}"))?;

    // Output logits/probs, shape [1, T, num_classes].
    let logits = first_output_array(&outputs)?;
    let text = ctc_greedy_decode(&logits.view(), &config.dict);
    Ok(text)
}

// === Preprocessing helpers ===

/// Compute DBNet input size: clamp longest side to DET_MAX_SIDE, round both
/// sides up to a multiple of DET_SIDE_MULTIPLE (min one multiple).
fn det_target_size(src_w: u32, src_h: u32) -> (u32, u32) {
    let longest = src_w.max(src_h) as f32;
    let scale = if longest > DET_MAX_SIDE as f32 {
        DET_MAX_SIDE as f32 / longest
    } else {
        1.0
    };
    let round = |v: f32| -> u32 {
        let r = ((v * scale).round() as u32).max(1);
        let m = DET_SIDE_MULTIPLE;
        (r.div_ceil(m) * m).max(m)
    };
    (round(src_w as f32), round(src_h as f32))
}

/// RGB image -> NCHW f32 array normalized with ImageNet mean/std (detector).
fn to_nchw_normalized(img: &image::RgbImage, w: u32, h: u32) -> ArrayD<f32> {
    let mut arr = Array::zeros(IxDyn(&[1, 3, h as usize, w as usize]));
    for (x, y, px) in img.enumerate_pixels() {
        for c in 0..3 {
            let v = px[c] as f32 / 255.0;
            arr[[0, c, y as usize, x as usize]] = (v - MEAN[c]) / STD[c];
        }
    }
    arr
}

/// RGB image -> NCHW f32 array normalized to [-1, 1] (PP-OCR recognizer).
fn to_nchw_rec_normalized(img: &image::RgbImage, w: u32, h: u32) -> ArrayD<f32> {
    let mut arr = Array::zeros(IxDyn(&[1, 3, h as usize, w as usize]));
    for (x, y, px) in img.enumerate_pixels() {
        for c in 0..3 {
            let v = px[c] as f32 / 255.0;
            // (v - 0.5) / 0.5
            arr[[0, c, y as usize, x as usize]] = (v - 0.5) / 0.5;
        }
    }
    arr
}

// === Postprocessing helpers ===

/// Extract the model's first output as an owned dynamic f32 array.
fn first_output_array(outputs: &ort::session::SessionOutputs) -> Result<ArrayD<f32>, String> {
    // SessionOutputs is index- and name-addressable; index 0 is the first
    // graph output, which is what PP-OCR det/rec emit as their primary tensor.
    let (_name, value) = outputs
        .iter()
        .next()
        .ok_or_else(|| "ort: model produced no outputs".to_string())?;
    let view = value
        .try_extract_array::<f32>()
        .map_err(|e| format!("ort: failed to extract f32 output tensor: {e}"))?;
    Ok(view.to_owned())
}

/// Connected-component box extraction from a DBNet probability map.
///
/// Simplified relative to PaddleOCR's polygon/unclip postprocess: binarize at
/// DET_BIN_THRESH, label 4-connected components, keep those whose mean
/// probability exceeds DET_BOX_THRESH, and emit each component's axis-aligned
/// bounding box scaled from net space back to source pixels.
fn boxes_from_prob_map(
    prob: &ndarray::ArrayViewD<f32>,
    net_w: u32,
    net_h: u32,
    src_w: u32,
    src_h: u32,
) -> Vec<DetBox> {
    let nh = net_h as usize;
    let nw = net_w as usize;

    // Flatten to a (h, w) accessor regardless of leading singleton dims.
    // Expected prob shape: [1, 1, nh, nw]; fall back gracefully on [nh, nw].
    let at = |y: usize, x: usize| -> f32 {
        match prob.ndim() {
            4 => prob[[0, 0, y, x]],
            3 => prob[[0, y, x]],
            2 => prob[[y, x]],
            _ => 0.0,
        }
    };

    let mut visited = vec![false; nh * nw];
    let mut boxes: Vec<DetBox> = Vec::new();
    let sx = src_w as f32 / net_w as f32;
    let sy = src_h as f32 / net_h as f32;

    for sy0 in 0..nh {
        for sx0 in 0..nw {
            let idx = sy0 * nw + sx0;
            if visited[idx] || at(sy0, sx0) < DET_BIN_THRESH {
                continue;
            }
            // BFS flood fill over the binary mask.
            let mut stack = vec![(sy0, sx0)];
            let (mut min_x, mut min_y, mut max_x, mut max_y) = (sx0, sy0, sx0, sy0);
            let mut sum = 0.0f32;
            let mut count = 0u32;
            while let Some((cy, cx)) = stack.pop() {
                let cidx = cy * nw + cx;
                if visited[cidx] {
                    continue;
                }
                let p = at(cy, cx);
                if p < DET_BIN_THRESH {
                    continue;
                }
                visited[cidx] = true;
                sum += p;
                count += 1;
                min_x = min_x.min(cx);
                min_y = min_y.min(cy);
                max_x = max_x.max(cx);
                max_y = max_y.max(cy);
                if cx > 0 {
                    stack.push((cy, cx - 1));
                }
                if cx + 1 < nw {
                    stack.push((cy, cx + 1));
                }
                if cy > 0 {
                    stack.push((cy - 1, cx));
                }
                if cy + 1 < nh {
                    stack.push((cy + 1, cx));
                }
            }

            if count == 0 {
                continue;
            }
            let mean = sum / count as f32;
            // Drop tiny specks and low-confidence regions.
            if mean < DET_BOX_THRESH || (max_x - min_x) < 2 || (max_y - min_y) < 2 {
                continue;
            }

            // Scale net-space box -> source pixels, with a small pad (PaddleOCR
            // "unclip" approximation) of 1px each side, clamped to bounds.
            let x0 = ((min_x as f32) * sx).floor().max(0.0) as u32;
            let y0 = ((min_y as f32) * sy).floor().max(0.0) as u32;
            let x1 = (((max_x + 1) as f32) * sx).ceil().min(src_w as f32) as u32;
            let y1 = (((max_y + 1) as f32) * sy).ceil().min(src_h as f32) as u32;
            let x0 = x0.saturating_sub(1);
            let y0 = y0.saturating_sub(1);
            let x1 = (x1 + 1).min(src_w);
            let y1 = (y1 + 1).min(src_h);

            boxes.push(DetBox {
                x: x0,
                y: y0,
                w: x1.saturating_sub(x0).max(1),
                h: y1.saturating_sub(y0).max(1),
            });
        }
    }

    boxes
}

/// CTC greedy decode of `[1, T, C]` (or `[T, C]`) logits/probs.
///
/// PP-OCR convention: class index 0 is the CTC "blank". The dictionary maps
/// emitted indices to characters; with the standard export, dict line `i`
/// corresponds to class index `i + 1` (blank shifts everything by one). We
/// collapse runs of identical argmax indices and drop blanks.
fn ctc_greedy_decode(logits: &ndarray::ArrayViewD<f32>, dict: &[String]) -> String {
    // Resolve (T, C) regardless of a leading batch dim, then index uniformly.
    // 3D -> [0, t, c]; 2D -> [t, c]; anything else -> empty.
    let (t_len, c_len, three_d) = match logits.ndim() {
        3 => (logits.shape()[1], logits.shape()[2], true),
        2 => (logits.shape()[0], logits.shape()[1], false),
        _ => return String::new(),
    };
    if t_len == 0 || c_len == 0 {
        return String::new();
    }
    let get = |t: usize, c: usize| -> f32 {
        if three_d {
            logits[[0, t, c]]
        } else {
            logits[[t, c]]
        }
    };

    let mut out = String::new();
    let mut prev_idx: isize = -1;
    for t in 0..t_len {
        // argmax over classes at timestep t.
        let mut best_c = 0usize;
        let mut best_v = f32::NEG_INFINITY;
        for c in 0..c_len {
            let v = get(t, c);
            if v > best_v {
                best_v = v;
                best_c = c;
            }
        }
        // Collapse repeats, skip blank (index 0).
        if best_c != 0 && best_c as isize != prev_idx {
            // ASSUMPTION (PP-OCRv4 default): blank at index 0, so the character
            // for class `best_c` is dict[best_c - 1].
            if let Some(ch) = dict.get(best_c - 1) {
                out.push_str(ch);
            }
        }
        prev_idx = best_c as isize;
    }
    out
}
