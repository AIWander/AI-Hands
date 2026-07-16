//! Browser manager - chromiumoxide wrapper with extras
// NAV: TOC at line 1356 | 3 fn | 3 struct | 2026-04-14
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::input::{DispatchKeyEventParams, DispatchKeyEventType};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::Page;
use futures::StreamExt;
use serde_json;
use std::collections::HashMap;
use std::fmt;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Debug)]
pub enum BrowserError {
    NotLaunched,
    NoPage,
    Cdp(String),
    Timeout(String),
    ElementNotFound(String),
    ProcessError(String),
}

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLaunched => write!(f, "Browser not launched"),
            Self::NoPage => write!(f, "No active page"),
            Self::Cdp(s) => write!(f, "CDP error: {}", s),
            Self::Timeout(s) => write!(f, "Timeout: {}", s),
            Self::ElementNotFound(s) => write!(f, "Element not found: {}", s),
            Self::ProcessError(s) => write!(f, "Process error: {}", s),
        }
    }
}

impl std::error::Error for BrowserError {}

/// A network route rule for request interception
#[derive(Clone, Debug)]
pub struct RouteRule {
    pub pattern: String,
    pub action: RouteAction,
}

#[derive(Clone, Debug)]
pub enum RouteAction {
    /// Block the request entirely
    Block,
    /// Return a mock response
    Mock {
        status: u16,
        content_type: String,
        body: String,
    },
    /// Just log the request (passthrough)
    Log,
}

fn serialize_route_rules(routes: &[RouteRule]) -> Result<String, serde_json::Error> {
    let values = routes
        .iter()
        .map(|route| {
            let action = match &route.action {
                RouteAction::Block => serde_json::json!({"type": "block"}),
                RouteAction::Mock {
                    status,
                    content_type,
                    body,
                } => serde_json::json!({
                    "type": "mock",
                    "status": status,
                    "contentType": content_type,
                    "body": body,
                }),
                RouteAction::Log => serde_json::json!({"type": "log"}),
            };
            serde_json::json!({"pattern": route.pattern, "action": action})
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&values)
}

fn js_string(raw: &str) -> String {
    serde_json::to_string(raw).expect("JSON string serialization cannot fail")
}

const SAFE_ROUTE_LITERALS: &[&str] = &[
    "api", "rest", "rpc", "graphql", "v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8", "v9", "auth",
    "oauth", "login", "logout", "callback", "users", "user", "accounts", "account", "orders",
    "order", "events", "webhooks", "health", "status", "assets", "static",
];

fn validate_route_pattern(pattern: &str) -> Result<String, BrowserError> {
    let trimmed = pattern.trim();
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.is_empty()
        || trimmed.len() > 2_048
        || trimmed.chars().any(char::is_control)
        || trimmed
            .chars()
            .any(|ch| matches!(ch, '?' | '#' | '@' | '\\' | '=' | '&' | ';'))
        || lower.contains("://")
        || lower.contains(':')
        || [
            "authorization",
            "bearer ",
            "basic ",
            "password",
            "secret",
            "token",
            "api_key",
            "api-key",
            "apikey",
            "cookie",
            "session",
            "credential",
            "one-time-code",
            "otp",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return Err(BrowserError::Cdp(
            "Route pattern must be a credential-free path-shape glob".into(),
        ));
    }
    if !trimmed.starts_with('/') && !trimmed.starts_with("**/") {
        return Err(BrowserError::Cdp(
            "Route pattern must start with '/' or '**/'".into(),
        ));
    }
    if trimmed.split('/').any(|segment| {
        let segment = segment.trim();
        !segment.is_empty()
            && !segment.chars().all(|ch| ch == '*')
            && !SAFE_ROUTE_LITERALS.contains(&segment.to_ascii_lowercase().as_str())
    }) {
        return Err(BrowserError::Cdp(
            "Route pattern contains an arbitrary path value; replace dynamic segments with '*'"
                .into(),
        ));
    }
    Ok(lower)
}

fn route_action_label(action: &RouteAction) -> &'static str {
    match action {
        RouteAction::Block => "block",
        RouteAction::Mock { .. } => "mock",
        RouteAction::Log => "log",
    }
}

/// Named browser context — isolated page group with separate state
pub struct BrowserContext {
    pub pages: Vec<Page>,
    pub active_page_index: usize,
}

pub struct BrowserManager {
    browser: Option<Browser>,
    pages: Vec<Page>,
    active_page_index: usize,
    headless: bool,
    current_url: String,
    /// Network interception routes (pattern → action)
    routes: Vec<RouteRule>,
    /// Whether network interception JS is injected on current page
    interception_active: bool,
    /// Named browser contexts for isolated sessions
    contexts: HashMap<String, BrowserContext>,
    /// Active context name (None = default)
    active_context: Option<String>,
    /// Trace recording state
    trace_active: bool,
    trace_entries: Vec<serde_json::Value>,
}

/// Check if a process is still alive by PID (Windows)
#[cfg(windows)]
fn is_pid_alive(pid: u32) -> bool {
    use std::process::Command as StdCommand;
    StdCommand::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/NH"])
        .output()
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            out.contains(&pid.to_string())
        })
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}

impl BrowserManager {
    pub fn new() -> Self {
        Self {
            browser: None,
            pages: Vec::new(),
            active_page_index: 0,
            headless: true,
            current_url: String::new(),
            routes: Vec::new(),
            interception_active: false,
            contexts: HashMap::new(),
            active_context: None,
            trace_active: false,
            trace_entries: Vec::new(),
        }
    }

    /// Launch Chrome with remote debugging enabled - for authenticated sessions
    /// This starts YOUR Chrome (with all your logins), then you use browser_attach to connect.
    /// When `wait_for_cdp` is true (default), polls the CDP endpoint until Chrome is ready.
    pub async fn debug_launch(
        port: u16,
        url: Option<&str>,
        wait_for_cdp: bool,
    ) -> Result<String, BrowserError> {
        // Find Chrome - build paths as Strings
        let localappdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let chrome_paths: Vec<String> = vec![
            r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".to_string(),
            format!(r"{}\Google\Chrome\Application\chrome.exe", localappdata),
        ];

        let chrome_path = chrome_paths
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .ok_or_else(|| BrowserError::ProcessError("Chrome not found".into()))?;

        let target_url = url.unwrap_or("about:blank");

        // Use a dedicated user-data-dir so debug Chrome forks a NEW process
        // even when the user's default Chrome is already running.
        let debug_profile = format!(
            "{}\\CPC\\chrome-debug-profile",
            std::env::var("LOCALAPPDATA")
                .unwrap_or_else(|_| r"C:\Users\Default\AppData\Local".into())
        );
        let _ = std::fs::create_dir_all(&debug_profile);

        // Launch Chrome with debug port + isolated profile
        let mut cmd = Command::new(chrome_path);
        cmd.arg(format!("--remote-debugging-port={}", port))
            .arg(format!("--user-data-dir={}", debug_profile))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg(target_url);

        let child = cmd
            .spawn()
            .map_err(|e| BrowserError::ProcessError(format!("Failed to launch Chrome: {}", e)))?;
        let pid = child.id();

        if !wait_for_cdp {
            return Ok(format!(
                "Chrome launched with debug port {}. URL: {}. Use browser_attach({}) to connect.",
                port, target_url, port
            ));
        }

        // Poll CDP endpoint until Chrome is ready
        let cdp_url = format!("http://localhost:{}/json/version", port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .map_err(|e| BrowserError::ProcessError(format!("HTTP client error: {}", e)))?;

        let start = std::time::Instant::now();
        let deadline = Duration::from_secs(10);
        let poll_interval = Duration::from_millis(150);

        loop {
            // Check if process is still alive
            let process_alive = is_pid_alive(pid);
            let elapsed = start.elapsed();

            if elapsed >= deadline {
                return Err(BrowserError::Timeout(format!(
                    "Chrome did not start CDP on port {} after {}ms (process alive: {})",
                    port,
                    elapsed.as_millis(),
                    process_alive
                )));
            }

            if !process_alive {
                return Err(BrowserError::ProcessError(format!(
                    "Chrome process (pid {}) died before CDP was ready on port {} (after {}ms)",
                    pid,
                    port,
                    elapsed.as_millis()
                )));
            }

            // Try to hit the CDP version endpoint
            if let Ok(resp) = client.get(&cdp_url).send().await {
                if resp.status().is_success() {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        if body.get("Browser").is_some() {
                            let ms = start.elapsed().as_millis();
                            return Ok(format!(
                                "Chrome launched with debug port {} (CDP ready in {}ms). URL: {}. Use browser_attach({}) to connect.",
                                port, ms, target_url, port
                            ));
                        }
                    }
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Connect to an existing Chrome instance via CDP
    pub async fn attach(&mut self, port: u16) -> Result<String, BrowserError> {
        let url = format!("http://localhost:{}", port);

        let (browser, mut handler) = Browser::connect(&url)
            .await
            .map_err(|e| BrowserError::Cdp(format!("Connect to {} failed: {}", url, e)))?;

        tokio::spawn(async move { while let Some(_) = handler.next().await {} });

        // Get existing pages or create new one
        let existing_pages = browser
            .pages()
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        let mut all_pages: Vec<Page> = existing_pages.into_iter().collect();
        if all_pages.is_empty() {
            let page = browser
                .new_page("about:blank")
                .await
                .map_err(|e| BrowserError::Cdp(e.to_string()))?;
            all_pages.push(page);
        }

        let current_url = all_pages[0]
            .url()
            .await
            .unwrap_or_default()
            .unwrap_or_default();

        self.browser = Some(browser);
        self.pages = all_pages;
        self.active_page_index = 0;
        self.headless = false;
        self.current_url = current_url.clone();

        Ok(current_url)
    }

    pub async fn launch(
        &mut self,
        headless: bool,
        profile_path: Option<String>,
    ) -> Result<(), BrowserError> {
        let mut builder = BrowserConfig::builder();

        if headless {
            builder = builder.arg("--headless=new");
        }

        if let Some(profile) = profile_path {
            builder = builder.arg(format!("--user-data-dir={}", profile));
        }

        builder = builder
            .arg("--disable-blink-features=AutomationControlled")
            .arg("--no-first-run")
            .arg("--no-default-browser-check");

        let config = builder
            .build()
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        tokio::spawn(async move { while let Some(_) = handler.next().await {} });

        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        self.browser = Some(browser);
        self.pages = vec![page];
        self.active_page_index = 0;
        self.headless = headless;

        Ok(())
    }

    pub async fn close(&mut self) -> Result<(), BrowserError> {
        self.pages.clear();
        self.active_page_index = 0;
        self.browser = None;
        Ok(())
    }

    /// Check if the browser is still responding
    pub async fn is_alive(&self) -> bool {
        match self.pages.get(self.active_page_index) {
            Some(page) => page.url().await.is_ok(),
            None => false,
        }
    }

    /// Ensure browser is alive, auto-recover if crashed
    pub async fn ensure_alive(&mut self) -> Result<(), BrowserError> {
        if !self.is_alive().await {
            if self.browser.is_some() {
                // Browser was running but crashed - recover
                eprintln!("[browser-mcp] Browser crashed, recovering...");
                self.pages.clear();
                self.active_page_index = 0;
                self.browser = None;
            }
            // Relaunch with last known settings
            self.launch(self.headless, None).await?;
        }
        Ok(())
    }

    fn page(&self) -> Result<&Page, BrowserError> {
        self.pages
            .get(self.active_page_index)
            .ok_or(BrowserError::NoPage)
    }

    pub async fn navigate(&mut self, url: &str, _wait_until: &str) -> Result<String, BrowserError> {
        self.current_url = url.to_string();

        let page = self.page()?;
        page.goto(url)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        let title = page
            .get_title()
            .await
            .unwrap_or_default()
            .unwrap_or_default();
        if !self.routes.is_empty() {
            self.inject_interception().await?;
        }
        Ok(title)
    }

    pub async fn click_selector(&self, selector: &str) -> Result<(), BrowserError> {
        let page = self.page()?;
        let el = page
            .find_element(selector)
            .await
            .map_err(|_| BrowserError::ElementNotFound(selector.into()))?;
        // Wrap click in timeout: if the click triggers cross-origin navigation,
        // chromiumoxide may wait for page load which times out. The click itself
        // was dispatched successfully — treat timeout as success.
        match tokio::time::timeout(Duration::from_secs(5), el.click()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(BrowserError::Cdp(e.to_string())),
            Err(_) => Ok(()), // timeout after click dispatched = navigation side-effect
        }
    }

    pub async fn click_coords(&self, x: i32, y: i32) -> Result<(), BrowserError> {
        let script = format!("document.elementFromPoint({}, {})?.click()", x, y);
        // Wrap in timeout: JS click may trigger navigation that blocks evaluate
        match tokio::time::timeout(Duration::from_secs(5), self.evaluate(&script)).await {
            Ok(result) => {
                result?;
                Ok(())
            }
            Err(_) => Ok(()), // timeout after click dispatched = navigation side-effect
        }
    }

    pub async fn type_text(
        &self,
        selector: &str,
        text: &str,
        clear: bool,
    ) -> Result<(), BrowserError> {
        let page = self.page()?;
        let el = page
            .find_element(selector)
            .await
            .map_err(|_| BrowserError::ElementNotFound(selector.into()))?;

        if clear {
            let clear_script =
                format!("document.querySelector({}).value = ''", js_string(selector));
            self.evaluate(&clear_script).await.ok();
        }

        el.type_str(text)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        Ok(())
    }

    pub async fn press_key(&self, key: &str) -> Result<(), BrowserError> {
        let page = self.page()?;

        // Map common key names to CDP key codes
        let (key_code, _text) = match key.to_lowercase().as_str() {
            "enter" | "return" => ("Enter", "\r"),
            "tab" => ("Tab", "\t"),
            "escape" | "esc" => ("Escape", ""),
            "backspace" => ("Backspace", ""),
            "delete" => ("Delete", ""),
            "arrowup" | "up" => ("ArrowUp", ""),
            "arrowdown" | "down" => ("ArrowDown", ""),
            "arrowleft" | "left" => ("ArrowLeft", ""),
            "arrowright" | "right" => ("ArrowRight", ""),
            "space" => ("Space", " "),
            _ => (key, key),
        };

        // Send keydown
        let cmd = DispatchKeyEventParams::builder()
            .r#type(DispatchKeyEventType::KeyDown)
            .key(key_code)
            .build()
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        page.execute(cmd)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        // Send keyup
        let cmd = DispatchKeyEventParams::builder()
            .r#type(DispatchKeyEventType::KeyUp)
            .key(key_code)
            .build()
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        page.execute(cmd)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        Ok(())
    }

    pub async fn screenshot(&self, full_page: bool, quality: u8) -> Result<String, BrowserError> {
        let page = self.page()?;

        let params = chromiumoxide::page::ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Jpeg)
            .quality(quality as i64)
            .full_page(full_page)
            .build();

        let bytes = page
            .screenshot(params)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &bytes,
        ))
    }

    /// Screenshot and save directly to file (0 tokens - returns filepath only)
    pub async fn screenshot_to_file(
        &self,
        path: &str,
        full_page: bool,
        quality: u8,
    ) -> Result<String, BrowserError> {
        let page = self.page()?;

        let params = chromiumoxide::page::ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Jpeg)
            .quality(quality as i64)
            .full_page(full_page)
            .build();

        let bytes = page
            .screenshot(params)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        std::fs::write(path, &bytes)
            .map_err(|e| BrowserError::ProcessError(format!("Failed to write: {}", e)))?;

        Ok(path.to_string())
    }

    pub async fn screenshot_element(
        &self,
        selector: &str,
        _quality: u8,
    ) -> Result<String, BrowserError> {
        let page = self.page()?;
        let el = page
            .find_element(selector)
            .await
            .map_err(|_| BrowserError::ElementNotFound(selector.into()))?;

        let bytes = el
            .screenshot(CaptureScreenshotFormat::Jpeg)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &bytes,
        ))
    }

    pub async fn wait_for(
        &self,
        selector: &str,
        timeout_ms: u64,
        check_visible: bool,
    ) -> Result<(), BrowserError> {
        let page = self.page()?;
        let timeout = Duration::from_millis(timeout_ms);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            // Check existence
            if page.find_element(selector).await.is_ok() {
                if !check_visible {
                    return Ok(());
                }

                // Check CSS visibility (display, visibility, opacity, offsetParent)
                let visible = page
                    .evaluate(format!(
                        "(() => {{ \
                        const el = document.querySelector({}); \
                        if (!el) return false; \
                        const style = window.getComputedStyle(el); \
                        return style.display !== 'none' && \
                               style.visibility !== 'hidden' && \
                               style.opacity !== '0' && \
                               el.offsetParent !== null; \
                    }})()",
                        js_string(selector)
                    ))
                    .await
                    .ok()
                    .and_then(|v| v.into_value::<bool>().ok())
                    .unwrap_or(false);

                if visible {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Err(BrowserError::Timeout(format!(
            "Waiting for {}{}",
            selector,
            if check_visible { " (visible)" } else { "" }
        )))
    }

    pub async fn get_html(&self, selector: Option<&str>) -> Result<String, BrowserError> {
        let page = self.page()?;

        if let Some(sel) = selector {
            let el = page
                .find_element(sel)
                .await
                .map_err(|_| BrowserError::ElementNotFound(sel.into()))?;
            el.outer_html()
                .await
                .map_err(|e| BrowserError::Cdp(e.to_string()))?
                .ok_or_else(|| BrowserError::Cdp("No HTML".into()))
        } else {
            page.content()
                .await
                .map_err(|e| BrowserError::Cdp(e.to_string()))
        }
    }

    pub async fn get_text(&self, selector: &str) -> Result<String, BrowserError> {
        let page = self.page()?;
        let el = page
            .find_element(selector)
            .await
            .map_err(|_| BrowserError::ElementNotFound(selector.into()))?;
        el.inner_text()
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?
            .ok_or_else(|| BrowserError::Cdp("No text".into()))
    }

    pub async fn evaluate(&self, script: &str) -> Result<serde_json::Value, BrowserError> {
        let page = self.page()?;
        let result = page
            .evaluate(script)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        Ok(result.into_value().unwrap_or(serde_json::Value::Null))
    }

    /// Resolve an XPath expression to a CSS-usable reference via JS.
    /// Returns a unique selector string or clicks/types by evaluating directly.
    pub async fn resolve_xpath(&self, xpath: &str) -> Result<serde_json::Value, BrowserError> {
        let script = format!(
            r#"(() => {{
                const result = document.evaluate({}, document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null);
                const el = result.singleNodeValue;
                if (!el) return null;
                const rect = el.getBoundingClientRect();
                // Try to build a unique selector
                let sel = null;
                if (el.id) sel = '#' + CSS.escape(el.id);
                else if (el.name) sel = '[name="' + el.name.replace(/"/g, '\\"') + '"]';
                return {{
                    found: true,
                    tag: el.tagName,
                    text: (el.innerText || '').trim().slice(0, 200),
                    selector: sel,
                    cx: Math.round(rect.x + rect.width/2),
                    cy: Math.round(rect.y + rect.height/2),
                    width: Math.round(rect.width),
                    height: Math.round(rect.height)
                }};
            }})()"#,
            js_string(xpath)
        );
        let val = self.evaluate(&script).await?;
        if val.is_null() {
            return Err(BrowserError::ElementNotFound(format!("xpath: {}", xpath)));
        }
        Ok(val)
    }

    /// Click an element found by XPath
    pub async fn click_xpath(&self, xpath: &str) -> Result<String, BrowserError> {
        let el = self.resolve_xpath(xpath).await?;
        let sel = el.get("selector").and_then(|v| v.as_str()).unwrap_or("");
        let text = el.get("text").and_then(|v| v.as_str()).unwrap_or("?");
        if !sel.is_empty() {
            self.click_selector(sel).await?;
            Ok(format!("Clicked '{}' via {}", text, sel))
        } else {
            let cx = el.get("cx").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let cy = el.get("cy").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            self.click_coords(cx, cy).await?;
            Ok(format!("Clicked '{}' at ({},{})", text, cx, cy))
        }
    }

    /// Type text into an element found by XPath
    pub async fn type_xpath(
        &self,
        xpath: &str,
        text: &str,
        clear: bool,
    ) -> Result<String, BrowserError> {
        let el = self.resolve_xpath(xpath).await?;
        let sel = el.get("selector").and_then(|v| v.as_str()).unwrap_or("");
        if !sel.is_empty() {
            self.type_text(sel, text, clear).await?;
            Ok(format!("Typed into xpath match via {}", sel))
        } else {
            // Focus via click then type via keyboard
            let cx = el.get("cx").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let cy = el.get("cy").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            self.click_coords(cx, cy).await?;
            // Clear existing content if requested
            if clear {
                self.evaluate("document.activeElement.value = ''")
                    .await
                    .ok();
            }
            // Type char by char via CDP key events
            let page = self.page()?;
            for ch in text.chars() {
                let key_event = DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyDown)
                    .text(ch.to_string())
                    .build()
                    .map_err(|e| BrowserError::Cdp(e.to_string()))?;
                page.execute(key_event)
                    .await
                    .map_err(|e| BrowserError::Cdp(e.to_string()))?;
            }
            Ok(format!("Typed into xpath match at ({},{})", cx, cy))
        }
    }

    /// Wait for an XPath expression to match an element
    pub async fn wait_for_xpath(
        &self,
        xpath: &str,
        timeout_ms: u64,
        check_visible: bool,
    ) -> Result<(), BrowserError> {
        let timeout = Duration::from_millis(timeout_ms);
        let start = std::time::Instant::now();
        let xpath_js = js_string(xpath);

        while start.elapsed() < timeout {
            let script = format!(
                r#"(() => {{
                    const result = document.evaluate({}, document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null);
                    const el = result.singleNodeValue;
                    if (!el) return {{ found: false, visible: false }};
                    const style = window.getComputedStyle(el);
                    const visible = style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0' && el.offsetParent !== null;
                    return {{ found: true, visible: visible }};
                }})()"#,
                xpath_js
            );
            if let Ok(val) = self.evaluate(&script).await {
                let found = val.get("found").and_then(|v| v.as_bool()).unwrap_or(false);
                let visible = val
                    .get("visible")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if found && (!check_visible || visible) {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(BrowserError::Timeout(format!(
            "Waiting for xpath: {}{}",
            xpath,
            if check_visible { " (visible)" } else { "" }
        )))
    }

    /// Auto-wait: wait for selector/xpath to be ready before interacting
    pub async fn auto_wait_selector(
        &self,
        selector: &str,
        timeout_ms: u64,
    ) -> Result<(), BrowserError> {
        self.wait_for(selector, timeout_ms, true).await
    }

    pub async fn auto_wait_xpath(&self, xpath: &str, timeout_ms: u64) -> Result<(), BrowserError> {
        self.wait_for_xpath(xpath, timeout_ms, true).await
    }

    pub async fn scroll(&self, direction: &str, amount: i32) -> Result<(), BrowserError> {
        let (x, y) = match direction {
            "up" => (0, -amount),
            "down" => (0, amount),
            "left" => (-amount, 0),
            "right" => (amount, 0),
            _ => (0, amount),
        };

        self.evaluate(&format!("window.scrollBy({}, {})", x, y))
            .await?;
        Ok(())
    }

    pub async fn get_element_bounds(
        &self,
        selector: &str,
    ) -> Result<(f64, f64, f64, f64), BrowserError> {
        let script = format!(
            r#"(() => {{
                const el = document.querySelector({});
                if (!el) return null;
                const r = el.getBoundingClientRect();
                return [r.x, r.y, r.width, r.height];
            }})()"#,
            js_string(selector)
        );

        let result = self.evaluate(&script).await?;
        if let Some(arr) = result.as_array() {
            if arr.len() == 4 {
                return Ok((
                    arr[0].as_f64().unwrap_or(0.0),
                    arr[1].as_f64().unwrap_or(0.0),
                    arr[2].as_f64().unwrap_or(0.0),
                    arr[3].as_f64().unwrap_or(0.0),
                ));
            }
        }
        Err(BrowserError::ElementNotFound(selector.into()))
    }

    pub async fn get_element_center(&self, selector: &str) -> Result<(i32, i32), BrowserError> {
        let (x, y, w, h) = self.get_element_bounds(selector).await?;
        Ok(((x + w / 2.0) as i32, (y + h / 2.0) as i32))
    }

    pub async fn select_option(&self, selector: &str, value: &str) -> Result<(), BrowserError> {
        let script = format!(
            r#"document.querySelector({}).value = {};
               document.querySelector({}).dispatchEvent(new Event('change'));"#,
            js_string(selector),
            js_string(value),
            js_string(selector)
        );
        self.evaluate(&script).await?;
        Ok(())
    }

    pub async fn get_cookies(&self) -> Result<Vec<serde_json::Value>, BrowserError> {
        let page = self.page()?;
        // Try chromiumoxide typed API first
        match page.get_cookies().await {
            Ok(cookies) => Ok(cookies
                .into_iter()
                .map(|c| {
                    serde_json::json!({
                        "name": c.name,
                        "value": c.value,
                        "domain": c.domain,
                    })
                })
                .collect()),
            Err(_) => {
                // Fallback: Chrome periodically adds/removes cookie fields (e.g. sameParty)
                // which breaks chromiumoxide's typed deserialization. Use JS fallback.
                // Note: document.cookie omits httpOnly cookies.
                let result = self.evaluate(
                    "document.cookie.split('; ').filter(c => c.length > 0).map(c => { const i = c.indexOf('='); return { name: c.substring(0, i), value: c.substring(i + 1), domain: location.hostname }; })"
                ).await?;
                Ok(result.as_array().cloned().unwrap_or_default())
            }
        }
    }

    pub async fn set_cookie(
        &self,
        name: &str,
        value: &str,
        domain: &str,
    ) -> Result<(), BrowserError> {
        let page = self.page()?;
        page.set_cookie(
            chromiumoxide::cdp::browser_protocol::network::CookieParam::builder()
                .name(name)
                .value(value)
                .domain(domain)
                .build()
                .map_err(|e| BrowserError::Cdp(e.to_string()))?,
        )
        .await
        .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        Ok(())
    }

    pub async fn clear_cookies(&self) -> Result<(), BrowserError> {
        let page = self.page()?;
        page.delete_cookies(vec![])
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        Ok(())
    }

    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.browser.is_some(),
            "headless": self.headless,
            "current_url": self.current_url,
            "tab_count": self.pages.len(),
            "active_tab": self.active_page_index,
        })
    }

    pub async fn new_page(&mut self) -> Result<(), BrowserError> {
        let browser = self.browser.as_ref().ok_or(BrowserError::NotLaunched)?;
        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        self.pages.push(page);
        self.active_page_index = self.pages.len() - 1;
        Ok(())
    }

    pub async fn list_tabs(&self) -> Result<Vec<serde_json::Value>, BrowserError> {
        let mut tabs = Vec::new();
        for (idx, page) in self.pages.iter().enumerate() {
            let url = page.url().await.unwrap_or_default().unwrap_or_default();
            let title = page
                .get_title()
                .await
                .unwrap_or_default()
                .unwrap_or_default();
            tabs.push(serde_json::json!({
                "index": idx,
                "url": url,
                "title": title,
                "active": idx == self.active_page_index,
            }));
        }
        Ok(tabs)
    }

    pub fn switch_tab_by_index(&mut self, index: usize) -> Result<(), BrowserError> {
        if index >= self.pages.len() {
            return Err(BrowserError::Cdp(format!(
                "Tab index {} out of range (0..{})",
                index,
                self.pages.len()
            )));
        }
        self.active_page_index = index;
        Ok(())
    }

    pub async fn switch_tab_by_url(&mut self, url_match: &str) -> Result<usize, BrowserError> {
        let lower = url_match.to_lowercase();
        for (idx, page) in self.pages.iter().enumerate() {
            let url = page.url().await.unwrap_or_default().unwrap_or_default();
            if url.to_lowercase().contains(&lower) {
                self.active_page_index = idx;
                return Ok(idx);
            }
        }
        Err(BrowserError::Cdp(format!(
            "No tab with URL containing '{}'",
            url_match
        )))
    }

    pub async fn close_tab(&mut self, index: usize) -> Result<(), BrowserError> {
        if index >= self.pages.len() {
            return Err(BrowserError::Cdp(format!(
                "Tab index {} out of range (0..{})",
                index,
                self.pages.len()
            )));
        }
        if self.pages.len() == 1 {
            return Err(BrowserError::Cdp("Cannot close the last tab".into()));
        }
        let page = self.pages.remove(index);
        page.close()
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        // Adjust active index
        if self.active_page_index >= self.pages.len() {
            self.active_page_index = self.pages.len() - 1;
        } else if self.active_page_index > index {
            self.active_page_index -= 1;
        }
        Ok(())
    }

    // === EXTRAS beyond Playwright ===

    pub async fn get_clickables(&self) -> Result<Vec<serde_json::Value>, BrowserError> {
        let script = r#"
            Array.from(document.querySelectorAll('a, button, input, [onclick], [role="button"]'))
                .filter(el => {
                    const rect = el.getBoundingClientRect();
                    return rect.width > 0 && rect.height > 0;
                })
                .slice(0, 100)
                .map(el => {
                    const rect = el.getBoundingClientRect();
                    const label = el.labels && el.labels.length ? el.labels[0].innerText : '';
                    return {
                        tag: el.tagName.toLowerCase(),
                        text: (el.innerText || el.getAttribute('aria-label') || label || el.getAttribute('placeholder') || el.getAttribute('name') || el.getAttribute('type') || '').slice(0, 50),
                        x: Math.round(rect.x + rect.width/2),
                        y: Math.round(rect.y + rect.height/2),
                        width: Math.round(rect.width),
                        height: Math.round(rect.height)
                    };
                });
        "#;

        let result = self.evaluate(script).await?;
        if let Some(arr) = result.as_array() {
            Ok(arr.clone())
        } else {
            Ok(vec![])
        }
    }

    pub async fn screenshot_burst(
        &self,
        count: usize,
        interval_ms: u64,
        quality: u8,
    ) -> Result<Vec<String>, BrowserError> {
        let mut shots = Vec::with_capacity(count);
        for _ in 0..count {
            shots.push(self.screenshot(false, quality).await?);
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
        Ok(shots)
    }

    pub async fn hover(&self, selector: &str) -> Result<(), BrowserError> {
        let page = self.page()?;
        let el = page
            .find_element(selector)
            .await
            .map_err(|_| BrowserError::ElementNotFound(selector.into()))?;
        el.scroll_into_view().await.ok();
        Ok(())
    }

    pub async fn focus(&self, selector: &str) -> Result<(), BrowserError> {
        let page = self.page()?;
        let el = page
            .find_element(selector)
            .await
            .map_err(|_| BrowserError::ElementNotFound(selector.into()))?;
        el.focus()
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        Ok(())
    }

    pub async fn exists(&self, selector: &str) -> Result<bool, BrowserError> {
        let page = self.page()?;
        Ok(page.find_element(selector).await.is_ok())
    }

    pub async fn get_metrics(&self) -> Result<serde_json::Value, BrowserError> {
        let script = r#"({
            url: window.location.href,
            title: document.title,
            scrollX: window.scrollX,
            scrollY: window.scrollY,
            innerWidth: window.innerWidth,
            innerHeight: window.innerHeight,
            documentHeight: document.documentElement.scrollHeight
        })"#;
        self.evaluate(script).await
    }

    pub async fn inject_script(&self, script: &str) -> Result<(), BrowserError> {
        self.evaluate(&format!("(function(){{ {} }})()", script))
            .await?;
        Ok(())
    }

    pub async fn get_forms(&self) -> Result<serde_json::Value, BrowserError> {
        let script = r#"
            Array.from(document.forms).map((f, i) => ({
                index: i,
                id: f.id,
                action: f.action,
                method: f.method,
                fields: Array.from(f.elements).filter(e => e.name).map(e => ({
                    name: e.name,
                    type: e.type,
                    id: e.id,
                    autocomplete: e.autocomplete || null,
                    has_value: typeof e.value === 'string' && e.value.length > 0,
                    value_length: typeof e.value === 'string' ? e.value.length : 0
                }))
            }))
        "#;
        self.evaluate(script).await
    }

    pub async fn fill_form(
        &self,
        form_selector: &str,
        data: &serde_json::Value,
    ) -> Result<(), BrowserError> {
        if let Some(obj) = data.as_object() {
            for (name, value) in obj {
                if let Some(v) = value.as_str() {
                    let field_selector = format!("[name={}]", js_string(name));
                    let script = format!(
                        r#"document.querySelector({})?.querySelector({})?.setAttribute('value', {})"#,
                        js_string(form_selector),
                        js_string(&field_selector),
                        js_string(v)
                    );
                    self.evaluate(&script).await.ok();
                }
            }
        }
        Ok(())
    }

    pub async fn submit_form(&self, selector: &str) -> Result<(), BrowserError> {
        let script = format!("document.querySelector({})?.submit()", js_string(selector));
        self.evaluate(&script).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(())
    }

    pub async fn wait_network_idle(&self, timeout_ms: u64) -> Result<(), BrowserError> {
        tokio::time::sleep(Duration::from_millis(timeout_ms.min(5000))).await;
        Ok(())
    }

    pub async fn get_url(&self) -> Result<String, BrowserError> {
        let result = self.evaluate("window.location.href").await?;
        result
            .as_str()
            .map(String::from)
            .ok_or(BrowserError::Cdp("No URL".into()))
    }

    pub async fn go_back(&self) -> Result<(), BrowserError> {
        self.evaluate("window.history.back()").await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(())
    }

    pub async fn go_forward(&self) -> Result<(), BrowserError> {
        self.evaluate("window.history.forward()").await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(())
    }

    pub async fn reload(&self) -> Result<(), BrowserError> {
        self.evaluate("window.location.reload()").await?;
        tokio::time::sleep(Duration::from_millis(1000)).await;
        Ok(())
    }

    // ===== P1: Network Interception (JS-based) =====

    /// Add a network route rule. Injects JS interception on first call.
    pub async fn add_route(
        &mut self,
        pattern: String,
        action: RouteAction,
    ) -> Result<String, BrowserError> {
        let pattern = validate_route_pattern(&pattern)?;
        if let RouteAction::Mock {
            status,
            content_type,
            body,
        } = &action
        {
            if !(100..=599).contains(status)
                || content_type.is_empty()
                || content_type.len() > 128
                || !content_type.chars().all(|ch| {
                    ch.is_ascii_alphanumeric()
                        || matches!(
                            ch,
                            '!' | '#' | '$' | '&' | '^' | '_' | '.' | '+' | '-' | '*' | '/'
                        )
                })
                || body.len() > 1_048_576
            {
                return Err(BrowserError::Cdp(
                    "Mock route requires status 100-599, a safe MIME type, and a body no larger than 1 MiB"
                        .into(),
                ));
            }
        }
        let previous = self.routes.clone();
        self.routes.retain(|route| route.pattern != pattern);
        self.routes.push(RouteRule {
            pattern: pattern.clone(),
            action: action.clone(),
        });
        if let Err(error) = self.inject_interception().await {
            self.routes = previous;
            return Err(error);
        }
        Ok(format!(
            "Route added ({}) with validated path shape; {} total routes",
            route_action_label(&action),
            self.routes.len()
        ))
    }

    /// Remove a route by pattern
    pub async fn remove_route(&mut self, pattern: &str) -> Result<String, BrowserError> {
        let pattern = validate_route_pattern(pattern)?;
        let previous = self.routes.clone();
        let before = self.routes.len();
        self.routes.retain(|r| r.pattern != pattern);
        let removed = before - self.routes.len();
        if self.interception_active {
            if let Err(error) = self.inject_interception().await {
                self.routes = previous;
                return Err(error);
            }
        }
        Ok(format!(
            "Removed {} route(s) matching the validated path shape; {} remaining",
            removed,
            self.routes.len()
        ))
    }

    /// List all active routes
    pub fn list_routes(&self) -> Vec<serde_json::Value> {
        self.routes
            .iter()
            .map(|r| {
                let action_str = match &r.action {
                    RouteAction::Block => "block".to_string(),
                    RouteAction::Mock {
                        status,
                        content_type,
                        ..
                    } => format!("mock({}; {})", status, content_type),
                    RouteAction::Log => "log".to_string(),
                };
                serde_json::json!({"pattern_shape": r.pattern, "action": action_str})
            })
            .collect()
    }

    /// Get intercepted/logged requests from the JS layer
    pub async fn get_intercepted_requests(&self) -> Result<serde_json::Value, BrowserError> {
        self.evaluate("JSON.stringify(window.__mcp_intercepted || [])")
            .await
    }

    /// Clear intercepted request log
    pub async fn clear_intercepted(&self) -> Result<(), BrowserError> {
        self.evaluate(
            "if (window.__mcp_reset_intercepted) { window.__mcp_reset_intercepted(false); } else { window.__mcp_intercepted = []; }",
        )
        .await?;
        Ok(())
    }

    /// Inject the JS interception layer (overrides fetch + XHR)
    async fn inject_interception(&mut self) -> Result<(), BrowserError> {
        let rules_js = serialize_route_rules(&self.routes).map_err(|error| {
            BrowserError::Cdp(format!("Could not serialize route rules: {error}"))
        })?;

        let script = format!(
            r#"(() => {{
            window.__mcp_routes = {rules};
            const safeRouteLiterals = new Set([
                'api', 'rest', 'rpc', 'graphql', 'v1', 'v2', 'v3', 'v4', 'v5', 'v6',
                'v7', 'v8', 'v9', 'auth', 'oauth', 'login', 'logout', 'callback', 'users',
                'user', 'accounts', 'account', 'orders', 'order', 'events', 'webhooks',
                'health', 'status', 'assets', 'static'
            ]);
            const safeQueryKeys = new Set([
                'page', 'limit', 'offset', 'sort', 'order', 'filter', 'q', 'query',
                'search', 'cursor', 'after', 'before', 'fields', 'include', 'expand',
                'lang', 'locale', 'format', 'version', 'id', 'code', 'state', 'token', 'key'
            ]);
            const safeUrlShape = raw => {{
                try {{
                    const parsed = new URL(String(raw), location.href);
                    const path = parsed.pathname.split('/').map(segment => {{
                        if (!segment) return '';
                        let decoded = '';
                        try {{ decoded = decodeURIComponent(segment).toLowerCase(); }}
                        catch (_error) {{ return '{{segment}}'; }}
                        return safeRouteLiterals.has(decoded) ? decoded : '{{segment}}';
                    }}).join('/');
                    const keys = [];
                    parsed.searchParams.forEach((_value, key) => {{
                        const normalized = String(key).toLowerCase();
                        const safeKey = safeQueryKeys.has(normalized) ? normalized : 'parameter';
                        if (!keys.includes(safeKey)) keys.push(safeKey);
                    }});
                    const query = keys.length
                        ? '?' + keys.map(key => encodeURIComponent(key) + '=[REDACTED]').join('&')
                        : '';
                    return path + query + (parsed.hash ? '#[REDACTED]' : '');
                }} catch (_error) {{
                    return '[URL_WITHHELD]';
                }}
            }};
            const routeMatches = (pattern, urlShape) => {{
                const escaped = String(pattern).replace(/[.+?^${{}}()|[\]\\]/g, '\\$&');
                const source = '^' + escaped.replace(/\*+/g, '.*') + '$';
                try {{ return new RegExp(source).test(String(urlShape)); }}
                catch (_error) {{ return false; }}
            }};
            window.__mcp_safe_url_shape = safeUrlShape;
            window.__mcp_reset_intercepted = preserve => {{
                const prior = preserve && window.__mcp_intercepted_safe_v2 === true
                    && Array.isArray(window.__mcp_intercepted)
                    ? window.__mcp_intercepted.slice()
                    : [];
                const guarded = [];
                Object.defineProperty(guarded, 'push', {{
                    configurable: false,
                    writable: false,
                    value: function(...items) {{
                        const safeItems = items.map(item => {{
                            if (!item || typeof item !== 'object') return {{event: '[WITHHELD]'}};
                            return {{...item, url: safeUrlShape(item.url)}};
                        }});
                        return Array.prototype.push.apply(this, safeItems);
                    }}
                }});
                window.__mcp_intercepted = guarded;
                window.__mcp_intercepted_safe_v2 = true;
                if (prior.length) guarded.push(...prior);
            }};
            window.__mcp_reset_intercepted(window.__mcp_intercepted_safe_v2 === true);
            if (window.__mcp_interception_installed) return 'updated';

            // Override fetch
            const origFetch = window.fetch;
            window.fetch = async function(input, init) {{
                const url = typeof input === 'string' ? input : input.url;
                for (const route of window.__mcp_routes) {{
                    if (routeMatches(route.pattern, safeUrlShape(url))) {{
                        window.__mcp_intercepted.push({{url: safeUrlShape(url), method: init?.method || 'GET', time: Date.now(), action: route.action.type}});
                        if (route.action.type === 'block') {{
                            return new Response('', {{status: 0, statusText: 'Blocked by MCP route'}});
                        }}
                        if (route.action.type === 'mock') {{
                            return new Response(route.action.body, {{
                                status: route.action.status,
                                headers: {{'Content-Type': route.action.contentType}}
                            }});
                        }}
                        // 'log' falls through
                    }}
                }}
                return origFetch.apply(this, arguments);
            }};

            // Override XMLHttpRequest
            const origXHROpen = XMLHttpRequest.prototype.open;
            XMLHttpRequest.prototype.open = function(method, url, ...rest) {{
                for (const route of window.__mcp_routes) {{
                    if (routeMatches(route.pattern, safeUrlShape(url))) {{
                        window.__mcp_intercepted.push({{url: safeUrlShape(url), method, time: Date.now(), action: route.action.type}});
                        if (route.action.type === 'block') {{
                            this.__mcp_blocked = true;
                        }}
                        if (route.action.type === 'mock') {{
                            this.__mcp_mock = route.action;
                        }}
                    }}
                }}
                return origXHROpen.apply(this, [method, url, ...rest]);
            }};

            const origXHRSend = XMLHttpRequest.prototype.send;
            XMLHttpRequest.prototype.send = function(body) {{
                if (this.__mcp_blocked) {{
                    Object.defineProperty(this, 'status', {{value: 0}});
                    Object.defineProperty(this, 'responseText', {{value: ''}});
                    this.dispatchEvent(new Event('error'));
                    return;
                }}
                if (this.__mcp_mock) {{
                    Object.defineProperty(this, 'status', {{value: this.__mcp_mock.status}});
                    Object.defineProperty(this, 'responseText', {{value: this.__mcp_mock.body}});
                    Object.defineProperty(this, 'readyState', {{value: 4}});
                    this.dispatchEvent(new Event('readystatechange'));
                    this.dispatchEvent(new Event('load'));
                    return;
                }}
                return origXHRSend.apply(this, arguments);
            }};

            window.__mcp_interception_installed = true;
            return 'installed';
        }})()"#,
            rules = rules_js
        );

        self.evaluate(&script).await?;
        self.interception_active = true;
        Ok(())
    }

    /// Disable interception — restore original fetch/XHR
    pub async fn disable_interception(&mut self) -> Result<String, BrowserError> {
        // Keep the one installed wrapper in place and empty its rules. Marking
        // it uninstalled would cause later adds to wrap fetch/XHR repeatedly.
        self.evaluate("window.__mcp_routes = []").await?;
        self.routes.clear();
        self.interception_active = false;
        Ok("Interception routes cleared; one pass-through wrapper remains installed".into())
    }

    pub fn persistent_page_automation(&self) -> serde_json::Value {
        serde_json::json!({
            "active": !self.routes.is_empty() || self.trace_active,
            "route_count": self.routes.len(),
            "trace_active": self.trace_active,
        })
    }

    // ===== P5: Multiple Browser Contexts =====

    /// Create a new named browser context with its own page
    pub async fn create_context(
        &mut self,
        name: &str,
        url: Option<&str>,
    ) -> Result<String, BrowserError> {
        let browser = self.browser.as_ref().ok_or(BrowserError::NotLaunched)?;
        let target_url = url.unwrap_or("about:blank");
        let page = browser
            .new_page(target_url)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        let ctx = BrowserContext {
            pages: vec![page],
            active_page_index: 0,
        };
        self.contexts.insert(name.to_string(), ctx);
        Ok(format!("Context '{}' created at {}", name, target_url))
    }

    /// Switch to a named context (saves current page state)
    pub fn switch_context(&mut self, name: &str) -> Result<String, BrowserError> {
        // Handle "default" / "__default__" as switching back to the original context
        let is_default = name == "default" || name == "__default__";

        if !is_default && !self.contexts.contains_key(name) {
            return Err(BrowserError::ElementNotFound(format!(
                "Context '{}' not found",
                name
            )));
        }

        // Already in the requested context? No-op.
        if is_default && self.active_context.is_none() {
            return Ok("Already in default context".to_string());
        }
        if !is_default && self.active_context.as_deref() == Some(name) {
            return Ok(format!("Already in context '{}'", name));
        }

        // Save current pages to current context (or default)
        let current_name = self
            .active_context
            .clone()
            .unwrap_or_else(|| "__default__".to_string());
        let saved = BrowserContext {
            pages: std::mem::take(&mut self.pages),
            active_page_index: self.active_page_index,
        };
        self.contexts.insert(current_name, saved);

        // Load target context
        let lookup_key = if is_default { "__default__" } else { name };
        let target = match self.contexts.remove(lookup_key) {
            Some(ctx) => ctx,
            None => {
                return Err(BrowserError::ElementNotFound(format!(
                    "Context '{}' not found",
                    name
                )))
            }
        };
        self.pages = target.pages;
        self.active_page_index = target.active_page_index;
        self.active_context = if is_default {
            None
        } else {
            Some(name.to_string())
        };

        Ok(format!("Switched to context '{}'", name))
    }

    /// Destroy a named context and close its pages
    pub async fn destroy_context(&mut self, name: &str) -> Result<String, BrowserError> {
        if let Some(ctx) = self.contexts.remove(name) {
            let count = ctx.pages.len();
            // Pages are dropped, which closes them
            drop(ctx);
            if self.active_context.as_deref() == Some(name) {
                self.active_context = None;
            }
            Ok(format!(
                "Context '{}' destroyed ({} pages closed)",
                name, count
            ))
        } else {
            Err(BrowserError::ElementNotFound(format!(
                "Context '{}' not found",
                name
            )))
        }
    }

    /// List all contexts
    pub fn list_contexts(&self) -> Vec<serde_json::Value> {
        let mut result = vec![serde_json::json!({
            "name": self.active_context.as_deref().unwrap_or("default"),
            "pages": self.pages.len(),
            "active": true
        })];
        for (name, ctx) in &self.contexts {
            result.push(serde_json::json!({
                "name": name,
                "pages": ctx.pages.len(),
                "active": false
            }));
        }
        result
    }

    // ===== P2: Trace Recording =====

    /// Start trace recording — captures navigation, clicks, network timing
    pub async fn trace_start(&mut self) -> Result<String, BrowserError> {
        if self.trace_active {
            return Err(BrowserError::Cdp("Trace already active".into()));
        }
        self.trace_entries.clear();

        // Inject performance observer JS
        let script = r#"(() => {
            if (window.__mcp_trace_observer) window.__mcp_trace_observer.disconnect();
            if (window.__mcp_trace_url_check) clearInterval(window.__mcp_trace_url_check);
            if (window.__mcp_trace_click_handler) {
                document.removeEventListener('click', window.__mcp_trace_click_handler, true);
            }
            window.__mcp_trace_start = Date.now();

            const safeRouteLiterals = new Set([
                'api', 'rest', 'rpc', 'graphql', 'v1', 'v2', 'v3', 'v4', 'v5', 'v6',
                'v7', 'v8', 'v9', 'auth', 'oauth', 'login', 'logout', 'callback', 'users',
                'user', 'accounts', 'account', 'orders', 'order', 'events', 'webhooks',
                'health', 'status', 'assets', 'static'
            ]);
            const safeQueryKeys = new Set([
                'page', 'limit', 'offset', 'sort', 'order', 'filter', 'q', 'query',
                'search', 'cursor', 'after', 'before', 'fields', 'include', 'expand',
                'lang', 'locale', 'format', 'version', 'id', 'code', 'state', 'token', 'key'
            ]);
            const safeUrlShape = raw => {
                try {
                    const parsed = new URL(String(raw), location.href);
                    const path = parsed.pathname.split('/').map(segment => {
                        if (!segment) return '';
                        let decoded = '';
                        try { decoded = decodeURIComponent(segment).toLowerCase(); }
                        catch (_error) { return '{segment}'; }
                        return safeRouteLiterals.has(decoded) ? decoded : '{segment}';
                    }).join('/');
                    const keys = [];
                    parsed.searchParams.forEach((_value, key) => {
                        const normalized = String(key).toLowerCase();
                        const safeKey = safeQueryKeys.has(normalized) ? normalized : 'parameter';
                        if (!keys.includes(safeKey)) keys.push(safeKey);
                    });
                    const query = keys.length
                        ? '?' + keys.map(key => encodeURIComponent(key) + '=[REDACTED]').join('&')
                        : '';
                    return path + query + (parsed.hash ? '#[REDACTED]' : '');
                } catch (_error) {
                    return '[URL_WITHHELD]';
                }
            };
            window.__mcp_trace_safe_url = safeUrlShape;

            const traceTypes = new Set([
                'navigation', 'resource', 'longtask', 'paint',
                'largest-contentful-paint', 'click'
            ]);
            const projectTraceEntry = item => {
                if (!item || typeof item !== 'object') return {type: 'event'};
                const rawType = String(item.type || '').toLowerCase();
                const type = traceTypes.has(rawType) ? rawType : 'event';
                const safe = {type};
                for (const key of ['start', 'duration', 'time', 'x', 'y']) {
                    if (Number.isFinite(item[key])) safe[key] = Number(item[key]);
                }
                if (type === 'click') {
                    const tag = String(item.tag || '').toUpperCase();
                    if (/^[A-Z0-9-]{1,20}$/.test(tag)) safe.tag = tag;
                } else if (type === 'navigation' || type === 'resource') {
                    safe.name = safeUrlShape(item.name);
                    if (item.from !== undefined) safe.from = safeUrlShape(item.from);
                } else if (['self', 'first-paint', 'first-contentful-paint'].includes(item.name)) {
                    safe.name = String(item.name);
                }
                return safe;
            };
            const guardedTrace = [];
            Object.defineProperty(guardedTrace, 'push', {
                configurable: false,
                writable: false,
                value: function(...items) {
                    return Array.prototype.push.apply(this, items.map(projectTraceEntry));
                }
            });
            window.__mcp_trace = guardedTrace;

            // Observe navigation + resource timing
            const observer = new PerformanceObserver(list => {
                for (const entry of list.getEntries()) {
                    window.__mcp_trace.push({
                        type: entry.entryType,
                        name: (entry.entryType === 'navigation' || entry.entryType === 'resource')
                            ? safeUrlShape(entry.name)
                            : String(entry.name).slice(0, 80),
                        start: Math.round(entry.startTime),
                        duration: Math.round(entry.duration),
                        time: Date.now() - window.__mcp_trace_start
                    });
                }
            });
            observer.observe({entryTypes: ['navigation', 'resource', 'longtask', 'paint', 'largest-contentful-paint']});
            window.__mcp_trace_observer = observer;

            // Track clicks
            window.__mcp_trace_click_handler = e => {
                const target = e.target;
                window.__mcp_trace.push({
                    type: 'click',
                    tag: String(target.tagName || '').slice(0, 20),
                    x: e.clientX, y: e.clientY,
                    time: Date.now() - window.__mcp_trace_start
                });
            };
            document.addEventListener('click', window.__mcp_trace_click_handler, true);

            // Track URL changes
            let lastUrl = safeUrlShape(location.href);
            const urlCheck = setInterval(() => {
                const currentUrl = safeUrlShape(location.href);
                if (currentUrl !== lastUrl) {
                    window.__mcp_trace.push({
                        type: 'navigation',
                        name: currentUrl,
                        from: lastUrl,
                        time: Date.now() - window.__mcp_trace_start
                    });
                    lastUrl = currentUrl;
                }
            }, 200);
            window.__mcp_trace_url_check = urlCheck;

            return 'trace started';
        })()"#;
        self.evaluate(script).await?;
        self.trace_active = true;

        // Record trace start entry
        self.trace_entries.push(serde_json::json!({
            "type": "trace_start",
            "time": 0,
            "url": "[captured-as-route-shape-in-page-trace]"
        }));

        Ok("Trace recording started".into())
    }

    /// Stop trace recording and return all entries
    pub async fn trace_stop(&mut self) -> Result<serde_json::Value, BrowserError> {
        if !self.trace_active {
            return Err(BrowserError::Cdp("No active trace".into()));
        }

        // Collect JS-side trace entries
        let js_entries = self
            .evaluate("JSON.stringify(window.__mcp_trace || [])")
            .await?;

        // Clean up
        self.evaluate(
            r#"(() => {
            if (window.__mcp_trace_observer) window.__mcp_trace_observer.disconnect();
            if (window.__mcp_trace_url_check) clearInterval(window.__mcp_trace_url_check);
            if (window.__mcp_trace_click_handler) {
                document.removeEventListener('click', window.__mcp_trace_click_handler, true);
            }
            const dropSink = [];
            Object.defineProperty(dropSink, 'push', {
                configurable: false,
                writable: false,
                value: function() { return this.length; }
            });
            window.__mcp_trace = dropSink;
            delete window.__mcp_trace_start;
            delete window.__mcp_trace_safe_url;
            delete window.__mcp_trace_observer;
            delete window.__mcp_trace_url_check;
            delete window.__mcp_trace_click_handler;
            return 'cleaned';
        })()"#,
        )
        .await
        .ok();

        self.trace_active = false;

        // Merge server-side and JS-side entries
        let mut all_entries = self.trace_entries.clone();
        if let Some(s) = js_entries.as_str() {
            if let Ok(js_arr) = serde_json::from_str::<Vec<serde_json::Value>>(s) {
                all_entries.extend(js_arr);
            }
        } else if let Some(arr) = js_entries.as_array() {
            all_entries.extend(arr.clone());
        }
        self.trace_entries.clear();

        Ok(serde_json::json!({
            "entries": all_entries,
            "count": all_entries.len()
        }))
    }

    /// Save trace to a JSON file
    pub async fn trace_save(&mut self, path: &str) -> Result<String, BrowserError> {
        let trace_data = self.trace_stop().await?;
        let json = serde_json::to_string_pretty(&trace_data)
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        std::fs::write(path, &json)
            .map_err(|e| BrowserError::Cdp(format!("Failed to write trace: {}", e)))?;
        let count = trace_data
            .get("count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        Ok(format!("Trace saved to {} ({} entries)", path, count))
    }

    /// Add a manual trace entry (from tool-level actions)
    pub fn trace_log(&mut self, entry_type: &str, name: &str, details: Option<serde_json::Value>) {
        if !self.trace_active {
            return;
        }
        let mut entry = serde_json::json!({
            "type": entry_type,
            "name": name,
            "time": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
        });
        if let Some(d) = details {
            entry.as_object_mut().unwrap().insert("details".into(), d);
        }
        self.trace_entries.push(entry);
    }
}

pub type SharedBrowser = Arc<RwLock<BrowserManager>>;

pub fn create_shared() -> SharedBrowser {
    Arc::new(RwLock::new(BrowserManager::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_string_round_trips_hostile_input_as_one_literal() {
        let hostile = "\";globalThis.PWNED=true;//\n\\tail'\r\t";
        let encoded = js_string(hostile);
        let decoded: String = serde_json::from_str(&encoded).expect("valid JSON string literal");
        assert_eq!(decoded, hostile);
        assert!(!encoded.contains('\n'));
        assert!(!encoded.contains('\r'));
    }

    #[test]
    fn route_patterns_are_limited_to_noncredential_path_shapes() {
        for accepted in ["/api/*/orders/**", "**/api/*", "/graphql"] {
            assert_eq!(validate_route_pattern(accepted).unwrap(), accepted);
        }
        for rejected in [
            "https://example.test/api/*",
            "/api/*?token=secret",
            "/api/*#private",
            "/api/user@example.test",
            "/private/customer-name",
            "api/*",
        ] {
            assert!(validate_route_pattern(rejected).is_err(), "{rejected}");
        }
    }

    #[test]
    fn route_listing_never_returns_mock_body() {
        let mut manager = BrowserManager::new();
        manager.routes.push(RouteRule {
            pattern: "/api/*".to_string(),
            action: RouteAction::Mock {
                status: 200,
                content_type: "application/json".to_string(),
                body: "MOCK_BODY_SECRET".to_string(),
            },
        });
        let listed = manager.list_routes();
        let serialized = serde_json::to_string(&listed).unwrap();
        assert_eq!(listed[0]["pattern_shape"], "/api/*");
        assert!(!serialized.contains("MOCK_BODY_SECRET"));
        assert!(listed[0].get("body").is_none());
    }

    #[test]
    fn route_rules_are_json_serialized_without_script_injection() {
        let hostile_pattern = "api\\\";globalThis.PWNED=true;//\nnext";
        let hostile_content_type = "text/plain\\\";throw new Error('PWNED');//";
        let hostile_body = "line1\n\"quoted\"\\tail";
        let rules = vec![RouteRule {
            pattern: hostile_pattern.to_string(),
            action: RouteAction::Mock {
                status: 207,
                content_type: hostile_content_type.to_string(),
                body: hostile_body.to_string(),
            },
        }];

        let serialized = serialize_route_rules(&rules).expect("route rules serialize");
        let parsed: serde_json::Value =
            serde_json::from_str(&serialized).expect("serialized rules remain valid JSON");
        assert_eq!(parsed[0]["pattern"], hostile_pattern);
        assert_eq!(parsed[0]["action"]["contentType"], hostile_content_type);
        assert_eq!(parsed[0]["action"]["body"], hostile_body);
        assert!(!serialized.contains(";globalThis.PWNED=true;//\n"));
    }
}

// === FILE NAVIGATION ===
// Generated: 2026-04-14T17:52:41
// Total: 1353 lines | 3 functions | 3 structs | 20 constants
//
// IMPORTS: chromiumoxide, futures, serde_json, std, tokio
//
// CONSTANTS:
//   const el: 477
//   const style: 479
//   const result: 543
//   const el: 544
//   const rect: 546
//   const result: 626
//   const el: 627
//   const style: 629
//   const visible: 630
//   const el: 672
//   const r: 674
//   const rect: 832
//   const rect: 837
//   const origFetch: 1047
//   const url: 1049
//   const origXHROpen: 1069
//   const origXHRSend: 1087
//   const observer: 1228
//   const target: 1244
//   const urlCheck: 1256
//
// STRUCTS:
//   pub RouteRule: 42-45
//   pub BrowserContext: 58-61
//   pub BrowserManager: 63-80
//
// ENUMS:
//   pub BrowserError: 16-23
//   pub RouteAction: 48-55
//
// IMPL BLOCKS:
//   impl fmt::Display for BrowserError: 25-36
//   impl std::error::Error for BrowserError: 38-38
//   impl BrowserManager: 101-1347
//
// FUNCTIONS:
//   is_pid_alive: 84-94
//   is_pid_alive: 97-99
//   pub +create_shared: 1351-1353
//
// === END FILE NAVIGATION ===
