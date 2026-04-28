#![allow(dead_code)] // scaffolded module, awaiting integration
//! Vision capture helper — all vision rungs go through this.
//! Enforces image size rules per spec:
//! - Default to cropped captures with ~200px padding over full-screen
//! - Full-screen fallback: pre-downscale to 1568px long edge
//! - Tiling mode for precision scans: 2×2 or 3×3 quadrants at native resolution
//! - Enforce 4.5MB cap (under 5MB API limit), JPEG Q85 default
//! - Cache capture buffer for 2s to allow reuse

use std::time::Instant;

/// Maximum image size for API submission (4.5MB, headroom under 5MB limit).
pub const MAX_IMAGE_BYTES: usize = 4_500_000;

/// Target long edge for downscaling (matches Anthropic analysis resolution).
pub const TARGET_LONG_EDGE: u32 = 1568;

/// Default JPEG quality.
pub const JPEG_QUALITY: u8 = 85;

/// Padding around cropped captures in pixels.
pub const CROP_PADDING: u32 = 200;

/// Cache duration for capture reuse in milliseconds.
pub const CACHE_DURATION_MS: u64 = 2000;

/// Capture mode for vision operations.
#[derive(Debug, Clone)]
pub enum CaptureMode {
    /// Crop to a specific region with padding.
    Cropped {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// Full screen, pre-downscaled to analysis resolution.
    FullScreen,
    /// Tiled: split into grid for precision analysis.
    Tiled { cols: u32, rows: u32 },
    /// Specific window by title.
    Window { title: String },
}

/// Capture result from vision_capture.
#[derive(Debug, Clone)]
pub struct CaptureResult {
    /// The capture mode used.
    pub mode: String,
    /// Image data as base64 JPEG (or multiple for tiled).
    pub images: Vec<CaptureImage>,
    /// Total size in bytes.
    pub total_bytes: usize,
    /// Whether the image was downscaled.
    pub downscaled: bool,
    /// Capture timestamp for cache reuse.
    pub captured_at: Instant,
}

/// Single captured image (one of potentially many tiles).
#[derive(Debug, Clone)]
pub struct CaptureImage {
    /// Base64-encoded image data.
    pub data_base64: String,
    /// Original dimensions before any downscaling.
    pub original_width: u32,
    pub original_height: u32,
    /// Actual dimensions sent.
    pub width: u32,
    pub height: u32,
    /// Region of the screen this covers (for tiled captures).
    pub region: Option<(i32, i32, u32, u32)>,
    /// Monitor index (for multi-monitor captures).
    pub monitor_index: Option<i32>,
}

/// Cached capture for reuse within CACHE_DURATION_MS.
pub struct CaptureCache {
    last_result: Option<CaptureResult>,
    last_mode_key: String,
}

impl CaptureCache {
    pub fn new() -> Self {
        Self {
            last_result: None,
            last_mode_key: String::new(),
        }
    }

    /// Check if a cached capture is still valid for the given mode.
    pub fn get(&self, mode_key: &str) -> Option<&CaptureResult> {
        if let Some(ref result) = self.last_result {
            if self.last_mode_key == mode_key
                && result.captured_at.elapsed().as_millis() < CACHE_DURATION_MS as u128
            {
                return Some(result);
            }
        }
        None
    }

    /// Store a capture result.
    pub fn store(&mut self, mode_key: String, result: CaptureResult) {
        self.last_mode_key = mode_key;
        self.last_result = Some(result);
    }

    /// Clear the cache.
    pub fn clear(&mut self) {
        self.last_result = None;
        self.last_mode_key.clear();
    }
}

/// Compute target dimensions when downscaling to fit TARGET_LONG_EDGE.
pub fn downscale_dimensions(width: u32, height: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= TARGET_LONG_EDGE {
        return (width, height);
    }
    let scale = TARGET_LONG_EDGE as f64 / long as f64;
    let new_w = (width as f64 * scale).round() as u32;
    let new_h = (height as f64 * scale).round() as u32;
    (new_w.max(1), new_h.max(1))
}

/// Compute crop region with padding, clamped to screen bounds.
pub fn compute_crop_region(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    screen_width: u32,
    screen_height: u32,
) -> (i32, i32, u32, u32) {
    let pad = CROP_PADDING as i32;
    let x1 = (x - pad).max(0);
    let y1 = (y - pad).max(0);
    let x2 = ((x + width as i32 + pad) as u32).min(screen_width);
    let y2 = ((y + height as i32 + pad) as u32).min(screen_height);
    (x1, y1, (x2 as i32 - x1) as u32, (y2 as i32 - y1) as u32)
}

/// Compute tile grid for a screen resolution.
pub fn compute_tile_grid(
    screen_width: u32,
    screen_height: u32,
    cols: u32,
    rows: u32,
) -> Vec<(i32, i32, u32, u32)> {
    let tile_w = screen_width / cols;
    let tile_h = screen_height / rows;
    let mut tiles = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let x = col * tile_w;
            let y = row * tile_h;
            // Last column/row absorbs remainder pixels
            let w = if col == cols - 1 {
                screen_width - x
            } else {
                tile_w
            };
            let h = if row == rows - 1 {
                screen_height - y
            } else {
                tile_h
            };
            tiles.push((x as i32, y as i32, w, h));
        }
    }
    tiles
}

/// Generate a cache key for a capture mode.
pub fn mode_cache_key(mode: &CaptureMode) -> String {
    match mode {
        CaptureMode::Cropped {
            x,
            y,
            width,
            height,
        } => {
            format!("crop_{}_{}_{}_{}", x, y, width, height)
        }
        CaptureMode::FullScreen => "fullscreen".to_string(),
        CaptureMode::Tiled { cols, rows } => format!("tiled_{}x{}", cols, rows),
        CaptureMode::Window { title } => format!("window_{}", title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downscale_small_image() {
        // Image already smaller — no change
        assert_eq!(downscale_dimensions(800, 600), (800, 600));
    }

    #[test]
    fn test_downscale_4k() {
        let (w, h) = downscale_dimensions(3840, 2160);
        assert!(w <= TARGET_LONG_EDGE);
        assert!(h <= TARGET_LONG_EDGE);
        // Aspect ratio preserved
        let ratio_orig = 3840.0 / 2160.0;
        let ratio_new = w as f64 / h as f64;
        assert!((ratio_orig - ratio_new).abs() < 0.05);
    }

    #[test]
    fn test_crop_with_padding() {
        let (x, y, w, h) = compute_crop_region(100, 100, 200, 50, 1920, 1080);
        assert!(x < 100); // padding applied
        assert!(y < 100);
        assert!(w > 200);
        assert!(h > 50);
    }

    #[test]
    fn test_crop_clamped_to_screen() {
        // Near edge — padding clamped
        let (x, y, _w, _h) = compute_crop_region(0, 0, 100, 100, 1920, 1080);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn test_tile_grid_2x2() {
        let tiles = compute_tile_grid(1920, 1080, 2, 2);
        assert_eq!(tiles.len(), 4);
        // All tiles should cover the full screen
        let total_area: u32 = tiles.iter().map(|(_, _, w, h)| w * h).sum();
        assert_eq!(total_area, 1920 * 1080);
    }

    #[test]
    fn test_tile_grid_3x3() {
        let tiles = compute_tile_grid(1920, 1080, 3, 3);
        assert_eq!(tiles.len(), 9);
    }
}
