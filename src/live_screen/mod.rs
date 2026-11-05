//! Tailnet-only live screen.
//!
//! An owner opens one page on a phone that is already on the same tailnet,
//! watches a downscaled JPEG of this machine, and can click or type. Hands
//! keeps serving its normal tools while that page is open. The listener binds
//! the machine's Tailscale IPv4 only.

mod http;
mod policy;

#[cfg(windows)]
mod platform;

use policy::{
    authorize, banner_text, combo_is_blocked, parse_action, parse_tailscale_endpoint,
    parse_viewport_width, sanitize_ai_name, sanitize_help_reason, DesktopMap, Gate, GateRequest,
    GateSession, OwnerAction, Route, DEFAULT_PORT, VIEWER_WINDOW_SECS,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const READ_TIMEOUT: Duration = Duration::from_secs(5);

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
    allow_loopback: bool,
    clients: Mutex<Vec<(Option<SocketAddr>, std::net::TcpStream)>>,
    inflight: Mutex<usize>,
    inflight_cv: Condvar,
    last_viewer: Mutex<Option<Instant>>,
    last_action: Mutex<Option<Instant>>,
    action_lock: Mutex<()>,
    frame: Mutex<Option<FrameState>>,
    canned_jpeg: Mutex<Option<Vec<u8>>>,
}

struct Runtime {
    live: Arc<Live>,
    http: JoinHandle<()>,
    timer: Option<JoinHandle<()>>,
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
    let listener = match TcpListener::bind(addr) {
        Ok(listener) => listener,
        Err(_) => {
            return failure(
                "Live screen could not bind that TCP port on the Tailscale address. Pick another port or stop whatever is already using it. It will not fall back to a public address.",
            );
        }
    };
    let live = new_live(
        token,
        port,
        endpoint.ipv4,
        endpoint.magic_dns,
        ttl,
        false,
        None,
    );
    let serve = Arc::clone(&live);
    let http = match thread::Builder::new()
        .name("hands-live-screen".to_string())
        .spawn(move || http::serve(serve, listener))
    {
        Ok(handle) => handle,
        Err(_) => {
            return failure("Live screen could not start its listener thread.");
        }
    };
    let timer = spawn_ttl(Arc::clone(&live));
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
        http::signal_stop(&live);
        drop(slot);
        let _ = http.join();
        if let Some(timer) = timer {
            let _ = timer.join();
        }
        http::wait_idle(&live);
        if let Some(banner) = banner {
            let _ = banner.join();
        }
        return failure(
            "A live screen started while this one was opening. Use live_screen_status.",
        );
    }
    *slot = Some(Runtime {
        live,
        http,
        timer,
        banner,
    });
    payload
}

fn new_live(
    token: String,
    port: u16,
    bind_ip: IpAddr,
    magic_dns: Option<String>,
    ttl: Option<Duration>,
    allow_loopback: bool,
    canned_jpeg: Option<Vec<u8>>,
) -> Arc<Live> {
    Arc::new(Live {
        token,
        port,
        bind_ip,
        magic_dns,
        started: Instant::now(),
        ttl,
        stop: AtomicBool::new(false),
        allow_loopback,
        clients: Mutex::new(Vec::new()),
        inflight: Mutex::new(0),
        inflight_cv: Condvar::new(),
        last_viewer: Mutex::new(None),
        last_action: Mutex::new(None),
        action_lock: Mutex::new(()),
        frame: Mutex::new(None),
        canned_jpeg: Mutex::new(canned_jpeg),
    })
}

fn spawn_ttl(live: Arc<Live>) -> Option<JoinHandle<()>> {
    let ttl = live.ttl?;
    let started = live.started;
    thread::Builder::new()
        .name("hands-live-ttl".to_string())
        .spawn(move || {
            let deadline = started + ttl;
            while Instant::now() < deadline {
                if live.stop.load(Ordering::SeqCst) {
                    return;
                }
                let slice = Duration::from_millis(50)
                    .min(deadline.saturating_duration_since(Instant::now()));
                if slice.is_zero() {
                    break;
                }
                thread::sleep(slice);
            }
            if !live.stop.load(Ordering::SeqCst) {
                http::signal_stop(&live);
            }
        })
        .ok()
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
        http::signal_stop(&runtime.live);
        let _ = runtime.http.join();
        if let Some(timer) = runtime.timer {
            let _ = timer.join();
        }
        http::wait_idle(&runtime.live);
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

struct Incoming<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub host: Option<&'a str>,
    pub origin: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub action_header: Option<&'a str>,
    pub if_none_match: Option<&'a str>,
    pub content_length: Option<usize>,
    pub chunked: bool,
}

struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub extra: Vec<(&'static str, String)>,
    pub count_viewer: bool,
}

enum Plan {
    Reply(Reply),
    ReadBody { length: usize },
}

fn plan(live: &Live, incoming: &Incoming<'_>, client_ip: Option<IpAddr>) -> Plan {
    if incoming.chunked {
        return Plan::Reply(json_reply(400, "bad_input", false));
    }
    match authorize(
        &GateRequest {
            method: incoming.method,
            path: incoming.path,
            client_ip,
            host: incoming.host,
            origin: incoming.origin,
            content_type: incoming.content_type,
            action_header: incoming.action_header,
            body_len: incoming.content_length.unwrap_or(0),
            content_length: incoming.content_length,
        },
        GateSession {
            token: &live.token,
            bind_ip: live.bind_ip,
            magic_dns: live.magic_dns.as_deref(),
            port: live.port,
            allow_loopback: live.allow_loopback,
        },
    ) {
        Gate::Reject(status) => Plan::Reply(json_reply(status, status_error(status), false)),
        Gate::Allow(Route::Action) => match incoming.content_length {
            Some(length) => Plan::ReadBody { length },
            None => Plan::Reply(json_reply(400, "bad_input", false)),
        },
        Gate::Allow(route) => Plan::Reply(reply_route(live, route, incoming)),
    }
}

fn reply_with_body(
    live: &Live,
    incoming: &Incoming<'_>,
    client_ip: Option<IpAddr>,
    body: &[u8],
) -> Reply {
    if !matches!(plan(live, incoming, client_ip), Plan::ReadBody { .. }) {
        return json_reply(400, "bad_input", false);
    }
    action_reply(live, body)
}

fn reply_route(live: &Live, route: Route, incoming: &Incoming<'_>) -> Reply {
    match route {
        Route::Page => Reply {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: PAGE.as_bytes().to_vec(),
            extra: Vec::new(),
            count_viewer: true,
        },
        Route::Frame => frame_reply(live, incoming.query, incoming.if_none_match),
        Route::Action => json_reply(400, "bad_input", false),
    }
}

fn frame_reply(live: &Live, query: &str, if_none_match: Option<&str>) -> Reply {
    let viewport = match parse_viewport_width(query) {
        Ok(width) => width,
        Err(()) => return json_reply(400, "bad_input", false),
    };
    let (jpeg, width, height, map) = match load_frame(live, viewport) {
        Ok(frame) => frame,
        Err(()) => return json_reply(500, "unexpected", false),
    };
    let etag = frame_etag(&jpeg);
    if let Ok(mut guard) = live.frame.lock() {
        *guard = Some(FrameState { width, height, map });
    }
    if policy::if_none_match_hits(if_none_match, &etag) {
        Reply {
            status: 304,
            content_type: "text/plain; charset=utf-8",
            body: Vec::new(),
            extra: vec![("ETag", etag)],
            count_viewer: true,
        }
    } else {
        Reply {
            status: 200,
            content_type: "image/jpeg",
            body: jpeg,
            extra: vec![("ETag", etag)],
            count_viewer: true,
        }
    }
}

fn action_reply(live: &Live, body: &[u8]) -> Reply {
    let value = match serde_json::from_slice::<Value>(body) {
        Ok(value) => value,
        Err(_) => return json_reply(400, "bad_input", false),
    };
    let action = match parse_action(&value) {
        Ok(action) => action,
        Err(fault) => {
            return json_reply(fault.status(), status_error(fault.status()), false);
        }
    };
    let _guard = match live.action_lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return json_reply(409, "busy", false),
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
    };
    live.note_action();
    match perform(live, action) {
        Ok(()) => json_body(200, br#"{"ok":true}"#.to_vec(), false),
        Err(ActionResult::BadInput) => json_reply(400, "bad_input", false),
        Err(ActionResult::Blocked) => json_reply(409, "blocked", false),
        Err(ActionResult::FocusChanged(typed)) => json_body(
            409,
            serde_json::to_vec(&json!({"error":"focus_changed","typed": typed}))
                .unwrap_or_default(),
            false,
        ),
        Err(ActionResult::Rejected { typed }) => json_body(
            409,
            serde_json::to_vec(
                &json!({"error":"input_rejected","reason":"elevated_window","typed": typed}),
            )
            .unwrap_or_default(),
            false,
        ),
        Err(ActionResult::Unexpected) => json_reply(500, "unexpected", false),
    }
}

fn json_reply(status: u16, error: &str, count_viewer: bool) -> Reply {
    let body = if error == "ok_true" {
        br#"{"ok":true}"#.to_vec()
    } else {
        format!(r#"{{"error":"{error}"}}"#).into_bytes()
    };
    json_body(status, body, count_viewer)
}

fn json_body(status: u16, body: Vec<u8>, count_viewer: bool) -> Reply {
    Reply {
        status,
        content_type: "application/json; charset=utf-8",
        body,
        extra: Vec::new(),
        count_viewer,
    }
}

fn status_error(status: u16) -> &'static str {
    match status {
        413 => "too_large",
        400 => "bad_input",
        409 => "blocked",
        _ => "rejected",
    }
}

fn load_frame(live: &Live, viewport: u32) -> Result<(Vec<u8>, u32, u32, DesktopMap), ()> {
    if let Some(bytes) = live.canned_jpeg.lock().ok().and_then(|guard| guard.clone()) {
        let map = policy::place_monitors(&[policy::MonitorGeometry {
            logical_x: 0,
            logical_y: 0,
            logical_w: 1,
            logical_h: 1,
            capture_w: 1,
            capture_h: 1,
            scale: 1.0,
        }]);
        return Ok((bytes, 1, 1, map));
    }
    #[cfg(windows)]
    {
        platform::capture_jpeg(viewport)
            .map(|frame| (frame.jpeg, frame.width, frame.height, frame.map))
    }
    #[cfg(not(windows))]
    {
        let _ = viewport;
        Err(())
    }
}

enum ActionResult {
    BadInput,
    Blocked,
    FocusChanged(usize),
    Rejected { typed: usize },
    Unexpected,
}

fn input_fault(fault: platform::InputFault) -> ActionResult {
    match fault {
        platform::InputFault::Rejected => ActionResult::Rejected { typed: 0 },
        platform::InputFault::Unexpected => ActionResult::Unexpected,
    }
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
                platform::click_at(screen_x, screen_y, button, double).map_err(input_fault)
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
                platform::scroll_at(screen_x, screen_y, dx, dy).map_err(input_fault)
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
                    Err(platform::TypeStop::Rejected(typed)) => {
                        Err(ActionResult::Rejected { typed })
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
                platform::tap_key(name).map_err(input_fault)
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
                platform::tap_combo(&combo).map_err(input_fault)
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

fn frame_etag(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("\"{hex}\"")
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
    else if (payload.error === 'input_rejected') setStatus('That window blocked the input. Hands cannot drive an elevated or UAC prompt unless it is running elevated.');
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
    if (res.status === 304) return; // keep the image already shown
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
