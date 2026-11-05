//! Tailnet checks, click mapping, and input policy for live screen.
//!
//! These functions do not read the desktop or the network. Windows capture,
//! injection, and the on-screen banner sit behind `cfg(windows)`.

use serde_json::Value;
use std::net::IpAddr;

pub const DEFAULT_PORT: u16 = 8744;
pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MAX_SERVED_WIDTH: u32 = 1280;
pub const JPEG_QUALITY: u8 = 65;
pub const VIEWER_WINDOW_SECS: u64 = 5;
pub const ACTION_HEADER: &str = "x-hands-live-action";
pub const ACTION_HEADER_VALUE: &str = "1";
pub const MAX_TYPED_CHARS: usize = 512;
pub const MAX_SCROLL_NOTCHES: i32 = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TailscaleEndpoint {
    pub ipv4: IpAddr,
    pub magic_dns: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Page,
    Frame,
    Action,
}

#[derive(Clone, Debug)]
pub struct GateRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub client_ip: Option<IpAddr>,
    pub host: Option<&'a str>,
    pub origin: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub action_header: Option<&'a str>,
    pub body_len: usize,
    pub content_length: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
pub struct GateSession<'a> {
    pub token: &'a str,
    pub bind_ip: IpAddr,
    pub magic_dns: Option<&'a str>,
    pub port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Allow(Route),
    Reject(u16),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorGeometry {
    pub logical_x: i32,
    pub logical_y: i32,
    pub logical_w: u32,
    pub logical_h: u32,
    pub capture_w: u32,
    pub capture_h: u32,
    pub scale: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedMonitor {
    pub geometry: MonitorGeometry,
    pub image_x: i32,
    pub image_y: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopMap {
    pub monitors: Vec<PlacedMonitor>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OwnerAction {
    Click {
        x: f64,
        y: f64,
        image_width: u32,
        image_height: u32,
        button: Button,
        double: bool,
    },
    Scroll {
        x: f64,
        y: f64,
        image_width: u32,
        image_height: u32,
        dx: i32,
        dy: i32,
    },
    Type {
        text: String,
    },
    Key {
        name: &'static str,
    },
    Combo {
        combo: Combo,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionFault {
    BadInput,
    Blocked,
}

impl ActionFault {
    pub fn status(self) -> u16 {
        match self {
            Self::BadInput => 400,
            Self::Blocked => 409,
        }
    }
}

/// Tailscale's IPv4 range is 100.64.0.0/10. Public, loopback, and unspecified
/// addresses are rejected, including 0.0.0.0.
pub fn is_tailnet_ipv4(ip: IpAddr) -> bool {
    let v4 = match ip {
        IpAddr::V4(v4) => v4,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4,
            None => return false,
        },
    };
    let [first, second, _, _] = v4.octets();
    first == 100 && (64..128).contains(&second)
}

pub fn parse_tailscale_endpoint(text: &str) -> Result<TailscaleEndpoint, &'static str> {
    let value: Value = serde_json::from_str(text).map_err(|_| "tailscale status was not json")?;
    let backend = value
        .get("BackendState")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !backend.eq_ignore_ascii_case("Running") {
        return Err("tailscale is not logged in");
    }
    let ips = value
        .pointer("/Self/TailscaleIPs")
        .and_then(Value::as_array)
        .ok_or("tailscale status has no address list")?;
    let mut ipv4 = None;
    for item in ips {
        let Some(text) = item.as_str() else {
            continue;
        };
        let Ok(ip) = text.trim().parse::<IpAddr>() else {
            continue;
        };
        let candidate = match ip {
            IpAddr::V4(v4) => IpAddr::V4(v4),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => continue,
            },
        };
        if is_tailnet_ipv4(candidate) {
            ipv4 = Some(candidate);
            break;
        }
    }
    let ipv4 = ipv4.ok_or("no tailscale ipv4 in 100.64.0.0/10")?;
    let magic_dns = value
        .pointer("/Self/DNSName")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(|name| name.trim_end_matches('.'))
        .filter(|name| valid_magic_dns(name))
        .map(str::to_string);
    Ok(TailscaleEndpoint { ipv4, magic_dns })
}

fn valid_magic_dns(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && !name.parse::<IpAddr>().is_ok()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("..")
}

pub fn host_allowed(header: &str, ip: IpAddr, magic_dns: Option<&str>, port: u16) -> bool {
    let Some((name, host_port)) = split_host(header) else {
        return false;
    };
    if host_port.is_some_and(|value| value != port) {
        return false;
    }
    if name.eq_ignore_ascii_case(&ip.to_string()) {
        return true;
    }
    magic_dns.is_some_and(|dns| name.eq_ignore_ascii_case(dns.trim_end_matches('.')))
}

fn split_host(header: &str) -> Option<(&str, Option<u16>)> {
    let header = header.trim();
    if header.is_empty() || header.contains('/') || header.contains(' ') || header.contains('@') {
        return None;
    }
    if let Some((name, port_text)) = header.rsplit_once(':') {
        if name.is_empty() || name.contains(':') || !port_text.chars().all(|ch| ch.is_ascii_digit())
        {
            return None;
        }
        let port = port_text.parse::<u16>().ok()?;
        if port == 0 {
            return None;
        }
        return Some((name, Some(port)));
    }
    Some((header, None))
}

pub fn authorize(request: &GateRequest<'_>, session: GateSession<'_>) -> Gate {
    let advertised = request.content_length.unwrap_or(0);
    if advertised > MAX_BODY_BYTES || request.body_len > MAX_BODY_BYTES {
        return Gate::Reject(413);
    }
    if !request.client_ip.is_some_and(is_tailnet_ipv4) {
        return Gate::Reject(403);
    }
    let host = match request.host.map(str::trim).filter(|host| !host.is_empty()) {
        Some(host) => host,
        None => return Gate::Reject(403),
    };
    if !host_allowed(host, session.bind_ip, session.magic_dns, session.port) {
        return Gate::Reject(403);
    }
    let Some((token, route)) = route_from_path(request.path) else {
        return Gate::Reject(403);
    };
    if !tokens_equal(token, session.token) {
        return Gate::Reject(403);
    }
    let Some(route) = route else {
        return Gate::Reject(400);
    };
    let method = request.method.trim();
    match route {
        Route::Page | Route::Frame if !method.eq_ignore_ascii_case("GET") => Gate::Reject(400),
        Route::Action if !method.eq_ignore_ascii_case("POST") => Gate::Reject(400),
        Route::Action => authorize_action(request, host),
        Route::Page | Route::Frame => Gate::Allow(route),
    }
}

fn authorize_action(request: &GateRequest<'_>, host: &str) -> Gate {
    if request.action_header.map(str::trim) != Some(ACTION_HEADER_VALUE) {
        return Gate::Reject(403);
    }
    let Some(origin) = request
        .origin
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
    else {
        return Gate::Reject(403);
    };
    let expected = format!("http://{host}");
    if !origin.eq_ignore_ascii_case(&expected) {
        return Gate::Reject(403);
    }
    if !content_type_is_json(request.content_type) {
        return Gate::Reject(400);
    }
    Gate::Allow(Route::Action)
}

fn content_type_is_json(value: Option<&str>) -> bool {
    let Some(value) = value.map(str::trim) else {
        return false;
    };
    let media = value.split(';').next().unwrap_or(value).trim();
    media.eq_ignore_ascii_case("application/json")
}

fn route_from_path(path: &str) -> Option<(&str, Option<Route>)> {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    if path.contains('%') || path.contains('\\') || path.contains("..") || path.contains(' ') {
        return None;
    }
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.split('/');
    let token = parts.next().unwrap_or("");
    let second = parts.next();
    if parts.next().is_some()
        || token.len() != 64
        || !token.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return None;
    }
    let route = match second {
        None => Some(Route::Page),
        Some("frame") => Some(Route::Frame),
        Some("action") => Some(Route::Action),
        Some(_) => None,
    };
    Some((token, route))
}

pub fn tokens_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.bytes().zip(right.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

pub fn parse_viewport_width(query: &str) -> Result<u32, ()> {
    if query.is_empty() {
        return Ok(MAX_SERVED_WIDTH);
    }
    let mut width = None;
    for part in query.split('&') {
        if part.is_empty() {
            continue;
        }
        let Some((key, value)) = part.split_once('=') else {
            return Err(());
        };
        if key != "w" {
            continue;
        }
        let parsed = value.parse::<u32>().map_err(|_| ())?;
        if parsed == 0 {
            return Err(());
        }
        width = Some(parsed.min(MAX_SERVED_WIDTH));
    }
    Ok(width.unwrap_or(MAX_SERVED_WIDTH))
}

pub fn served_size(src_w: u32, src_h: u32, viewport: u32) -> Option<(u32, u32)> {
    if src_w == 0 || src_h == 0 {
        return None;
    }
    let cap = viewport.clamp(1, MAX_SERVED_WIDTH);
    if src_w <= cap {
        return Some((src_w, src_h));
    }
    let height = ((src_h as f64) * (cap as f64) / (src_w as f64))
        .round()
        .max(1.0) as u32;
    Some((cap, height.max(1)))
}

pub fn place_monitors(monitors: &[MonitorGeometry]) -> DesktopMap {
    let mut drafts: Vec<(MonitorGeometry, i32, i32)> = monitors
        .iter()
        .filter(|monitor| monitor.capture_w > 0 && monitor.capture_h > 0)
        .filter(|monitor| monitor.logical_w > 0 && monitor.logical_h > 0)
        .cloned()
        .map(|monitor| {
            let (x, y) = image_origin(&monitor);
            (monitor, x, y)
        })
        .collect();
    separate_overlaps(&mut drafts);
    let min_x = drafts.iter().map(|(_, x, _)| *x).min().unwrap_or(0);
    let min_y = drafts.iter().map(|(_, _, y)| *y).min().unwrap_or(0);
    let placed = drafts
        .into_iter()
        .map(|(geometry, x, y)| PlacedMonitor {
            geometry,
            image_x: x - min_x,
            image_y: y - min_y,
        })
        .collect::<Vec<_>>();
    let width = placed
        .iter()
        .map(|monitor| monitor.image_x as u32 + monitor.geometry.capture_w)
        .max()
        .unwrap_or(0);
    let height = placed
        .iter()
        .map(|monitor| monitor.image_y as u32 + monitor.geometry.capture_h)
        .max()
        .unwrap_or(0);
    DesktopMap {
        monitors: placed,
        width,
        height,
    }
}

fn image_origin(monitor: &MonitorGeometry) -> (i32, i32) {
    let same_pixels =
        monitor.capture_w == monitor.logical_w && monitor.capture_h == monitor.logical_h;
    let scale = if monitor.scale.is_finite() && monitor.scale > 0.0 {
        monitor.scale
    } else {
        1.0
    };
    if same_pixels || (scale - 1.0).abs() < 0.01 {
        (monitor.logical_x, monitor.logical_y)
    } else {
        (
            (monitor.logical_x as f64 * scale).round() as i32,
            (monitor.logical_y as f64 * scale).round() as i32,
        )
    }
}

fn separate_overlaps(drafts: &mut [(MonitorGeometry, i32, i32)]) {
    let mut order: Vec<usize> = (0..drafts.len()).collect();
    order.sort_by(|&left, &right| {
        drafts[left]
            .2
            .cmp(&drafts[right].2)
            .then(drafts[left].1.cmp(&drafts[right].1))
            .then(left.cmp(&right))
    });
    for (position, index) in order.iter().copied().enumerate() {
        for _ in 0..drafts.len().saturating_add(1) {
            let mut nudge = None;
            for earlier in order.iter().copied().take(position) {
                if rects_overlap(&drafts[index], &drafts[earlier]) {
                    nudge = Some(drafts[earlier].1 + drafts[earlier].0.capture_w as i32);
                    break;
                }
            }
            let Some(next_x) = nudge else {
                break;
            };
            drafts[index].1 = if next_x == drafts[index].1 {
                next_x.saturating_add(1)
            } else {
                next_x
            };
        }
    }
}

fn rects_overlap(left: &(MonitorGeometry, i32, i32), right: &(MonitorGeometry, i32, i32)) -> bool {
    let (a, ax, ay) = left;
    let (b, bx, by) = right;
    *ax < *bx + b.capture_w as i32
        && *bx < *ax + a.capture_w as i32
        && *ay < *by + b.capture_h as i32
        && *by < *ay + a.capture_h as i32
}

/// Map a point in the served JPEG onto a logical screen pixel.
///
/// `image_x` and `image_y` are continuous pixels in the JPEG the phone is
/// showing. The selected canvas pixel is the one under that point, and the
/// returned coordinate is the logical pixel that contains the centre of the
/// matching physical pixel. Clicks that land in a gap between monitors, or
/// outside the image, return `None`.
pub fn map_click(
    image_x: f64,
    image_y: f64,
    image_w: u32,
    image_h: u32,
    desktop: &DesktopMap,
) -> Option<(i32, i32)> {
    if image_w == 0 || image_h == 0 || desktop.width == 0 || desktop.height == 0 {
        return None;
    }
    if !image_x.is_finite() || !image_y.is_finite() {
        return None;
    }
    if image_x < 0.0 || image_y < 0.0 || image_x >= image_w as f64 || image_y >= image_h as f64 {
        return None;
    }
    let canvas_x = image_x * desktop.width as f64 / image_w as f64;
    let canvas_y = image_y * desktop.height as f64 / image_h as f64;
    if canvas_x < 0.0
        || canvas_y < 0.0
        || canvas_x >= desktop.width as f64
        || canvas_y >= desktop.height as f64
    {
        return None;
    }
    let px = canvas_pixel(canvas_x, desktop.width);
    let py = canvas_pixel(canvas_y, desktop.height);
    let monitor = desktop.monitors.iter().find(|monitor| {
        px >= monitor.image_x
            && py >= monitor.image_y
            && px < monitor.image_x + monitor.geometry.capture_w as i32
            && py < monitor.image_y + monitor.geometry.capture_h as i32
    })?;
    let local_x = (px - monitor.image_x) as f64;
    let local_y = (py - monitor.image_y) as f64;
    let logical_x = monitor.geometry.logical_x as f64
        + (local_x + 0.5) * monitor.geometry.logical_w as f64 / monitor.geometry.capture_w as f64;
    let logical_y = monitor.geometry.logical_y as f64
        + (local_y + 0.5) * monitor.geometry.logical_h as f64 / monitor.geometry.capture_h as f64;
    let x = logical_x.floor() as i32;
    let y = logical_y.floor() as i32;
    let max_x = monitor.geometry.logical_x + monitor.geometry.logical_w as i32 - 1;
    let max_y = monitor.geometry.logical_y + monitor.geometry.logical_h as i32 - 1;
    Some((
        x.clamp(monitor.geometry.logical_x, max_x),
        y.clamp(monitor.geometry.logical_y, max_y),
    ))
}

fn canvas_pixel(position: f64, span: u32) -> i32 {
    let pixel = if position <= 0.0 {
        0.0
    } else {
        (position - 1e-6).floor()
    };
    pixel.clamp(0.0, span.saturating_sub(1) as f64) as i32
}

/// Normalize one axis into the 0..=65535 absolute mouse range, aiming at the
/// centre of `pixel` inside the virtual screen that starts at `origin`.
pub fn absolute_mouse_axis(pixel: i32, origin: i32, span: i32) -> Option<i32> {
    if span <= 0 {
        return None;
    }
    let relative = pixel.checked_sub(origin)?;
    if relative < 0 || relative >= span {
        return None;
    }
    let centre = relative as f64 + 0.5;
    Some(
        ((centre / span as f64) * 65535.0)
            .round()
            .clamp(0.0, 65535.0) as i32,
    )
}

pub fn parse_action(value: &Value) -> Result<OwnerAction, ActionFault> {
    let op = value
        .get("op")
        .and_then(Value::as_str)
        .ok_or(ActionFault::BadInput)?
        .trim();
    match op {
        "click" => parse_click(value),
        "scroll" => parse_scroll(value),
        "type" => parse_type(value),
        "key" => parse_key(value),
        "combo" => parse_combo_action(value),
        _ => Err(ActionFault::BadInput),
    }
}

fn parse_click(value: &Value) -> Result<OwnerAction, ActionFault> {
    let (x, y, image_width, image_height) = image_point(value)?;
    let button = match value
        .get("button")
        .and_then(Value::as_str)
        .unwrap_or("left")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "left" => Button::Left,
        "right" => Button::Right,
        _ => return Err(ActionFault::BadInput),
    };
    let double = value
        .get("double")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(OwnerAction::Click {
        x,
        y,
        image_width,
        image_height,
        button,
        double,
    })
}

fn parse_scroll(value: &Value) -> Result<OwnerAction, ActionFault> {
    let (x, y, image_width, image_height) = image_point(value)?;
    let dx = json_i32(value.get("dx")).ok_or(ActionFault::BadInput)?;
    let dy = json_i32(value.get("dy")).ok_or(ActionFault::BadInput)?;
    if dx == 0 && dy == 0 {
        return Err(ActionFault::BadInput);
    }
    if !(-MAX_SCROLL_NOTCHES..=MAX_SCROLL_NOTCHES).contains(&dx)
        || !(-MAX_SCROLL_NOTCHES..=MAX_SCROLL_NOTCHES).contains(&dy)
    {
        return Err(ActionFault::BadInput);
    }
    Ok(OwnerAction::Scroll {
        x,
        y,
        image_width,
        image_height,
        dx,
        dy,
    })
}

fn image_point(value: &Value) -> Result<(f64, f64, u32, u32), ActionFault> {
    let x = value
        .get("x")
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or(ActionFault::BadInput)?;
    let y = value
        .get("y")
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or(ActionFault::BadInput)?;
    let image_width = json_u32(value.get("image_width")).ok_or(ActionFault::BadInput)?;
    let image_height = json_u32(value.get("image_height")).ok_or(ActionFault::BadInput)?;
    if image_width == 0 || image_height == 0 || image_width > 10_000 || image_height > 10_000 {
        return Err(ActionFault::BadInput);
    }
    if x < 0.0 || y < 0.0 || x >= image_width as f64 || y >= image_height as f64 {
        return Err(ActionFault::BadInput);
    }
    Ok((x, y, image_width, image_height))
}

fn parse_type(value: &Value) -> Result<OwnerAction, ActionFault> {
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .ok_or(ActionFault::BadInput)?;
    let count = text.chars().count();
    if count == 0 || count > MAX_TYPED_CHARS {
        return Err(ActionFault::BadInput);
    }
    if text
        .chars()
        .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    {
        return Err(ActionFault::BadInput);
    }
    Ok(OwnerAction::Type {
        text: text.to_string(),
    })
}

fn parse_key(value: &Value) -> Result<OwnerAction, ActionFault> {
    let name = value
        .get("key")
        .and_then(Value::as_str)
        .ok_or(ActionFault::BadInput)?
        .trim()
        .to_ascii_lowercase();
    let canonical = match name.as_str() {
        "enter" | "return" => "enter",
        "tab" => "tab",
        "esc" | "escape" => "esc",
        "backspace" | "bksp" => "backspace",
        "ctrl+l" | "control+l" => "ctrl+l",
        _ => return Err(ActionFault::BadInput),
    };
    Ok(OwnerAction::Key { name: canonical })
}

fn parse_combo_action(value: &Value) -> Result<OwnerAction, ActionFault> {
    let raw = value
        .get("keys")
        .and_then(Value::as_str)
        .ok_or(ActionFault::BadInput)?;
    let combo = parse_combo(raw).map_err(|_| ActionFault::BadInput)?;
    if combo_is_blocked(&combo) {
        return Err(ActionFault::Blocked);
    }
    Ok(OwnerAction::Combo { combo })
}

pub fn parse_combo(raw: &str) -> Result<Combo, ()> {
    let parts = raw
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 5 {
        return Err(());
    }
    let mut combo = Combo {
        ctrl: false,
        alt: false,
        shift: false,
        win: false,
        key: String::new(),
    };
    for part in parts {
        match part.as_str() {
            "ctrl" | "control" => set_modifier(&mut combo.ctrl)?,
            "alt" | "option" | "menu" => set_modifier(&mut combo.alt)?,
            "shift" => set_modifier(&mut combo.shift)?,
            "win" | "lwin" | "rwin" | "meta" | "super" | "cmd" => set_modifier(&mut combo.win)?,
            other => {
                if !combo.key.is_empty() || !known_key(other) {
                    return Err(());
                }
                combo.key = normalize_key(other).to_string();
            }
        }
    }
    if combo.key.is_empty() {
        return Err(());
    }
    Ok(combo)
}

fn set_modifier(slot: &mut bool) -> Result<(), ()> {
    if *slot {
        return Err(());
    }
    *slot = true;
    Ok(())
}

fn known_key(name: &str) -> bool {
    normalize_key(name) != "unknown"
}

fn normalize_key(name: &str) -> &'static str {
    match name {
        "enter" | "return" => "enter",
        "tab" => "tab",
        "esc" | "escape" => "esc",
        "backspace" | "bksp" => "backspace",
        "del" | "delete" => "delete",
        "space" => "space",
        "up" => "up",
        "down" => "down",
        "left" => "left",
        "right" => "right",
        "home" => "home",
        "end" => "end",
        "pageup" | "pgup" => "pageup",
        "pagedown" | "pgdn" => "pagedown",
        "pause" | "break" => "pause",
        "insert" | "ins" => "insert",
        "apps" | "menu" => "apps",
        other if other.len() == 1 && other.chars().all(|ch| ch.is_ascii_alphanumeric()) => {
            match other {
                "a" => "a",
                "b" => "b",
                "c" => "c",
                "d" => "d",
                "e" => "e",
                "f" => "f",
                "g" => "g",
                "h" => "h",
                "i" => "i",
                "j" => "j",
                "k" => "k",
                "l" => "l",
                "m" => "m",
                "n" => "n",
                "o" => "o",
                "p" => "p",
                "q" => "q",
                "r" => "r",
                "s" => "s",
                "t" => "t",
                "u" => "u",
                "v" => "v",
                "w" => "w",
                "x" => "x",
                "y" => "y",
                "z" => "z",
                "0" => "0",
                "1" => "1",
                "2" => "2",
                "3" => "3",
                "4" => "4",
                "5" => "5",
                "6" => "6",
                "7" => "7",
                "8" => "8",
                "9" => "9",
                _ => "unknown",
            }
        }
        other if function_key(other).is_some() => function_key(other).unwrap_or("unknown"),
        _ => "unknown",
    }
}

fn function_key(name: &str) -> Option<&'static str> {
    const KEYS: [&str; 24] = [
        "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "f13", "f14",
        "f15", "f16", "f17", "f18", "f19", "f20", "f21", "f22", "f23", "f24",
    ];
    KEYS.into_iter().find(|key| *key == name)
}

pub fn combo_is_blocked(combo: &Combo) -> bool {
    let key = combo.key.as_str();
    if combo.win && matches!(key, "l" | "r" | "d" | "m" | "x" | "u" | "pause") {
        return true;
    }
    if combo.alt && !combo.ctrl && !combo.win && key == "f4" {
        return true;
    }
    if combo.ctrl && combo.alt && matches!(key, "delete" | "end") {
        return true;
    }
    if combo.ctrl && combo.shift && !combo.alt && !combo.win && key == "esc" {
        return true;
    }
    false
}

pub fn sanitize_ai_name(raw: &str) -> String {
    let cleaned = raw
        .chars()
        .filter(|ch| !ch.is_control())
        .take(40)
        .collect::<String>();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Assistant".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn banner_text(ai_name: &str, override_text: Option<&str>) -> String {
    if let Some(text) = override_text {
        let decoded = text.replace("\\n", "\n");
        let cleaned = decoded
            .chars()
            .filter(|ch| *ch == '\n' || !ch.is_control())
            .take(400)
            .collect::<String>();
        let trimmed = cleaned.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    format!(
        "{} is taking over\nPlease stand by\n\nI said please",
        sanitize_ai_name(ai_name)
    )
}

pub fn access_log_line(method: &str, path: &str, status: u16, client_ip: &str) -> String {
    let path_only = path.split(['?', '#']).next().unwrap_or(path);
    format!(
        "{method} {} {status} {client_ip}",
        redact_session_path(path_only)
    )
}

fn redact_session_path(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let mut parts = trimmed.splitn(2, '/');
    let first = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");
    if first.len() == 64 && first.chars().all(|ch| ch.is_ascii_hexdigit()) {
        if rest.is_empty() {
            "/{session}/".to_string()
        } else {
            format!("/{{session}}/{rest}")
        }
    } else {
        "/{rejected}".to_string()
    }
}

pub fn sanitize_help_reason(raw: &str) -> String {
    raw.chars()
        .filter(|ch| !ch.is_control())
        .take(160)
        .collect::<String>()
        .trim()
        .to_string()
}

fn json_u32(value: Option<&Value>) -> Option<u32> {
    let value = value?;
    if let Some(number) = value.as_u64() {
        return u32::try_from(number).ok();
    }
    let number = value.as_f64()?;
    if number.is_finite() && number >= 0.0 && number <= u32::MAX as f64 && number.fract() == 0.0 {
        Some(number as u32)
    } else {
        None
    }
}

fn json_i32(value: Option<&Value>) -> Option<i32> {
    let value = value?;
    if let Some(number) = value.as_i64() {
        return i32::try_from(number).ok();
    }
    let number = value.as_f64()?;
    if number.is_finite()
        && number.fract() == 0.0
        && number >= i32::MIN as f64
        && number <= i32::MAX as f64
    {
        Some(number as i32)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn session() -> GateSession<'static> {
        GateSession {
            token: TOKEN,
            bind_ip: ip("100.100.1.2"),
            magic_dns: Some("hands-host.example.ts.net"),
            port: DEFAULT_PORT,
        }
    }

    fn request<'a>(
        method: &'a str,
        path: &'a str,
        client: Option<IpAddr>,
        host: Option<&'a str>,
    ) -> GateRequest<'a> {
        GateRequest {
            method,
            path,
            client_ip: client,
            host,
            origin: None,
            content_type: None,
            action_header: None,
            body_len: 0,
            content_length: Some(0),
        }
    }

    fn monitor(
        logical_x: i32,
        logical_y: i32,
        logical_w: u32,
        logical_h: u32,
        capture_w: u32,
        capture_h: u32,
        scale: f64,
    ) -> MonitorGeometry {
        MonitorGeometry {
            logical_x,
            logical_y,
            logical_w,
            logical_h,
            capture_w,
            capture_h,
            scale,
        }
    }

    #[test]
    fn tailnet_range_rejects_public_loopback_and_unspecified() {
        assert!(is_tailnet_ipv4(ip("100.64.0.1")));
        assert!(is_tailnet_ipv4(ip("100.127.255.255")));
        assert!(is_tailnet_ipv4(ip("::ffff:100.64.0.1")));
        for rejected in [
            "0.0.0.0",
            "127.0.0.1",
            "8.8.8.8",
            "192.168.1.10",
            "100.63.255.255",
            "100.128.0.1",
            "::1",
        ] {
            assert!(!is_tailnet_ipv4(ip(rejected)), "{rejected}");
        }
    }

    #[test]
    fn tailscale_status_uses_cgnat_ipv4_and_magic_dns() {
        let parsed = parse_tailscale_endpoint(
            r#"{"BackendState":"Running","Self":{"DNSName":"hands-host.example.ts.net.","TailscaleIPs":["fd7a:115c:a1e0::1","100.100.1.2"]}}"#,
        )
        .unwrap();
        assert_eq!(parsed.ipv4, ip("100.100.1.2"));
        assert_eq!(
            parsed.magic_dns.as_deref(),
            Some("hands-host.example.ts.net")
        );

        assert!(parse_tailscale_endpoint(r#"{"BackendState":"NeedsLogin","Self":{}}"#).is_err());
        assert!(parse_tailscale_endpoint(
            r#"{"BackendState":"Running","Self":{"TailscaleIPs":["8.8.8.8","fd7a:115c:a1e0::1"]}}"#
        )
        .is_err());
    }

    #[test]
    fn host_must_be_the_tailnet_name_or_address() {
        let bind = ip("100.100.1.2");
        let dns = Some("hands-host.example.ts.net");
        assert!(host_allowed("100.100.1.2", bind, dns, 8744));
        assert!(host_allowed("100.100.1.2:8744", bind, dns, 8744));
        assert!(host_allowed(
            "Hands-Host.Example.Ts.Net:8744",
            bind,
            dns,
            8744
        ));
        assert!(!host_allowed("100.100.1.2:8745", bind, dns, 8744));
        assert!(!host_allowed("100.100.9.9", bind, dns, 8744));
        assert!(!host_allowed("0.0.0.0", bind, dns, 8744));
        assert!(!host_allowed("evil.example", bind, dns, 8744));
        assert!(!host_allowed(
            "hands-host.example.ts.net.evil.example",
            bind,
            dns,
            8744
        ));
    }

    #[test]
    fn request_gate_rejects_the_open_internet_and_bad_actions() {
        let session = session();
        let phone = ip("100.88.0.5");
        let path = format!("/{TOKEN}/action");
        let page = format!("/{TOKEN}/");

        assert_eq!(
            authorize(
                &request(
                    "GET",
                    &page,
                    Some(phone),
                    Some("hands-host.example.ts.net:8744")
                ),
                session
            ),
            Gate::Allow(Route::Page)
        );

        let mut loopback = request(
            "GET",
            &page,
            Some(ip("127.0.0.1")),
            Some("hands-host.example.ts.net:8744"),
        );
        assert_eq!(authorize(&loopback, session), Gate::Reject(403));

        loopback.client_ip = Some(phone);
        loopback.host = Some("evil.example:8744");
        assert_eq!(authorize(&loopback, session), Gate::Reject(403));

        let mut oversized = request("POST", &path, Some(phone), Some("100.100.1.2:8744"));
        oversized.content_length = Some(MAX_BODY_BYTES + 1);
        assert_eq!(authorize(&oversized, session), Gate::Reject(413));

        let mut action = request("POST", &path, Some(phone), Some("100.100.1.2:8744"));
        action.body_len = 32;
        action.content_length = Some(32);
        assert_eq!(authorize(&action, session), Gate::Reject(403));

        action.action_header = Some("1");
        action.origin = Some("http://evil.example");
        action.content_type = Some("application/json");
        assert_eq!(authorize(&action, session), Gate::Reject(403));

        action.origin = Some("https://100.100.1.2:8744");
        assert_eq!(authorize(&action, session), Gate::Reject(403));

        action.origin = Some("http://100.100.1.2:8744");
        action.content_type = Some("text/plain");
        assert_eq!(authorize(&action, session), Gate::Reject(400));

        action.content_type = Some("application/json; charset=utf-8");
        assert_eq!(authorize(&action, session), Gate::Allow(Route::Action));

        let wrong = format!("/{TOKEN}ff/frame");
        assert_eq!(
            authorize(
                &request("GET", &wrong, Some(phone), Some("100.100.1.2:8744")),
                session
            ),
            Gate::Reject(403)
        );
        let other = format!("/{:064x}/frame", 1);
        assert_eq!(
            authorize(
                &request("GET", &other, Some(phone), Some("100.100.1.2")),
                session
            ),
            Gate::Reject(403)
        );
    }

    #[test]
    fn viewport_width_caps_at_1280_and_served_size_does_not_upscale() {
        assert_eq!(parse_viewport_width("").unwrap(), 1280);
        assert_eq!(parse_viewport_width("w=390").unwrap(), 390);
        assert_eq!(parse_viewport_width("w=4000").unwrap(), 1280);
        assert!(parse_viewport_width("w=0").is_err());
        assert_eq!(served_size(800, 600, 1280), Some((800, 600)));
        assert_eq!(served_size(1920, 1080, 1280), Some((1280, 720)));
    }

    #[test]
    fn click_mapping_aims_at_pixel_centres_across_monitors() {
        let desktop = place_monitors(&[monitor(0, 0, 100, 80, 100, 80, 1.0)]);
        assert_eq!(map_click(0.0, 0.0, 100, 80, &desktop), Some((0, 0)));
        assert_eq!(map_click(10.2, 20.8, 100, 80, &desktop), Some((10, 20)));
        assert_eq!(map_click(99.9, 79.2, 100, 80, &desktop), Some((99, 79)));
        assert_eq!(map_click(100.0, 0.0, 100, 80, &desktop), None);

        let scaled = place_monitors(&[
            monitor(0, 0, 100, 100, 100, 100, 1.0),
            monitor(100, 0, 100, 100, 200, 200, 2.0),
        ]);
        assert_eq!(scaled.width, 400);
        assert!(scaled.monitors.iter().any(|item| item.image_x == 200));
        assert_eq!(map_click(250.2, 40.2, 400, 200, &scaled), Some((125, 20)));
        assert_eq!(map_click(150.0, 10.0, 400, 200, &scaled), None);

        let shifted = place_monitors(&[monitor(-100, -20, 100, 40, 100, 40, 1.0)]);
        assert_eq!(map_click(10.4, 5.2, 100, 40, &shifted), Some((-90, -15)));

        let overlapped = place_monitors(&[
            monitor(0, 0, 100, 100, 200, 200, 2.0),
            monitor(100, 0, 100, 100, 100, 100, 1.0),
        ]);
        let second = overlapped
            .monitors
            .iter()
            .find(|item| item.geometry.logical_x == 100)
            .unwrap();
        assert!(second.image_x >= 200);
        let click_x = second.image_x as f64 + 0.2;
        assert_eq!(
            map_click(
                click_x,
                0.2,
                overlapped.width,
                overlapped.height,
                &overlapped
            ),
            Some((100, 0))
        );
    }

    #[test]
    fn absolute_mouse_axis_uses_the_pixel_centre() {
        assert_eq!(absolute_mouse_axis(0, 0, 100), Some(328));
        assert_eq!(absolute_mouse_axis(99, 0, 100), Some(65207));
        assert!(absolute_mouse_axis(-100, -100, 200).is_some());
        assert_eq!(absolute_mouse_axis(50, 0, 0), None);
        assert_eq!(absolute_mouse_axis(-1, 0, 100), None);
    }

    #[test]
    fn dangerous_combos_are_blocked_and_ordinary_ones_are_not() {
        for blocked in [
            "win+l",
            "Win+L",
            "lwin+l",
            "meta+L",
            "win+shift+l",
            "win+r",
            "win+d",
            "win+m",
            "win+x",
            "win+u",
            "win+pause",
            "alt+f4",
            "alt+F4",
            "ctrl+alt+del",
            "ctrl+alt+delete",
            "control+alt+del",
            "ctrl+alt+end",
            "ctrl+shift+esc",
            "ctrl+shift+escape",
        ] {
            let combo = parse_combo(blocked).unwrap_or_else(|_| panic!("parse {blocked}"));
            assert!(combo_is_blocked(&combo), "{blocked}");
            let parsed = parse_action(&json!({"op":"combo","keys": blocked}));
            assert_eq!(parsed, Err(ActionFault::Blocked), "{blocked}");
        }

        for allowed in ["ctrl+l", "ctrl+shift+t", "ctrl+c", "alt+tab", "enter"] {
            let combo = parse_combo(allowed).unwrap();
            assert!(!combo_is_blocked(&combo), "{allowed}");
        }

        assert!(parse_combo("win").is_err());
        assert!(parse_combo("ctrl+notakey").is_err());
        assert_eq!(
            parse_action(&json!({"op":"key","key":"ctrl+l"})),
            Ok(OwnerAction::Key { name: "ctrl+l" })
        );
        assert_eq!(
            parse_action(&json!({"op":"key","key":"win+l"})),
            Err(ActionFault::BadInput)
        );
    }

    #[test]
    fn action_parser_rejects_bad_input_without_keeping_secrets_in_the_error() {
        assert!(matches!(
            parse_action(&json!({"op":"type","text":"secret"})),
            Ok(OwnerAction::Type { .. })
        ));
        assert_eq!(
            parse_action(&json!({"op":"type","text":""})),
            Err(ActionFault::BadInput)
        );
        assert_eq!(
            parse_action(&json!({"op":"type","text":"\u{0001}"})),
            Err(ActionFault::BadInput)
        );
        assert_eq!(
            parse_action(
                &json!({"op":"scroll","x":1.0,"y":1.0,"image_width":10,"image_height":10,"dx":0,"dy":0})
            ),
            Err(ActionFault::BadInput)
        );
        assert_eq!(
            parse_action(
                &json!({"op":"scroll","x":1.0,"y":1.0,"image_width":10,"image_height":10,"dx":0,"dy":80})
            ),
            Err(ActionFault::BadInput)
        );
        assert_eq!(
            parse_action(
                &json!({"op":"click","x":1.0,"y":1.0,"image_width":10,"image_height":10,"button":"right","double":true})
            ),
            Ok(OwnerAction::Click {
                x: 1.0,
                y: 1.0,
                image_width: 10,
                image_height: 10,
                button: Button::Right,
                double: true,
            })
        );
        let fault = parse_action(&json!({"op":"type","text":"\u{0001}secret"})).unwrap_err();
        assert_eq!(fault, ActionFault::BadInput);
        assert!(!format!("{fault:?}").contains("secret"));
    }

    #[test]
    fn banner_defaults_are_generic_and_configurable() {
        let text = banner_text("Assistant", None);
        assert_eq!(
            text,
            "Assistant is taking over\nPlease stand by\n\nI said please"
        );
        assert_eq!(banner_text("  ", Some("One\\nTwo")), "One\nTwo");
        assert_eq!(sanitize_ai_name("\n\t"), "Assistant");
        assert_eq!(sanitize_ai_name("  Desk "), "Desk");
    }

    #[test]
    fn access_log_keeps_method_path_status_and_client_only() {
        let line = access_log_line(
            "POST",
            &format!("/{TOKEN}/action?text=secret"),
            200,
            "100.88.0.5",
        );
        assert_eq!(line, "POST /{session}/action 200 100.88.0.5");
        assert!(!line.contains(TOKEN));
        assert!(!line.contains("secret"));
        assert_eq!(
            access_log_line("GET", "/nope", 403, "8.8.8.8"),
            "GET /{rejected} 403 8.8.8.8"
        );
    }
}
