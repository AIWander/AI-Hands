//! Tailnet-only live screen.
//!
//! An owner opens one page on a phone that is already on the same tailnet,
//! watches a downscaled JPEG of this machine, and can click or type. Hands
//! keeps serving its normal tools while that page is open. The listener binds
//! the machine's Tailscale IPv4 only.

mod policy;

#[cfg(windows)]
mod platform;

use policy::{
    access_log_line, authorize, banner_text, combo_is_blocked, parse_action,
    parse_tailscale_endpoint, parse_viewport_width, sanitize_ai_name, sanitize_help_reason,
    DesktopMap, Gate, GateRequest, GateSession, OwnerAction, Route, DEFAULT_PORT, MAX_BODY_BYTES,
    VIEWER_WINDOW_SECS,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PRESERVED_FIELDS: &[&str] = &["/url", "/fallback_url", "/message"];

pub fn preserved_output_pointers(name: &str) -> &'static [&'static str] {
    if matches!(
        name,
        "live_screen_start"
            | "live_screen_status"
            | "live_screen_stop"
            | "live_screen_request_help"
    ) {
        PRESERVED_FIELDS
    } else {
        &[]
    }
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "live_screen_start",
            "description": "Open a tailnet-only live view of this machine's screen. Binds the Tailscale IPv4 discovered at runtime and refuses to start when there is no tailnet. Does not bind a public address and does not open Tailscale Funnel. Returns a MagicDNS URL plus an IP fallback. The owner opens it on a phone already on the tailnet. Optional port (default 8744) and ttl_minutes (1 through 1440).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "port": { "type": "integer", "minimum": 1, "maximum": 65535, "default": DEFAULT_PORT, "description": "TCP port on the Tailscale address. Default 8744." },
                    "ttl_minutes": { "type": "integer", "minimum": 1, "maximum": 1440, "description": "Stop the session after this many minutes. Omit to run until live_screen_stop." }
                }
            }
        }),
        json!({
            "name": "live_screen_status",
            "description": "Report whether the tailnet live screen is running, the URL the owner should open, and whether a viewer has polled recently.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "live_screen_stop",
            "description": "Stop the live screen, invalidate its URL token, and hide the on-screen banner.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "live_screen_request_help",
            "description": "Use this when a sign-in, captcha, UAC prompt, or unexpected dialog is blocking you and the owner is away from the computer. Starts the tailnet live screen if it is not already running and returns a short message you can relay. Do not type passwords or other secrets for the user. You can keep calling normal Hands tools while the owner watches.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reason": { "type": "string", "description": "Short reason the owner will see, with no secrets." },
                    "port": { "type": "integer", "minimum": 1, "maximum": 65535 },
                    "ttl_minutes": { "type": "integer", "minimum": 1, "maximum": 1440 }
                },
                "required": ["reason"]
            }
        }),
    ]
}

pub fn handle_tool(name: &str, args: &Value) -> Option<Value> {
    match name {
        "live_screen_start" => Some(start(args)),
        "live_screen_status" => Some(status()),
        "live_screen_stop" => Some(stop()),
        "live_screen_request_help" => Some(request_help(args)),
        _ => None,
    }
}

struct FrameState {
    etag: String,
    width: u32,
    height: u32,
    map: DesktopMap,
}

struct Live {
    token: String,
    port: u16,
    bind_ip: IpAddr,
    magic_dns: Option<String>,
    started: Instant,
    ttl: Option<Duration>,
    stop: AtomicBool,
    server: Arc<tiny_http::Server>,
    last_viewer: Mutex<Option<Instant>>,
    last_action: Mutex<Option<Instant>>,
    action_lock: Mutex<()>,
    frame: Mutex<Option<FrameState>>,
}

struct Runtime {
    live: Arc<Live>,
    http: JoinHandle<()>,
    banner: Option<JoinHandle<()>>,
}

fn runtime_slot() -> &'static Mutex<Option<Runtime>> {
    static SLOT: Mutex<Option<Runtime>> = Mutex::new(None);
    &SLOT
}

fn start(args: &Value) -> Value {
    #[cfg(not(windows))]
    {
        let _ = args;
        return windows_required();
    }
    #[cfg(windows)]
    start_on_windows(args)
}

#[cfg(windows)]
fn start_on_windows(args: &Value) -> Value {
    let port = match parse_port(args.get("port")) {
        Ok(port) => port,
        Err(error) => return failure(error),
    };
    let ttl = match parse_ttl(args.get("ttl_minutes")) {
        Ok(ttl) => ttl,
        Err(error) => return failure(error),
    };
    retire_expired();
    if let Some(existing) = running() {
        if existing.port != port {
            return failure(
                "A live screen is already running on a different port. Stop it before choosing another port.",
            );
        }
        return session_payload(&existing, true);
    }
    let endpoint = match tailscale_endpoint() {
        Ok(endpoint) => endpoint,
        Err(error) => return failure(error),
    };
    if !policy::is_tailnet_ipv4(endpoint.ipv4) || endpoint.ipv4.is_unspecified() {
        return failure(
            "Live screen refused to start because the Tailscale address was not in 100.64.0.0/10.",
        );
    }
    let token = match new_token() {
        Ok(token) => token,
        Err(()) => return failure("Live screen could not generate a session token."),
    };
    let addr = SocketAddr::from((endpoint.ipv4, port));
    let server = match tiny_http::Server::http(addr) {
        Ok(server) => Arc::new(server),
        Err(_) => {
            return failure(
                "Live screen could not bind that TCP port on the Tailscale address. Pick another port or stop whatever is already using it. It will not fall back to a public address.",
            );
        }
    };
    let live = Arc::new(Live {
        token,
        port,
        bind_ip: endpoint.ipv4,
        magic_dns: endpoint.magic_dns,
        started: Instant::now(),
        ttl,
        stop: AtomicBool::new(false),
        server: Arc::clone(&server),
        last_viewer: Mutex::new(None),
        last_action: Mutex::new(None),
        action_lock: Mutex::new(()),
        frame: Mutex::new(None),
    });
    let serve = Arc::clone(&live);
    let http = match thread::Builder::new()
        .name("hands-live-screen".to_string())
        .spawn(move || serve_loop(serve, server))
    {
        Ok(handle) => handle,
        Err(_) => {
            return failure("Live screen could not start its listener thread.");
        }
    };
    let banner_live = Arc::clone(&live);
    let banner = thread::Builder::new()
        .name("hands-live-banner".to_string())
        .spawn(move || {
            platform::run_banner(
                || !banner_live.stop.load(Ordering::SeqCst),
                || banner_live.visible_banner(),
            );
        })
        .ok();
    let payload = session_payload(&live, false);
    let mut slot = lock_runtime();
    if slot.is_some() {
        live.stop.store(true, Ordering::SeqCst);
        live.server.unblock();
        drop(slot);
        let _ = http.join();
        if let Some(banner) = banner {
            let _ = banner.join();
        }
        return failure(
            "A live screen started while this one was opening. Use live_screen_status.",
        );
    }
    *slot = Some(Runtime { live, http, banner });
    payload
}

fn request_help(args: &Value) -> Value {
    #[cfg(not(windows))]
    {
        let _ = args;
        return windows_required();
    }
    #[cfg(windows)]
    {
        let reason = match args.get("reason").and_then(Value::as_str) {
            Some(reason) => sanitize_help_reason(reason),
            None => return failure("live_screen_request_help requires a short reason."),
        };
        let mut payload = start(args);
        if payload.get("success") != Some(&json!(true)) {
            return payload;
        }
        let url = payload
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let fallback = payload
            .get("fallback_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "message".to_string(),
                json!(help_message(&url, &fallback, &reason)),
            );
            object.insert("reason".to_string(), json!(reason));
        }
        payload
    }
}

fn status() -> Value {
    #[cfg(not(windows))]
    {
        return windows_required();
    }
    #[cfg(windows)]
    {
        retire_expired();
        match running() {
            Some(live) => session_payload(&live, true),
            None => json!({ "success": true, "running": false }),
        }
    }
}

fn stop() -> Value {
    #[cfg(not(windows))]
    {
        return windows_required();
    }
    #[cfg(windows)]
    {
        let runtime = lock_runtime().take();
        let Some(runtime) = runtime else {
            return json!({ "success": true, "running": false });
        };
        runtime.live.stop.store(true, Ordering::SeqCst);
        runtime.live.server.unblock();
        let _ = runtime.http.join();
        if let Some(banner) = runtime.banner {
            let _ = banner.join();
        }
        json!({ "success": true, "running": false })
    }
}

fn running() -> Option<Arc<Live>> {
    let guard = lock_runtime();
    let runtime = guard.as_ref()?;
    if runtime.live.stop.load(Ordering::SeqCst) || runtime.live.expired() {
        return None;
    }
    Some(Arc::clone(&runtime.live))
}

fn retire_expired() {
    let expired = {
        let guard = lock_runtime();
        guard.as_ref().is_some_and(|runtime| {
            runtime.live.expired() || runtime.live.stop.load(Ordering::SeqCst)
        })
    };
    if expired {
        let _ = stop();
    }
}

fn lock_runtime() -> std::sync::MutexGuard<'static, Option<Runtime>> {
    runtime_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[cfg(not(windows))]
fn windows_required() -> Value {
    failure("Live screen capture and input require Windows.")
}

fn failure(error: &str) -> Value {
    json!({ "success": false, "error": error })
}

fn session_payload(live: &Live, already_running: bool) -> Value {
    let (url, fallback_url) = live.urls();
    json!({
        "success": true,
        "running": true,
        "already_running": already_running,
        "port": live.port,
        "url": url,
        "fallback_url": fallback_url,
        "bind": "tailscale-ipv4",
        "viewer_recent": live.attention(),
        "banner": live.banner_label(),
        "banner_excluded_from_capture": banner_excluded(),
        "expires_in_seconds": live.remaining_seconds(),
        "acl_note": acl_note(live.port),
    })
}

fn acl_note(port: u16) -> String {
    format!(
        "Tailscale ACLs must allow this TCP port from the owner's phone to this machine. Example grant: {{\"src\":[\"<owner phone>\"],\"dst\":[\"<this machine>\"],\"ip\":[\"tcp:{port}\"]}}. The Windows Firewall program rule for this executable must allow the Private profile. Tailscale is a Private network. Live screen does not bind a public address and does not open Tailscale Funnel."
    )
}

fn help_message(url: &str, fallback: &str, reason: &str) -> String {
    let mut message = format!(
        "Open this link on your phone to help. You can watch the screen and take over with taps or typing. I will keep working, and I will not type a password. {url}"
    );
    if fallback != url {
        message.push_str(&format!(" If that name does not open, use {fallback}."));
    }
    if !reason.is_empty() {
        message.push_str(&format!(" Asked because: {reason}."));
    }
    message
}

fn tailscale_endpoint() -> Result<policy::TailscaleEndpoint, &'static str> {
    let text = crate::setup_status::tailscale_status_text().map_err(|_| {
        "Tailscale is not available. Connect this machine to a tailnet before starting live screen."
    })?;
    parse_tailscale_endpoint(&text).map_err(|_| {
        "Live screen refused to start because this machine has no Tailscale IPv4 in 100.64.0.0/10. Connect to a tailnet first. It does not bind a public address and does not open Tailscale Funnel."
    })
}

fn new_token() -> Result<String, ()> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| ())?;
    let alphabet = b"0123456789abcdef";
    let mut token = String::with_capacity(64);
    for byte in bytes {
        token.push(char::from(alphabet[(byte >> 4) as usize]));
        token.push(char::from(alphabet[(byte & 0x0f) as usize]));
    }
    Ok(token)
}

fn parse_port(value: Option<&Value>) -> Result<u16, &'static str> {
    let Some(value) = value else {
        return Ok(DEFAULT_PORT);
    };
    if value.is_null() {
        return Ok(DEFAULT_PORT);
    }
    let port = value
        .as_u64()
        .ok_or("port must be an integer from 1 to 65535")?;
    if (1..=u16::MAX as u64).contains(&port) {
        Ok(port as u16)
    } else {
        Err("port must be an integer from 1 to 65535")
    }
}

fn parse_ttl(value: Option<&Value>) -> Result<Option<Duration>, &'static str> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let minutes = value
        .as_u64()
        .ok_or("ttl_minutes must be an integer from 1 to 1440")?;
    if (1..=1440).contains(&minutes) {
        Ok(Some(Duration::from_secs(minutes * 60)))
    } else {
        Err("ttl_minutes must be an integer from 1 to 1440")
    }
}

impl Live {
    fn urls(&self) -> (String, String) {
        let fallback = format!("http://{}:{}/{}/", self.bind_ip, self.port, self.token);
        let url = match &self.magic_dns {
            Some(name) => format!("http://{name}:{}/{}/", self.port, self.token),
            None => fallback.clone(),
        };
        (url, fallback)
    }

    fn expired(&self) -> bool {
        self.ttl.is_some_and(|ttl| self.started.elapsed() >= ttl)
    }

    fn remaining_seconds(&self) -> Option<u64> {
        self.ttl
            .map(|ttl| ttl.saturating_sub(self.started.elapsed()).as_secs())
    }

    fn attention(&self) -> bool {
        recent(&self.last_viewer) || recent(&self.last_action)
    }

    fn note_viewer(&self) {
        note(&self.last_viewer);
    }

    fn note_action(&self) {
        note(&self.last_action);
    }

    fn visible_banner(&self) -> Option<String> {
        if banner_disabled() || !self.attention() {
            None
        } else {
            Some(current_banner_text())
        }
    }

    fn banner_label(&self) -> &'static str {
        if banner_disabled() {
            "disabled"
        } else if banner_failed() {
            "unavailable"
        } else if self.attention() {
            "shown"
        } else {
            "hidden"
        }
    }
}

fn recent(slot: &Mutex<Option<Instant>>) -> bool {
    slot.lock()
        .ok()
        .and_then(|guard| *guard)
        .is_some_and(|instant| instant.elapsed() <= Duration::from_secs(VIEWER_WINDOW_SECS))
}

fn note(slot: &Mutex<Option<Instant>>) {
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(Instant::now());
    }
}

fn banner_disabled() -> bool {
    std::env::var("HANDS_LIVE_SCREEN_DISABLE_BANNER")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn banner_failed() -> bool {
    #[cfg(windows)]
    {
        platform::banner_failed()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn banner_excluded() -> bool {
    #[cfg(windows)]
    {
        platform::banner_excluded_from_capture()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn current_banner_text() -> String {
    let name = std::env::var("HANDS_LIVE_SCREEN_AI_NAME")
        .ok()
        .map(|value| sanitize_ai_name(&value))
        .unwrap_or_else(|| "Assistant".to_string());
    let override_text = std::env::var("HANDS_LIVE_SCREEN_BANNER_TEXT").ok();
    banner_text(&name, override_text.as_deref())
}

fn serve_loop(live: Arc<Live>, server: Arc<tiny_http::Server>) {
    for request in server.incoming_requests() {
        if live.stop.load(Ordering::SeqCst) || live.expired() {
            live.stop.store(true, Ordering::SeqCst);
            let _ = send_json(request, 403, &json!({"error":"rejected"}));
            break;
        }
        dispatch_request(request, &live);
    }
    release_if_current(&live);
}

fn release_if_current(live: &Arc<Live>) {
    let runtime = {
        let mut guard = lock_runtime();
        let current = guard
            .as_ref()
            .is_some_and(|runtime| Arc::ptr_eq(&runtime.live, live));
        if current {
            guard.take()
        } else {
            None
        }
    };
    let Some(runtime) = runtime else {
        return;
    };
    runtime.live.stop.store(true, Ordering::SeqCst);
    let Runtime { live, http, banner } = runtime;
    drop(http);
    if let Some(banner) = banner {
        let _ = banner.join();
    }
    drop(live);
}

fn dispatch_request(mut request: tiny_http::Request, live: &Live) {
    let method = request.method().as_str().to_string();
    let url = request.url().to_string();
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (url.clone(), String::new()),
    };
    let client_ip = request.remote_addr().map(|addr| addr.ip());
    let host = header(&request, "Host");
    let origin = header(&request, "Origin");
    let content_type = header(&request, "Content-Type");
    let action_header = header(&request, policy::ACTION_HEADER);
    let if_none_match = header(&request, "If-None-Match");
    let content_length = request.body_length();
    let body = if method.eq_ignore_ascii_case("POST") {
        match read_body(&mut request) {
            Ok(body) => body,
            Err(status) => {
                log_access(&method, &path, status, client_ip);
                let error = if status == 413 {
                    "too_large"
                } else {
                    "unexpected"
                };
                let _ = send_json(request, status, &json!({"error": error}));
                return;
            }
        }
    } else {
        Vec::new()
    };
    let gate = authorize(
        &GateRequest {
            method: &method,
            path: &path,
            client_ip,
            host: host.as_deref(),
            origin: origin.as_deref(),
            content_type: content_type.as_deref(),
            action_header: action_header.as_deref(),
            body_len: body.len(),
            content_length,
        },
        GateSession {
            token: &live.token,
            bind_ip: live.bind_ip,
            magic_dns: live.magic_dns.as_deref(),
            port: live.port,
        },
    );
    let status = match gate {
        Gate::Reject(status) => {
            log_access(&method, &path, status, client_ip);
            let error = match status {
                413 => "too_large",
                400 => "bad_input",
                _ => "rejected",
            };
            let _ = send_json(request, status, &json!({"error": error}));
            return;
        }
        Gate::Allow(route) => match route {
            Route::Page => send_page(request),
            Route::Frame => send_frame(request, live, &query, if_none_match.as_deref()),
            Route::Action => send_action(request, live, &body),
        },
    };
    if matches!(status, 200 | 304) && path_ends_with_page_or_frame(&path) {
        live.note_viewer();
    }
    log_access(&method, &path, status, client_ip);
}

fn path_ends_with_page_or_frame(path: &str) -> bool {
    let trimmed = path.trim_end_matches('/');
    !trimmed.ends_with("/action")
}

fn send_page(request: tiny_http::Request) -> u16 {
    let _ = send_bytes(
        request,
        200,
        "text/html; charset=utf-8",
        PAGE.as_bytes(),
        &[],
    );
    200
}

fn send_frame(
    request: tiny_http::Request,
    live: &Live,
    query: &str,
    if_none_match: Option<&str>,
) -> u16 {
    let viewport = match parse_viewport_width(query) {
        Ok(width) => width,
        Err(()) => {
            let _ = send_json(request, 400, &json!({"error":"bad_input"}));
            return 400;
        }
    };
    #[cfg(not(windows))]
    {
        let _ = (live, viewport, if_none_match);
        let _ = send_json(request, 500, &json!({"error":"unexpected"}));
        return 500;
    }
    #[cfg(windows)]
    {
        let captured = match platform::capture_jpeg(viewport) {
            Ok(captured) => captured,
            Err(()) => {
                let _ = send_json(request, 500, &json!({"error":"unexpected"}));
                return 500;
            }
        };
        let etag = frame_etag(&captured.jpeg);
        let unchanged = live
            .frame
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|frame| frame.etag == etag))
            .unwrap_or(false);
        if let Ok(mut guard) = live.frame.lock() {
            *guard = Some(FrameState {
                etag: etag.clone(),
                width: captured.width,
                height: captured.height,
                map: captured.map,
            });
        }
        if if_none_match.is_some_and(|value| value.split(',').any(|part| part.trim() == etag)) {
            let _ = send_empty(request, 304, &[("ETag", etag)]);
            304
        } else if unchanged {
            let _ = send_bytes(
                request,
                200,
                "text/plain; charset=utf-8",
                b"unchanged",
                &[("ETag", etag)],
            );
            200
        } else {
            let _ = send_bytes(
                request,
                200,
                "image/jpeg",
                &captured.jpeg,
                &[("ETag", etag)],
            );
            200
        }
    }
}

fn send_action(request: tiny_http::Request, live: &Live, body: &[u8]) -> u16 {
    let value = match serde_json::from_slice::<Value>(body) {
        Ok(value) => value,
        Err(_) => {
            let _ = send_json(request, 400, &json!({"error":"bad_input"}));
            return 400;
        }
    };
    let action = match parse_action(&value) {
        Ok(action) => action,
        Err(fault) => {
            let status = fault.status();
            let error = if status == 409 {
                "blocked"
            } else {
                "bad_input"
            };
            let _ = send_json(request, status, &json!({"error": error}));
            return status;
        }
    };
    let _guard = match live.action_lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => {
            let _ = send_json(request, 409, &json!({"error":"busy"}));
            return 409;
        }
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
    };
    live.note_action();
    match perform(live, action) {
        Ok(()) => {
            let _ = send_json(request, 200, &json!({"ok":true}));
            200
        }
        Err(ActionResult::BadInput) => {
            let _ = send_json(request, 400, &json!({"error":"bad_input"}));
            400
        }
        Err(ActionResult::Blocked) => {
            let _ = send_json(request, 409, &json!({"error":"blocked"}));
            409
        }
        Err(ActionResult::FocusChanged(typed)) => {
            let _ = send_json(
                request,
                409,
                &json!({"error":"focus_changed","typed": typed}),
            );
            409
        }
        Err(ActionResult::Unexpected) => {
            let _ = send_json(request, 500, &json!({"error":"unexpected"}));
            500
        }
    }
}

enum ActionResult {
    BadInput,
    Blocked,
    FocusChanged(usize),
    Unexpected,
}

fn perform(live: &Live, action: OwnerAction) -> Result<(), ActionResult> {
    match action {
        OwnerAction::Click {
            x,
            y,
            image_width,
            image_height,
            button,
            double,
        } => {
            let (screen_x, screen_y) = screen_point(live, x, y, image_width, image_height)?;
            #[cfg(windows)]
            {
                platform::click_at(screen_x, screen_y, button, double)
                    .map_err(|_| ActionResult::Unexpected)
            }
            #[cfg(not(windows))]
            {
                let _ = (screen_x, screen_y, button, double);
                Err(ActionResult::Unexpected)
            }
        }
        OwnerAction::Scroll {
            x,
            y,
            image_width,
            image_height,
            dx,
            dy,
        } => {
            let (screen_x, screen_y) = screen_point(live, x, y, image_width, image_height)?;
            #[cfg(windows)]
            {
                platform::scroll_at(screen_x, screen_y, dx, dy)
                    .map_err(|_| ActionResult::Unexpected)
            }
            #[cfg(not(windows))]
            {
                let _ = (screen_x, screen_y, dx, dy);
                Err(ActionResult::Unexpected)
            }
        }
        OwnerAction::Type { text } => {
            #[cfg(windows)]
            {
                match platform::type_text(&text) {
                    Ok(_) => Ok(()),
                    Err(platform::TypeStop::FocusChanged(typed)) => {
                        Err(ActionResult::FocusChanged(typed))
                    }
                }
            }
            #[cfg(not(windows))]
            {
                let _ = text;
                Err(ActionResult::Unexpected)
            }
        }
        OwnerAction::Key { name } => {
            #[cfg(windows)]
            {
                platform::tap_key(name).map_err(|_| ActionResult::Unexpected)
            }
            #[cfg(not(windows))]
            {
                let _ = name;
                Err(ActionResult::Unexpected)
            }
        }
        OwnerAction::Combo { combo } => {
            if combo_is_blocked(&combo) {
                return Err(ActionResult::Blocked);
            }
            #[cfg(windows)]
            {
                platform::tap_combo(&combo).map_err(|_| ActionResult::Unexpected)
            }
            #[cfg(not(windows))]
            {
                let _ = combo;
                Err(ActionResult::Unexpected)
            }
        }
    }
}

fn screen_point(
    live: &Live,
    x: f64,
    y: f64,
    image_width: u32,
    image_height: u32,
) -> Result<(i32, i32), ActionResult> {
    let guard = live.frame.lock().unwrap_or_else(|error| error.into_inner());
    let Some(frame) = guard.as_ref() else {
        return Err(ActionResult::BadInput);
    };
    if frame.width != image_width || frame.height != image_height {
        return Err(ActionResult::BadInput);
    }
    policy::map_click(x, y, image_width, image_height, &frame.map).ok_or(ActionResult::BadInput)
}

fn read_body(request: &mut tiny_http::Request) -> Result<Vec<u8>, u16> {
    if request
        .body_length()
        .is_some_and(|length| length > MAX_BODY_BYTES)
    {
        return Err(413);
    }
    let mut body = Vec::new();
    let mut limited = request.as_reader().take(MAX_BODY_BYTES as u64 + 1);
    limited.read_to_end(&mut body).map_err(|_| 500u16)?;
    if body.len() > MAX_BODY_BYTES {
        Err(413)
    } else {
        Ok(body)
    }
}

fn header(request: &tiny_http::Request, name: &'static str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str().to_string())
}

fn frame_etag(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("\"{hex}\"")
}

fn log_access(method: &str, path: &str, status: u16, client_ip: Option<IpAddr>) {
    let client = client_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    eprintln!(
        "[hands/live-screen] {}",
        access_log_line(method, path, status, &client)
    );
}

fn send_json(request: tiny_http::Request, status: u16, body: &Value) -> Result<(), ()> {
    let bytes = serde_json::to_vec(body).map_err(|_| ())?;
    send_bytes(
        request,
        status,
        "application/json; charset=utf-8",
        &bytes,
        &[],
    )
}

fn send_empty(
    request: tiny_http::Request,
    status: u16,
    extra: &[(&str, String)],
) -> Result<(), ()> {
    let mut response = tiny_http::Response::empty(status);
    add_security_headers(&mut response, "text/plain");
    for (name, value) in extra {
        if let Ok(header) = format!("{name}: {value}").parse::<tiny_http::Header>() {
            response.add_header(header);
        }
    }
    request.respond(response).map_err(|_| ())
}

fn send_bytes(
    request: tiny_http::Request,
    status: u16,
    content_type: &str,
    body: &[u8],
    extra: &[(&str, String)],
) -> Result<(), ()> {
    let mut response = tiny_http::Response::from_data(body.to_vec()).with_status_code(status);
    add_security_headers(&mut response, content_type);
    for (name, value) in extra {
        if let Ok(header) = format!("{name}: {value}").parse::<tiny_http::Header>() {
            response.add_header(header);
        }
    }
    request.respond(response).map_err(|_| ())
}

fn add_security_headers<R: Read>(response: &mut tiny_http::Response<R>, content_type: &str) {
    for line in [
        format!("Content-Type: {content_type}"),
        "X-Frame-Options: DENY".to_string(),
        "Content-Security-Policy: default-src 'none'; img-src 'self' blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'".to_string(),
        "X-Content-Type-Options: nosniff".to_string(),
        "Referrer-Policy: no-referrer".to_string(),
        "Cache-Control: no-store".to_string(),
    ] {
        if let Ok(header) = line.parse::<tiny_http::Header>() {
            response.add_header(header);
        }
    }
}

const PAGE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Live screen</title>
<style>
body { margin: 0; font: 16px system-ui, sans-serif; background: #111; color: #eee; }
img { width: 100%; height: auto; background: #000; touch-action: none; }
.bar { display: flex; flex-wrap: wrap; gap: 8px; padding: 8px; }
button, input { font: inherit; padding: 10px 12px; }
input { flex: 1; min-width: 8rem; }
#status { padding: 0 8px 12px; min-height: 1.2em; }
</style>
</head>
<body>
<img id="screen" alt="Live screen">
<div class="bar">
  <button type="button" data-mode="left">Tap</button>
  <button type="button" data-mode="right">Right-click</button>
  <button type="button" data-mode="double">Double-click</button>
</div>
<div class="bar">
  <input id="typed" type="password" autocomplete="off" autocapitalize="off" spellcheck="false" maxlength="512" placeholder="Type here">
  <button type="button" id="send-text">Send</button>
</div>
<div class="bar">
  <button type="button" data-key="enter">Enter</button>
  <button type="button" data-key="tab">Tab</button>
  <button type="button" data-key="esc">Esc</button>
  <button type="button" data-key="backspace">Backspace</button>
  <button type="button" data-key="ctrl+l">Ctrl+L</button>
</div>
<div class="bar">
  <input id="combo" type="text" autocomplete="off" maxlength="64" placeholder="Key combo, such as ctrl+shift+t">
  <button type="button" id="send-combo">Send combo</button>
</div>
<p id="status"></p>
<script>
const root = location.pathname.endsWith('/') ? location.pathname : location.pathname + '/';
const img = document.getElementById('screen');
const status = document.getElementById('status');
let mode = 'left';
let etag = '';
let ignoreUntil = 0;
let pending = 0;
let scrollTimer = null;
let touchY = null;
function setStatus(text) { status.textContent = text; }
function point(ev) {
  const rect = img.getBoundingClientRect();
  const width = img.naturalWidth || rect.width;
  const height = img.naturalHeight || rect.height;
  return {
    x: (ev.clientX - rect.left) * (width / rect.width),
    y: (ev.clientY - rect.top) * (height / rect.height),
    image_width: width,
    image_height: height
  };
}
async function post(body) {
  const res = await fetch(root + 'action', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-Hands-Live-Action': '1' },
    body: JSON.stringify(body)
  });
  if (res.status === 409) {
    const payload = await res.json().catch(function () { return {}; });
    if (payload.error === 'focus_changed') setStatus('Typing stopped because the focused window changed.');
    else if (payload.error === 'busy') setStatus('Another action is still running.');
    else setStatus('That action is blocked.');
    return;
  }
  if (!res.ok) setStatus('The action was not accepted.');
  else setStatus('');
}
document.querySelectorAll('[data-mode]').forEach(function (button) {
  button.addEventListener('click', function () {
    mode = button.getAttribute('data-mode');
    setStatus(mode === 'left' ? 'Tap' : mode);
  });
});
img.addEventListener('click', function (ev) {
  if (Date.now() < ignoreUntil || !img.naturalWidth) return;
  const body = point(ev);
  body.op = 'click';
  body.button = mode === 'right' ? 'right' : 'left';
  body.double = mode === 'double';
  post(body);
});
img.addEventListener('wheel', function (ev) {
  ev.preventDefault();
  pending += ev.deltaY;
  if (!scrollTimer) scrollTimer = setTimeout(flushScroll, 80);
}, { passive: false });
img.addEventListener('touchstart', function (ev) {
  if (ev.touches.length === 1) touchY = ev.touches[0].clientY;
}, { passive: true });
img.addEventListener('touchmove', function (ev) {
  if (touchY === null || ev.touches.length !== 1) return;
  const dy = touchY - ev.touches[0].clientY;
  if (Math.abs(dy) < 24) return;
  ev.preventDefault();
  touchY = ev.touches[0].clientY;
  ignoreUntil = Date.now() + 400;
  pending += dy;
  if (!scrollTimer) scrollTimer = setTimeout(flushScroll, 80);
}, { passive: false });
function flushScroll() {
  scrollTimer = null;
  const dy = Math.max(-40, Math.min(40, Math.round(-pending / 120)));
  pending = 0;
  if (!dy || !img.naturalWidth) return;
  post({
    op: 'scroll',
    x: img.naturalWidth / 2,
    y: img.naturalHeight / 2,
    image_width: img.naturalWidth,
    image_height: img.naturalHeight,
    dx: 0,
    dy: dy
  });
}
function sendText() {
  const field = document.getElementById('typed');
  const text = field.value;
  if (!text) return;
  field.value = '';
  post({ op: 'type', text: text });
}
document.getElementById('send-text').addEventListener('click', sendText);
document.getElementById('typed').addEventListener('keydown', function (ev) {
  if (ev.key === 'Enter') { ev.preventDefault(); sendText(); }
});
document.querySelectorAll('[data-key]').forEach(function (button) {
  button.addEventListener('click', function () {
    post({ op: 'key', key: button.getAttribute('data-key') });
  });
});
document.getElementById('send-combo').addEventListener('click', function () {
  const keys = document.getElementById('combo').value.trim();
  if (keys) post({ op: 'combo', keys: keys });
});
async function poll() {
  try {
    const width = Math.max(1, Math.min(1280, Math.round(window.innerWidth || 1280)));
    const headers = {};
    if (etag) headers['If-None-Match'] = etag;
    const res = await fetch(root + 'frame?w=' + width, { headers: headers, cache: 'no-store' });
    if (res.status === 304) return;
    const next = res.headers.get('ETag');
    if (next) etag = next;
    if (res.status !== 200) return;
    const type = (res.headers.get('Content-Type') || '').toLowerCase();
    if (type.indexOf('image/') !== 0) { await res.text(); return; }
    const blob = await res.blob();
    const url = URL.createObjectURL(blob);
    const previous = img.dataset.blob || '';
    img.src = url;
    img.dataset.blob = url;
    if (previous.indexOf('blob:') === 0) URL.revokeObjectURL(previous);
  } catch (err) {}
}
setInterval(poll, 1500);
poll();
</script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_and_ttl_reject_unspecified_and_out_of_range() {
        assert_eq!(parse_port(None).unwrap(), DEFAULT_PORT);
        assert_eq!(parse_port(Some(&json!(8744))).unwrap(), 8744);
        assert!(parse_port(Some(&json!(0))).is_err());
        assert!(parse_port(Some(&json!(70000))).is_err());
        assert_eq!(parse_ttl(None).unwrap(), None);
        assert_eq!(
            parse_ttl(Some(&json!(15))).unwrap(),
            Some(Duration::from_secs(15 * 60))
        );
        assert!(parse_ttl(Some(&json!(0))).is_err());
        assert!(parse_ttl(Some(&json!(1441))).is_err());
    }

    #[test]
    fn help_message_points_at_the_session_urls() {
        let message = help_message(
            "http://hands-host.example.ts.net:8744/abc/",
            "http://100.64.0.2:8744/abc/",
            "sign-in",
        );
        assert!(message.contains("http://hands-host.example.ts.net:8744/abc/"));
        assert!(message.contains("http://100.64.0.2:8744/abc/"));
        assert!(message.contains("will not type a password"));
        assert!(message.contains("sign-in"));
        assert!(policy::tokens_equal("abc", "abc"));
        assert!(!policy::tokens_equal("abc", "abd"));
        assert!(PAGE.to_ascii_lowercase().contains(policy::ACTION_HEADER));
    }
}
