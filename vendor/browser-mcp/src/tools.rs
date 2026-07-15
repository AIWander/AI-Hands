//! Tool definitions and handlers
// NAV: TOC at line 3250 | 32 fn | 0 struct | 2026-06-13
use crate::browser::{BrowserManager, RouteAction, SharedBrowser};
use crate::types::*;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static A11Y_REF_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
static TEMP_IMAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_temp_image_path(prefix: &str) -> String {
    let counter = TEMP_IMAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let directory = std::env::temp_dir().join("hands-browser-captures");
    let _ = std::fs::create_dir_all(&directory);
    directory
        .join(format!(
            "{}_{}_{}_{}.png",
            prefix,
            nanos,
            std::process::id(),
            counter
        ))
        .to_string_lossy()
        .into_owned()
}

pub fn list_tools() -> Vec<ToolInfo> {
    let mut tools = vec![
        tool("launch", "Launch browser. headless=true for no window, profile_path for persistent session", json!({
            "type": "object",
            "properties": {
                "headless": {"type": "boolean", "default": true},
                "profile_path": {"type": "string", "description": "Path to Chrome profile for persistent logins"}
            }
        })),
        tool("attach", "Attach to existing Chrome with remote debugging. Start Chrome with --remote-debugging-port=9222", json!({
            "type": "object",
            "properties": {
                "port": {"type": "integer", "default": 9222, "description": "Chrome debugging port"}
            }
        })),
        tool("debug_launch", "Launch YOUR Chrome with debug port enabled. Keeps your logins/sessions. Waits for CDP to be ready before returning (configurable). Then use browser_attach to connect.", json!({
            "type": "object",
            "properties": {
                "port": {"type": "integer", "default": 9222, "description": "Debug port to use"},
                "url": {"type": "string", "description": "URL to open (optional, default: about:blank)"},
                "wait_for_cdp": {"type": "boolean", "default": true, "description": "Wait for CDP endpoint to be ready before returning (default: true). Set false for legacy fire-and-forget behavior."}
            }
        })),
        tool("close", "Close browser", json!({"type": "object", "properties": {}})),
        tool("navigate", "Navigate to URL", json!({
            "type": "object",
            "properties": {
                "url": {"type": "string"},
                "wait_until": {"type": "string", "default": "load", "enum": ["load", "networkidle"]}
            },
            "required": ["url"]
        })),
        tool("smart_navigate", "Launch or attach if needed, navigate, wait, and optionally verify page state with evidence.", json!({
            "type": "object",
            "properties": {
                "url": {"type": "string"},
                "headless": {"type": "boolean", "default": true},
                "profile_path": {"type": "string", "description": "Optional Chrome profile path when launch is needed"},
                "attach_port": {"type": "integer", "description": "Attach to an existing debug Chrome port instead of launching"},
                "wait_until": {"type": "string", "default": "networkidle", "enum": ["load", "networkidle"]},
                "wait_idle_ms": {"type": "integer", "default": 2000},
                "verify_text": {"type": "string", "description": "Optional text expected after navigation"},
                "verify_selector": {"type": "string", "description": "Optional selector expected after navigation"},
                "screenshot_path": {"type": "string", "description": "Optional evidence screenshot path"}
            },
            "required": ["url"]
        })),
        tool("click", "Click element by CSS selector, XPath, coordinates, or fuzzy text match. Supports automatic retry on transient failures via the `retry`/`retry_delay_ms` options — subsumes the legacy `retry_click` combo tool.", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string", "description": "CSS selector"},
                "xpath": {"type": "string", "description": "XPath expression (e.g. //button[@id='submit'], //div[contains(text(),'Login')])"},
                "a11y_ref": {"type": "string", "description": "Ref ID from a11y_snapshot/a11y_find, e.g. ref_3"},
                "match_text": {"type": "string", "description": "Fuzzy text match - finds clickable element by visible text, aria-label, title, href"},
                "x": {"type": "integer"},
                "y": {"type": "integer"},
                "auto_wait": {"type": "boolean", "default": false, "description": "Wait for element to be visible before clicking (up to auto_wait_ms)"},
                "auto_wait_ms": {"type": "integer", "default": 5000, "description": "Max wait time in ms when auto_wait is true"},
                "retry": {"type": "integer", "default": 1, "minimum": 1, "maximum": 3, "description": "Total attempts (default: 1 = no retry, max: 3). Between attempts, if `selector` was provided the click waits for it to appear via wait_for(timeout_ms = retry_delay_ms*2)."},
                "retry_delay_ms": {"type": "integer", "default": 500, "description": "Delay between retry attempts in ms (default: 500). Also doubles as the wait_for timeout between retries when a selector is provided."}
            }
        })),
        tool("type", "Type text into element by selector, XPath, or fuzzy text match", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string", "description": "CSS selector"},
                "xpath": {"type": "string", "description": "XPath expression (e.g. //input[@name='email'], //textarea[contains(@placeholder,'Search')])"},
                "match_text": {"type": "string", "description": "Fuzzy text match - finds input/textarea by visible text, aria-label, placeholder, name"},
                "text": {"type": "string"},
                "clear": {"type": "boolean", "default": true},
                "auto_wait": {"type": "boolean", "default": false, "description": "Wait for element to be visible before typing (up to auto_wait_ms)"},
                "auto_wait_ms": {"type": "integer", "default": 5000, "description": "Max wait time in ms when auto_wait is true"}
            },
            "required": ["text"]
        })),
        tool("press", "Press keyboard key", json!({
            "type": "object",
            "properties": {
                "key": {"type": "string", "description": "Key name: Enter, Tab, Escape, ArrowDown, etc"}
            },
            "required": ["key"]
        })),
        tool("screenshot", "Take screenshot. Returns base64 JPEG unless save_path provided (then saves to file, returns path)", json!({
            "type": "object",
            "properties": {
                "full_page": {"type": "boolean", "default": false},
                "quality": {"type": "integer", "default": 80, "minimum": 1, "maximum": 100},
                "selector": {"type": "string", "description": "Screenshot specific element"},
                "save_path": {"type": "string", "description": "Save to file path instead of returning base64 (0 tokens)"},
                "ocr": {"type": "boolean", "description": "If true, run OCR on the screenshot and include text in response"},
                "max_width": {"type": "integer", "description": "Resize to this max width (preserves aspect ratio). Recommended: 1024 for AI vision analysis. Reduces token cost."}
            }
        })),
        tool("page_capture", "Capture the current page as image, DOM dump, scroll-collected dump, or saved artifact bundle.", json!({
            "type": "object",
            "properties": {
                "mode": {"type": "string", "default": "bundle", "enum": ["image", "dom", "collector", "bundle"]},
                "url": {"type": "string", "description": "Optional URL to navigate before capture"},
                "use_current": {"type": "boolean", "default": true},
                "artifact_dir": {"type": "string", "description": "Directory for bundle artifacts; autogenerated under C:/temp when omitted"},
                "save_path": {"type": "string", "description": "Image mode screenshot path"},
                "full_page": {"type": "boolean", "default": true},
                "quality": {"type": "integer", "default": 80, "minimum": 1, "maximum": 100},
                "include_screenshot": {"type": "boolean", "default": true},
                "include_html": {"type": "boolean", "default": true},
                "include_text": {"type": "boolean", "default": true},
                "include_forms": {"type": "boolean", "default": true},
                "include_clickables": {"type": "boolean", "default": true},
                "include_a11y": {"type": "boolean", "default": true},
                "include_network": {"type": "boolean", "default": true},
                "max_text_length": {"type": "integer", "default": 20000},
                "max_a11y_nodes": {"type": "integer", "default": 200},
                "max_scrolls": {"type": "integer", "default": 10},
                "wait_ms": {"type": "integer", "default": 1000}
            }
        })),
        tool("page_dump", "Alias for page_capture(mode='dom' or mode='bundle'): structured page data with optional saved artifacts.", json!({
            "type": "object",
            "properties": {
                "mode": {"type": "string", "default": "dom", "enum": ["dom", "collector", "bundle"]},
                "artifact_dir": {"type": "string"},
                "full_page": {"type": "boolean", "default": true},
                "max_text_length": {"type": "integer", "default": 20000},
                "max_a11y_nodes": {"type": "integer", "default": 200},
                "max_scrolls": {"type": "integer", "default": 10},
                "wait_ms": {"type": "integer", "default": 1000}
            }
        })),
        tool("screenshot_burst", "Rapid screenshots for state tracking", json!({
            "type": "object",
            "properties": {
                "count": {"type": "integer", "default": 5},
                "interval_ms": {"type": "integer", "default": 200},
                "quality": {"type": "integer", "default": 60}
            }
        })),
        tool("wait_for", "Wait for element by CSS selector or XPath. condition: 'visible' (default, checks CSS visibility), 'exists' (DOM only)", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string", "description": "CSS selector"},
                "xpath": {"type": "string", "description": "XPath expression"},
                "timeout_ms": {"type": "integer", "default": 10000},
                "condition": {"type": "string", "default": "visible", "enum": ["visible", "exists"]}
            }
        })),
        tool("verify_state", "Verify page state with DOM checks first and optional screenshot/OCR fallback.", json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text expected in document.body innerText"},
                "selector": {"type": "string", "description": "Selector expected to exist"},
                "url_contains": {"type": "string"},
                "title_contains": {"type": "string"},
                "timeout_ms": {"type": "integer", "default": 5000},
                "poll_ms": {"type": "integer", "default": 250},
                "ocr": {"type": "boolean", "default": false},
                "screenshot_path": {"type": "string"},
                "full_page": {"type": "boolean", "default": false}
            }
        })),
        tool("get_html", "Get page or element HTML", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string", "description": "CSS selector, omit for full page"},
                "xpath": {"type": "string", "description": "XPath expression"}
            }
        })),
        tool("get_text", "Get text content of element", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string", "description": "CSS selector"},
                "xpath": {"type": "string", "description": "XPath expression"}
            },
            "required": ["selector"]
        })),
        tool("eval", "Execute JavaScript", json!({
            "type": "object",
            "properties": {
                "script": {"type": "string"}
            },
            "required": ["script"]
        })),
        tool("scroll", "Scroll page", json!({
            "type": "object",
            "properties": {
                "direction": {"type": "string", "enum": ["up", "down", "left", "right"], "default": "down"},
                "amount": {"type": "integer", "default": 500}
            }
        })),
        tool("select", "Select dropdown option", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string"},
                "value": {"type": "string"}
            },
            "required": ["selector", "value"]
        })),
        tool("hover", "Hover over element", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string"}
            },
            "required": ["selector"]
        })),
        tool("focus", "Focus element", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string"}
            },
            "required": ["selector"]
        })),
        tool("exists", "Check if element exists", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string"}
            },
            "required": ["selector"]
        })),
        tool("get_bounds", "Get element position and size", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string"}
            },
            "required": ["selector"]
        })),
        tool("get_clickables", "Get all clickable elements with coordinates (for vision)", json!({
            "type": "object",
            "properties": {}
        })),
        tool("a11y_snapshot", "Build an accessibility-oriented DOM snapshot with ref IDs usable by click(a11y_ref=...).", json!({
            "type": "object",
            "properties": {
                "max_nodes": {"type": "integer", "default": 200},
                "root_selector": {"type": "string", "description": "Optional selector to scope snapshot"}
            }
        })),
        tool("a11y_find", "Find nodes in the cached/refreshed accessibility snapshot by role and name.", json!({
            "type": "object",
            "properties": {
                "role": {"type": "string"},
                "name": {"type": "string"},
                "exact": {"type": "boolean", "default": false},
                "max_nodes": {"type": "integer", "default": 200}
            }
        })),
        tool("get_metrics", "Get page metrics (scroll position, dimensions)", json!({
            "type": "object",
            "properties": {}
        })),
        tool("cookies", "Manage cookies: get, set, clear", json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["get", "set", "clear"], "default": "get"},
                "name": {"type": "string"},
                "value": {"type": "string"},
                "domain": {"type": "string"}
            }
        })),
        tool("status", "Get browser status", json!({
            "type": "object",
            "properties": {}
        })),
        tool("new_tab", "Open new tab", json!({
            "type": "object",
            "properties": {}
        })),
        tool("list_tab", "List all open browser tabs with their URLs and titles.", json!({
            "type": "object",
            "properties": {}
        })),
        tool("switch_tab", "Switch to a different browser tab by index or URL match.", json!({
            "type": "object",
            "properties": {
                "index": {"type": "integer", "description": "Tab index (from list_tabs)"},
                "url_match": {"type": "string", "description": "Switch to tab whose URL contains this string"}
            }
        })),
        tool("close_tab", "Close a browser tab by index.", json!({
            "type": "object",
            "properties": {
                "index": {"type": "integer", "description": "Tab index to close (from list_tabs)"}
            },
            "required": ["index"]
        })),
        tool("get_url", "Get current page URL", json!({
            "type": "object",
            "properties": {}
        })),
        tool("back", "Go back in history", json!({
            "type": "object",
            "properties": {}
        })),
        tool("forward", "Go forward in history", json!({
            "type": "object",
            "properties": {}
        })),
        tool("reload", "Reload current page", json!({
            "type": "object",
            "properties": {}
        })),
        tool("get_forms", "Get all forms on page with their fields", json!({
            "type": "object",
            "properties": {}
        })),
        tool("fill_form", "Fill form fields by name", json!({
            "type": "object",
            "properties": {
                "form_selector": {"type": "string", "description": "Form selector (e.g. form, #myform)"},
                "data": {"type": "object", "description": "Field name -> value mapping"}
            },
            "required": ["form_selector", "data"]
        })),
        tool("submit_form", "Submit a form", json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string", "description": "Form selector"}
            },
            "required": ["selector"]
        })),
        tool("inject_script", "Inject and execute JavaScript", json!({
            "type": "object",
            "properties": {
                "script": {"type": "string"}
            },
            "required": ["script"]
        })),
        tool("wait_idle", "Wait for network idle", json!({
            "type": "object",
            "properties": {
                "timeout_ms": {"type": "integer", "default": 2000}
            }
        })),
        tool("http_scrape", "Simple HTTP GET, strip HTML, return text. Fast fallback when JS rendering not needed. No browser spin-up.", json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to scrape"}
            },
            "required": ["url"]
        })),
        tool("crawl", "Polite recursive crawler: fetches a site from a start URL, honoring robots.txt, per-domain rate limits, and an honest user-agent. Bounded by max_depth/max_pages/timeout. Returns clean text per page. Set CPC_CRAWLER_PROXY to egress through a non-home IP.", json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Start URL (http/https)"},
                "max_depth": {"type": "integer", "default": 3, "description": "Link depth from start (0 = start page only). Hard cap 12."},
                "max_pages": {"type": "integer", "default": 50, "description": "Max pages to fetch. Hard cap 2000."},
                "same_domain": {"type": "boolean", "default": true, "description": "Stay on the start URL's domain (www. treated as same site)"},
                "include": {"type": "array", "items": {"type": "string"}, "description": "Only crawl URLs containing one of these substrings"},
                "exclude": {"type": "array", "items": {"type": "string"}, "description": "Skip URLs containing any of these substrings"},
                "delay_ms": {"type": "integer", "default": 1000, "description": "Min delay between requests to the same domain; robots.txt Crawl-delay overrides upward"},
                "timeout_secs": {"type": "integer", "default": 120, "description": "Wall-clock budget for the whole crawl (clamped 5-1800)"},
                "respect_robots": {"type": "boolean", "default": true, "description": "Honor robots.txt — disable only for sites you own"},
                "max_chars_per_page": {"type": "integer", "default": 4000, "description": "Truncate each page's extracted text to this many chars"},
                "user_agent": {"type": "string", "description": "Override crawler user-agent (default: CPCBot, or env CPC_CRAWLER_UA)"},
                "proxy": {"type": "string", "description": "HTTP proxy URL for egress (default: env CPC_CRAWLER_PROXY, else direct)"}
            },
            "required": ["url"]
        })),
        tool("map", "Discover URLs on a site without extracting content. Reads sitemap.xml (incl. sitemap indexes) and supplements with a shallow link crawl. Returns a deduped URL list.", json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Site URL (http/https)"},
                "max_urls": {"type": "integer", "default": 200, "description": "Max URLs to return. Hard cap 2000."},
                "same_domain": {"type": "boolean", "default": true, "description": "Only include URLs on the start domain"},
                "user_agent": {"type": "string", "description": "Override user-agent (default: CPCBot, or env CPC_CRAWLER_UA)"},
                "proxy": {"type": "string", "description": "HTTP proxy URL for egress (default: env CPC_CRAWLER_PROXY)"}
            },
            "required": ["url"]
        })),
        tool("js_extract", "Spawn Node.js to extract content from a URL without Chrome. Supports linkedom or jsdom engines.", json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to extract"},
                "selector": {"type": "string", "description": "Optional CSS selector to target"},
                "engine": {"type": "string", "default": "linkedom", "enum": ["linkedom", "jsdom"]},
                "timeout_ms": {"type": "integer", "default": 5000, "description": "Extraction timeout in milliseconds"}
            },
            "required": ["url"]
        })),
        tool("smart_browse", "Try HTTP scrape first, then escalate to linkedom, then jsdom, and finally flag when Chrome rendering is needed.", json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to browse"},
                "selector": {"type": "string", "description": "Optional CSS selector to target"}
            },
            "required": ["url"]
        })),
        tool("scroll_collect", "Scroll the active page until lazy-loaded content stops expanding, then extract content.", json!({
            "type": "object",
            "properties": {
                "max_scrolls": {"type": "integer", "default": 10},
                "wait_ms": {"type": "integer", "default": 2000}
            }
        })),
        tool("wait_stable", "Wait for visual stability by comparing consecutive screenshot file sizes.", json!({
            "type": "object",
            "properties": {
                "interval_ms": {"type": "integer", "default": 500},
                "max_attempts": {"type": "integer", "default": 10}
            }
        })),
        tool("agent", "DOM-based browser agent. Executes multi-step workflows using DOM heuristics (fuzzy text matching, no vision). Returns step-by-step narrative log. On failure, saves screenshot and returns available DOM elements.", json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Starting URL"},
                "steps": {
                    "type": "array",
                    "description": "Ordered steps to execute",
                    "items": {
                        "type": "object",
                        "properties": {
                            "intent": {"type": "string", "enum": ["click", "login", "fill", "extract", "wait", "navigate", "select"], "description": "Action type"},
                            "match_text": {"type": "string", "description": "Text to fuzzy-match against visible DOM elements"},
                            "data": {"type": "object", "description": "Key-value pairs for login/fill (field_name: value)"},
                            "selector": {"type": "string", "description": "CSS selector override (skips fuzzy match)"},
                            "timeout_ms": {"type": "integer", "default": 10000}
                        },
                        "required": ["intent"]
                    }
                },
                "headless": {"type": "boolean", "default": true},
                "save_screenshots": {"type": "boolean", "default": false, "description": "Save screenshot after each step to C:/temp/agent_run/"},
                "profile_path": {"type": "string", "description": "Chrome profile path for persistent sessions"}
            },
            "required": ["url", "steps"]
        })),
        tool("verify_visual", "Take screenshot and verify expected text is present via OCR. Returns pass/fail with OCR text.", json!({
            "type": "object",
            "properties": {
                "expected_text": {"type": "string", "description": "Text that should appear on screen"},
                "selector": {"type": "string", "description": "Optional CSS selector to screenshot specific element"}
            },
            "required": ["expected_text"]
        })),
        tool("extract_content", "Extract clean article/main content from URL, stripping nav, ads, and boilerplate", json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to extract content from (navigates if needed)"},
                "use_current": {"type": "boolean", "default": false, "description": "Extract from current page instead of navigating"},
                "include_links": {"type": "boolean", "default": false, "description": "Include hyperlinks in output"},
                "max_length": {"type": "integer", "default": 10000, "description": "Max characters to return"}
            }
        })),
        tool("iframe_extract", "Extract content from iframes on current page. Auto-detects same-origin (reads DOM) vs cross-origin (screenshots + OCR).", json!({
            "type": "object",
            "properties": {
                "target_index": {"type": "integer", "description": "Extract specific iframe by index (0-based). Omit for first iframe."},
                "include_all": {"type": "boolean", "default": false, "description": "Extract from ALL iframes, return array"}
            }
        })),
        tool("bulk_extract", "Extract text from multiple URLs in sequence. Uses smart_browse auto-escalation for each URL.", json!({
            "type": "object",
            "properties": {
                "urls": {"type": "array", "items": {"type": "string"}, "description": "Array of URLs to extract"},
                "max_length_per_page": {"type": "integer", "default": 5000, "description": "Max chars per page"},
                "selector": {"type": "string", "description": "CSS selector to apply to all pages"}
            },
            "required": ["urls"]
        })),
        // P4: Script Batching
        tool("script", "Execute a batch of browser steps with variable substitution. Steps run sequentially; {{var}} placeholders replaced from vars map. On failure, returns partial results + screenshot.", json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "description": "Array of step objects to execute sequentially",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": {"type": "string", "description": "Tool name: navigate, click, type, press, wait_for, scroll, evaluate, extract_content, screenshot, get_text, get_html, select"},
                            "params": {"type": "object", "description": "Parameters for the tool (same as calling it directly). Use {{var}} for variable substitution."}
                        },
                        "required": ["tool"]
                    }
                },
                "vars": {
                    "type": "object",
                    "description": "Variable map for substitution. Keys without braces, e.g. {\"email\": \"me@x.com\"} replaces {{email}} in all step params."
                },
                "stop_on_error": {"type": "boolean", "default": true, "description": "Stop execution on first error (true) or continue (false)"},
                "step_delay_ms": {"type": "integer", "default": 0, "description": "Delay between steps in ms"}
            },
            "required": ["steps"]
        })),

        // P1: Network Interception
        tool("route", "Add a network interception route. Intercepts fetch() and XHR requests matching the URL pattern.", json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "URL substring or regex pattern to match"},
                "action": {"type": "string", "enum": ["block", "mock", "log"], "default": "log", "description": "block=reject request, mock=return fake response, log=passthrough+record"},
                "mock_status": {"type": "integer", "default": 200, "description": "HTTP status for mock responses"},
                "mock_content_type": {"type": "string", "default": "application/json", "description": "Content-Type for mock responses"},
                "mock_body": {"type": "string", "description": "Response body for mock action"}
            },
            "required": ["pattern"]
        })),
        tool("route_remove", "Remove a network interception route by pattern", json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Pattern to remove"}
            },
            "required": ["pattern"]
        })),
        tool("route_list", "List all active network interception routes", json!({
            "type": "object",
            "properties": {}
        })),
        tool("route_clear", "Disable all network interception and restore normal fetch/XHR", json!({
            "type": "object",
            "properties": {}
        })),
        tool("get_network_log", "Get intercepted/logged network requests captured by routes", json!({
            "type": "object",
            "properties": {
                "clear": {"type": "boolean", "default": false, "description": "Clear the log after reading"}
            }
        })),

        // P5: Multiple Browser Contexts
        tool("context_create", "Create a named browser context with isolated page(s). Each context has separate cookies, storage, and DOM state.", json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Context name (e.g. 'logged-in', 'incognito', 'scraper')"},
                "url": {"type": "string", "description": "Initial URL to navigate to (default: about:blank)"}
            },
            "required": ["name"]
        })),
        tool("context_switch", "Switch to a named browser context. Current context is saved and can be switched back to.", json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Context name to switch to"}
            },
            "required": ["name"]
        })),
        tool("context_destroy", "Destroy a named context and close all its pages", json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Context name to destroy"}
            },
            "required": ["name"]
        })),
        tool("context_list", "List all browser contexts and their page counts", json!({
            "type": "object",
            "properties": {}
        })),

        // P2: Trace Recording
        tool("trace_start", "Start recording a browser trace. Captures navigation, clicks, resource loading, paint timing, and URL changes.", json!({
            "type": "object",
            "properties": {}
        })),
        tool("trace_stop", "Stop trace recording and return all captured entries as JSON", json!({
            "type": "object",
            "properties": {}
        })),
        tool("trace_save", "Stop trace recording and save to a JSON file", json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path to save trace JSON (e.g. C:/temp/trace.json)"}
            },
            "required": ["path"]
        })),
    ];
    tools.push(tool("plan", "Analyze a task and return its ingredients: what tools are needed, which depend on each other, and whether breadcrumbing is warranted.", json!({"type": "object", "properties": {"task": {"type": "string", "description": "What needs to be done"}, "context": {"type": "string", "description": "Additional context"}}, "required": ["task"]})));

    // P6: Spec-Driven Evaluation
    tools.push(tool("evaluate", "Evaluate a web artifact. With spec: runs test steps with assertions. Without spec: auto-discovery mode — checks page load, console errors, broken images, unlabeled inputs, link validity, a11y summary, and page metrics. Returns structured pass/fail/warning with screenshot evidence.", json!({
        "type": "object",
        "properties": {
            "target": {"type": "string", "description": "URL to open"},
            "intent": {"type": "string", "description": "What is being verified (used in summary)"},
            "spec": {
                "type": "array",
                "description": "Test steps to execute. Each step has: tool (browser tool name), params (tool params), and optional assert (type + target + expected).",
                "items": {
                    "type": "object",
                    "properties": {
                        "tool": {"type": "string"},
                        "params": {"type": "object"},
                        "assert": {
                            "type": "object",
                            "properties": {
                                "type": {"type": "string", "enum": ["text_contains", "element_exists", "value_equals"]},
                                "target": {"type": "string", "description": "CSS selector or XPath"},
                                "expected": {"type": "string"}
                            }
                        }
                    }
                }
            },
            "evidence": {"type": "boolean", "default": true, "description": "Save screenshots at each step"}
        },
        "required": ["target", "intent"]
    })));

    tools
}

fn tool(name: &str, description: &str, schema: serde_json::Value) -> ToolInfo {
    ToolInfo {
        name: name.into(),
        description: description.into(),
        input_schema: schema,
    }
}

async fn http_scrape_text(url: &str) -> Result<(String, String, u128), String> {
    let started = Instant::now();
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| e.to_string())?;
    let html = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let document = scraper::Html::parse_document(&html);
    let text_content: String = document
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Ok((html, text_content, started.elapsed().as_millis()))
}

fn has_js_shell_signals(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    [
        "id=\"root\"",
        "id='root'",
        "id=\"app\"",
        "id='app'",
        "id=\"__next\"",
        "id='__next'",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn text_is_short(text: &str) -> bool {
    text.trim().chars().count() < 100
}

fn looks_like_shell_page(html: &str, text: &str) -> bool {
    text_is_short(text) || (has_js_shell_signals(html) && text.trim().chars().count() < 300)
}

fn run_js_extract(
    url: &str,
    selector: Option<&str>,
    engine: &str,
    timeout_ms: u64,
) -> Result<serde_json::Value, String> {
    let payload = json!({
        "url": url,
        "selector": selector,
        "engine": engine,
        "timeout": timeout_ms
    });
    let json_args = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    let node_exe = std::env::var_os("AIHANDS_NODE_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("node.exe"));
    let script = if let Some(configured) = std::env::var_os("AIHANDS_JS_EXTRACT_SCRIPT") {
        PathBuf::from(configured)
    } else {
        let executable_relative = std::env::current_exe().ok().and_then(|path| {
            path.parent()
                .map(|parent| parent.join("helpers").join("js_extract.js"))
        });
        let repository_relative = std::env::current_dir()
            .ok()
            .map(|path| path.join("scripts").join("js_extract.js"));
        executable_relative
            .into_iter()
            .chain(repository_relative)
            .find(|path| path.is_file())
            .ok_or_else(|| {
                "js_extract helper not found; reinstall the AI-Hands optional package or set AIHANDS_JS_EXTRACT_SCRIPT"
                    .to_string()
            })?
    };

    let mut command = Command::new(&node_exe);
    command.arg(&script).arg(&json_args);
    if let Some(node_path) = std::env::var_os("AIHANDS_NODE_PATH") {
        command.env("NODE_PATH", node_path);
    }
    let output = command.output().map_err(|e| {
        format!(
            "spawn js_extract with {} failed: {}; install Node.js or set AIHANDS_NODE_EXE",
            node_exe.display(),
            e
        )
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stdout.is_empty() {
        return Err(if stderr.is_empty() {
            "js_extract produced no stdout".into()
        } else {
            format!("js_extract stderr: {}", stderr)
        });
    }

    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("js_extract JSON parse failed: {} | stdout: {}", e, stdout))?;
    if output.status.success() {
        Ok(parsed)
    } else {
        let msg = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                if stderr.is_empty() {
                    None
                } else {
                    Some(stderr.clone())
                }
            })
            .unwrap_or_else(|| format!("js_extract exited with {}", output.status));
        Err(msg)
    }
}

/// Shared extraction logic used by both smart_browse and bulk_extract.
/// Tries http_scrape → linkedom → jsdom → returns needs_chrome marker.
async fn smart_extract(url: &str, selector: Option<&str>) -> (String, &'static str) {
    // Tier 1: HTTP scrape
    if let Ok((html, text_content, _)) = http_scrape_text(url).await {
        if !looks_like_shell_page(&html, &text_content) {
            return (text_content, "http");
        }
    }
    // Tier 2: linkedom
    if let Ok(linkedom) = run_js_extract(url, selector, "linkedom", 5000) {
        let t = linkedom
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !t.is_empty() && !text_is_short(&t) {
            return (t, "linkedom");
        }
    }
    // Tier 3: jsdom
    if let Ok(jsdom) = run_js_extract(url, selector, "jsdom", 5000) {
        let t = jsdom
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !t.is_empty() {
            return (t, "jsdom");
        }
    }
    (String::new(), "needs_chrome")
}

fn resize_jpeg_bytes(bytes: &[u8], max_width: u32, quality: u8) -> Result<Vec<u8>, String> {
    use image::GenericImageView;
    use std::io::Cursor;

    let img = image::load_from_memory(bytes).map_err(|e| format!("image decode: {}", e))?;

    let (w, h) = img.dimensions();
    if w <= max_width {
        return Ok(bytes.to_vec());
    }

    let ratio = max_width as f64 / w as f64;
    let new_h = (h as f64 * ratio) as u32;
    let resized = img.resize_exact(max_width, new_h, image::imageops::FilterType::Lanczos3);

    let mut output = Cursor::new(Vec::new());
    resized
        .write_to(&mut output, image::ImageOutputFormat::Jpeg(quality))
        .map_err(|e| format!("image encode: {}", e))?;

    Ok(output.into_inner())
}

fn resize_b64_jpeg(b64: &str, max_width: u32, quality: u8) -> Result<String, String> {
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
        .map_err(|e| format!("base64 decode: {}", e))?;
    let resized = resize_jpeg_bytes(&bytes, max_width, quality)?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &resized,
    ))
}

fn format_json_text(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn build_extract_script(include_links: bool, max_length: usize) -> String {
    let link_mode = if include_links { "true" } else { "false" };
    format!(
        r#"
        (() => {{
            const remove = ['nav', 'header', 'footer', 'aside', 'script', 'style', 'noscript',
                '[role="navigation"]', '[role="banner"]', '[role="contentinfo"]', '.sidebar',
                '.nav', '.menu', '.footer', '.header', '.ad', '.ads', '.advertisement',
                '.cookie-banner', '.popup', '#cookie-consent', '.social-share'];
            const doc = document.cloneNode(true);
            remove.forEach(sel => doc.querySelectorAll(sel).forEach(el => el.remove()));

            const main = doc.querySelector('article, main, [role="main"], .post-content, .article-body, .entry-content')
                || doc.querySelector('.content, #content, .post, .article')
                || doc.body;

            const title = document.querySelector('h1')?.innerText?.trim()
                || document.title
                || '';
            const author = document.querySelector('meta[name="author"]')?.content
                || document.querySelector('[rel="author"]')?.innerText?.trim()
                || '';
            const date = document.querySelector('meta[property="article:published_time"]')?.content
                || document.querySelector('time')?.getAttribute('datetime')
                || '';

            let content;
            if ({link_mode}) {{
                const walk = (node) => {{
                    if (node.nodeType === 3) return node.textContent;
                    if (node.tagName === 'A' && node.href) return `[${{node.innerText.trim()}}](${{node.href}})`;
                    if (node.tagName === 'BR') return '\n';
                    if (['P', 'DIV', 'H1', 'H2', 'H3', 'H4', 'LI', 'TR'].includes(node.tagName)) {{
                        return '\n' + Array.from(node.childNodes).map(walk).join('') + '\n';
                    }}
                    return Array.from(node.childNodes).map(walk).join('');
                }};
                content = walk(main);
            }} else {{
                content = main.innerText;
            }}

            content = (content || '').replace(/\n{{3,}}/g, '\n\n').trim();
            return JSON.stringify({{
                title,
                author,
                date,
                url: window.location.href,
                content: content.slice(0, {max_length}),
                truncated: content.length > {max_length},
                content_length: content.length
            }});
        }})()
    "#
    )
}

async fn extract_page_content(
    browser: &SharedBrowser,
    include_links: bool,
    max_length: usize,
) -> Result<serde_json::Value, String> {
    let script = build_extract_script(include_links, max_length);
    let result = browser
        .read()
        .await
        .evaluate(&script)
        .await
        .map_err(|e| e.to_string())?;
    let result_str = result.as_str().unwrap_or("");
    serde_json::from_str::<serde_json::Value>(result_str).map_err(|_| result_str.to_string())
}

fn a11y_cache() -> &'static Mutex<HashMap<String, String>> {
    A11Y_REF_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

fn parse_eval_json(value: serde_json::Value) -> serde_json::Value {
    if let Some(s) = value.as_str() {
        serde_json::from_str(s).unwrap_or_else(|_| json!(s))
    } else {
        value
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_millis(0))
        .as_millis()
}

fn default_artifact_dir(prefix: &str) -> String {
    format!("C:/temp/{}_{}", prefix, now_millis())
}

fn write_json_file(path: &str, value: &serde_json::Value) -> Result<String, String> {
    let body =
        serde_json::to_string_pretty(value).map_err(|e| format!("serialize {}: {}", path, e))?;
    std::fs::write(path, body).map_err(|e| format!("write {}: {}", path, e))?;
    Ok(path.to_string())
}

fn lookup_a11y_selector(ref_id: &str) -> Option<String> {
    a11y_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(ref_id).cloned())
}

fn build_a11y_snapshot_script(max_nodes: usize, root_selector: Option<&str>) -> String {
    let root_expr = match root_selector {
        Some(sel) if !sel.trim().is_empty() => {
            format!("document.querySelector({}) || document", js_string(sel))
        }
        _ => "document".to_string(),
    };

    format!(
        r#"
        (() => {{
            const root = {root_expr};
            const maxNodes = {max_nodes};
            const cssPath = (el) => {{
                if (!el || !el.tagName) return '';
                if (el.id) return '#' + CSS.escape(el.id);
                const parts = [];
                let cur = el;
                while (cur && cur.nodeType === 1 && cur !== document.body && cur !== document.documentElement) {{
                    let part = cur.tagName.toLowerCase();
                    if (cur.name) {{
                        part += '[name="' + String(cur.name).replace(/"/g, '\\"') + '"]';
                        parts.unshift(part);
                        break;
                    }}
                    const parent = cur.parentElement;
                    if (parent) {{
                        const siblings = Array.from(parent.children).filter(c => c.tagName === cur.tagName);
                        if (siblings.length > 1) part += ':nth-of-type(' + (siblings.indexOf(cur) + 1) + ')';
                    }}
                    parts.unshift(part);
                    cur = parent;
                }}
                return parts.join(' > ');
            }};
            const inferRole = (el) => {{
                const explicit = el.getAttribute('role');
                if (explicit) return explicit;
                const tag = el.tagName.toLowerCase();
                const type = (el.getAttribute('type') || '').toLowerCase();
                if (tag === 'a') return 'link';
                if (tag === 'button' || type === 'button' || type === 'submit') return 'button';
                if (tag === 'input' && ['checkbox', 'radio'].includes(type)) return type;
                if (tag === 'input' || tag === 'textarea') return 'textbox';
                if (tag === 'select') return 'combobox';
                if (/^h[1-6]$/.test(tag)) return 'heading';
                if (tag === 'img') return 'img';
                return tag;
            }};
            const nameOf = (el) => (
                el.getAttribute('aria-label') ||
                el.getAttribute('alt') ||
                el.getAttribute('title') ||
                el.getAttribute('placeholder') ||
                el.innerText ||
                el.value ||
                ''
            ).trim().replace(/\s+/g, ' ');
            const candidates = Array.from(root.querySelectorAll(
                'a,button,input,textarea,select,summary,[role],[aria-label],[onclick],h1,h2,h3,h4,h5,h6,img'
            ));
            return JSON.stringify(candidates.filter(el => {{
                const rect = el.getBoundingClientRect();
                const style = window.getComputedStyle(el);
                return rect.width > 0 && rect.height > 0 &&
                    style.display !== 'none' && style.visibility !== 'hidden';
            }}).slice(0, maxNodes).map((el) => {{
                const rect = el.getBoundingClientRect();
                return {{
                    role: inferRole(el),
                    name: nameOf(el).slice(0, 160),
                    tag: el.tagName.toLowerCase(),
                    selector: cssPath(el),
                    disabled: !!el.disabled || el.getAttribute('aria-disabled') === 'true',
                    bounds: {{
                        x: Math.round(rect.x),
                        y: Math.round(rect.y),
                        width: Math.round(rect.width),
                        height: Math.round(rect.height)
                    }}
                }};
            }}));
        }})()
    "#
    )
}

async fn refresh_a11y_snapshot_from_manager(
    bm: &BrowserManager,
    max_nodes: usize,
    root_selector: Option<&str>,
) -> Result<serde_json::Value, String> {
    let script = build_a11y_snapshot_script(max_nodes.max(1), root_selector);
    let raw = bm.evaluate(&script).await.map_err(|e| e.to_string())?;
    let parsed = parse_eval_json(raw);
    let arr = parsed.as_array().cloned().unwrap_or_default();
    let mut nodes = Vec::new();
    let mut cache = HashMap::new();

    for (idx, node) in arr.into_iter().enumerate() {
        let ref_id = format!("ref_{}", idx);
        if let Some(selector) = node.get("selector").and_then(|v| v.as_str()) {
            if !selector.is_empty() {
                cache.insert(ref_id.clone(), selector.to_string());
            }
        }
        let mut node_obj = node.as_object().cloned().unwrap_or_default();
        node_obj.insert("ref".to_string(), json!(ref_id));
        nodes.push(serde_json::Value::Object(node_obj));
    }

    if let Ok(mut global_cache) = a11y_cache().lock() {
        *global_cache = cache;
    }

    Ok(json!({"count": nodes.len(), "nodes": nodes}))
}

async fn scroll_until_stable(
    browser: &SharedBrowser,
    max_scrolls: usize,
    wait_ms: u64,
) -> Result<serde_json::Value, String> {
    let mut scrolls_performed = 0usize;
    let mut last_height = browser
        .read()
        .await
        .evaluate("Math.max(document.body.scrollHeight, document.documentElement.scrollHeight)")
        .await
        .map_err(|e| e.to_string())?
        .as_i64()
        .unwrap_or(0);

    for _ in 0..max_scrolls.max(1) {
        browser
            .read()
            .await
            .evaluate("window.scrollTo(0, Math.max(document.body.scrollHeight, document.documentElement.scrollHeight))")
            .await
            .map_err(|e| e.to_string())?;
        if wait_ms > 0 {
            let _ = browser.read().await.wait_network_idle(wait_ms).await;
            tokio::time::sleep(Duration::from_millis(wait_ms.min(1000))).await;
        }
        let new_height = browser
            .read()
            .await
            .evaluate("Math.max(document.body.scrollHeight, document.documentElement.scrollHeight)")
            .await
            .map_err(|e| e.to_string())?
            .as_i64()
            .unwrap_or(last_height);
        scrolls_performed += 1;
        if new_height <= last_height {
            last_height = new_height;
            break;
        }
        last_height = new_height;
    }

    Ok(json!({"scrolls_performed": scrolls_performed, "final_scroll_height": last_height}))
}

fn build_page_dump_script(max_text_length: usize) -> String {
    format!(
        r#"
        (() => {{
            const maxText = {max_text_length};
            const trim = (s, n = 240) => String(s || '').replace(/\s+/g, ' ').trim().slice(0, n);
            const text = trim(document.body ? document.body.innerText : '', maxText);
            const headings = Array.from(document.querySelectorAll('h1,h2,h3,h4,h5,h6')).slice(0, 120).map(h => ({{
                level: h.tagName.toLowerCase(),
                text: trim(h.innerText)
            }}));
            const links = Array.from(document.querySelectorAll('a[href]')).slice(0, 250).map(a => ({{
                text: trim(a.innerText || a.getAttribute('aria-label')),
                href: a.href
            }}));
            const buttons = Array.from(document.querySelectorAll('button,[role=button],input[type=button],input[type=submit]')).slice(0, 160).map(b => ({{
                text: trim(b.innerText || b.value || b.getAttribute('aria-label')),
                disabled: !!b.disabled || b.getAttribute('aria-disabled') === 'true'
            }}));
            const forms = Array.from(document.forms).slice(0, 50).map((f, i) => ({{
                index: i,
                id: f.id || '',
                action: f.action || '',
                method: f.method || '',
                fields: Array.from(f.elements).slice(0, 80).map(el => ({{
                    tag: el.tagName.toLowerCase(),
                    type: el.type || '',
                    name: el.name || '',
                    id: el.id || '',
                    label: trim(el.labels && el.labels[0] ? el.labels[0].innerText : el.getAttribute('aria-label') || el.placeholder || '')
                }}))
            }}));
            return JSON.stringify({{
                url: window.location.href,
                title: document.title,
                ready_state: document.readyState,
                text,
                text_length: document.body && document.body.innerText ? document.body.innerText.length : 0,
                truncated: document.body && document.body.innerText ? document.body.innerText.length > maxText : false,
                headings,
                links,
                buttons,
                forms,
                counts: {{
                    elements: document.querySelectorAll('*').length,
                    links: document.querySelectorAll('a[href]').length,
                    forms: document.forms.length,
                    iframes: document.querySelectorAll('iframe').length,
                    scripts: document.querySelectorAll('script').length
                }}
            }});
        }})()
    "#
    )
}

async fn page_dump_from_manager(
    bm: &BrowserManager,
    max_text_length: usize,
) -> Result<serde_json::Value, String> {
    let raw = bm
        .evaluate(&build_page_dump_script(max_text_length.max(1)))
        .await
        .map_err(|e| e.to_string())?;
    Ok(parse_eval_json(raw))
}

async fn handle_page_capture(
    browser: &SharedBrowser,
    name: &str,
    params: &serde_json::Value,
) -> Result<Vec<ToolContent>, String> {
    let p = |key: &str| params.get(key);
    let s = |key: &str| p(key).and_then(|v| v.as_str()).unwrap_or_default();
    let b = |key: &str, def: bool| p(key).and_then(|v| v.as_bool()).unwrap_or(def);
    let i = |key: &str, def: i64| p(key).and_then(|v| v.as_i64()).unwrap_or(def);

    let mode = {
        let raw = s("mode");
        if raw.is_empty() {
            if name == "page_dump" {
                "dom"
            } else {
                "bundle"
            }
        } else {
            raw
        }
    };
    let use_current = b("use_current", true);
    let full_page = b("full_page", true);
    let quality = i("quality", 80).clamp(1, 100) as u8;
    let max_text_length = i("max_text_length", 20000).max(1) as usize;
    let max_a11y_nodes = i("max_a11y_nodes", 200).max(1) as usize;
    let max_scrolls = i("max_scrolls", 10).max(1) as usize;
    let wait_ms = i("wait_ms", 1000).max(0) as u64;

    if let Some(url) = p("url").and_then(|v| v.as_str()) {
        if !use_current || !url.is_empty() {
            {
                let mut bw = browser.write().await;
                if !bw.is_alive().await {
                    bw.launch(true, None).await.map_err(|e| e.to_string())?;
                }
                bw.navigate(url, "networkidle")
                    .await
                    .map_err(|e| e.to_string())?;
            }
            if wait_ms > 0 {
                let _ = browser.read().await.wait_network_idle(wait_ms).await;
            }
        }
    }

    let scroll_summary = if mode == "collector" || b("scroll_collect", false) {
        Some(scroll_until_stable(browser, max_scrolls, wait_ms).await?)
    } else {
        None
    };

    if mode == "image" {
        let save_path = p("save_path").and_then(|v| v.as_str());
        let br = browser.read().await;
        if let Some(path) = save_path {
            let saved = br
                .screenshot_to_file(path, full_page, quality)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(vec![text(
                &json!({"mode": "image", "full_page": full_page, "screenshot_path": saved})
                    .to_string(),
            )]);
        }
        let image = br
            .screenshot(full_page, quality)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(vec![ToolContent::Image {
            data: image,
            mime_type: "image/jpeg".into(),
        }]);
    }

    let include_screenshot = b("include_screenshot", true);
    let include_html = b("include_html", true);
    let include_text = b("include_text", true);
    let include_forms = b("include_forms", true);
    let include_clickables = b("include_clickables", true);
    let include_a11y = b("include_a11y", true);
    let include_network = b("include_network", true);

    let br = browser.read().await;
    let metrics = br.get_metrics().await.map_err(|e| e.to_string())?;
    let mut dump = page_dump_from_manager(&br, max_text_length).await?;
    let mut artifacts = serde_json::Map::new();

    if let Some(summary) = scroll_summary {
        dump["scroll_collect"] = summary;
    }
    dump["metrics"] = metrics;

    if include_forms {
        dump["forms_detail"] = br.get_forms().await.unwrap_or(json!([]));
    }
    if include_clickables {
        dump["clickables"] = json!(br.get_clickables().await.unwrap_or_default());
    }
    if include_a11y {
        dump["a11y"] = refresh_a11y_snapshot_from_manager(&br, max_a11y_nodes, None)
            .await
            .unwrap_or_else(|e| json!({"error": e}));
    }
    if include_network {
        dump["network_log"] = br.get_intercepted_requests().await.unwrap_or(json!([]));
    }

    if mode == "bundle" {
        let artifact_dir = p("artifact_dir")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| default_artifact_dir("browser_page_capture"));
        std::fs::create_dir_all(&artifact_dir)
            .map_err(|e| format!("create artifact_dir {}: {}", artifact_dir, e))?;

        if include_screenshot {
            let path = format!("{}/screenshot.jpg", artifact_dir);
            br.screenshot_to_file(&path, full_page, quality)
                .await
                .map_err(|e| e.to_string())?;
            artifacts.insert("screenshot".into(), json!(path));
        }
        if include_html {
            let html = br.get_html(None).await.map_err(|e| e.to_string())?;
            let path = format!("{}/page.html", artifact_dir);
            std::fs::write(&path, html).map_err(|e| format!("write {}: {}", path, e))?;
            artifacts.insert("html".into(), json!(path));
        }
        if include_text {
            let page_text = dump.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let path = format!("{}/page.txt", artifact_dir);
            std::fs::write(&path, page_text).map_err(|e| format!("write {}: {}", path, e))?;
            artifacts.insert("text".into(), json!(path));
        }
        let dump_path = format!("{}/dump.json", artifact_dir);
        write_json_file(&dump_path, &dump)?;
        artifacts.insert("dump".into(), json!(dump_path));
        dump["artifact_dir"] = json!(artifact_dir);
        dump["artifacts"] = serde_json::Value::Object(artifacts);
    }

    Ok(vec![text(&format_json_text(&dump))])
}

async fn handle_verify_state(
    browser: &SharedBrowser,
    params: &serde_json::Value,
) -> Result<Vec<ToolContent>, String> {
    let p = |key: &str| params.get(key);
    let s = |key: &str| p(key).and_then(|v| v.as_str()).unwrap_or_default();
    let b = |key: &str, def: bool| p(key).and_then(|v| v.as_bool()).unwrap_or(def);
    let i = |key: &str, def: i64| p(key).and_then(|v| v.as_i64()).unwrap_or(def);
    let timeout_ms = i("timeout_ms", 5000).max(0) as u64;
    let poll_ms = i("poll_ms", 250).max(25) as u64;
    let started = Instant::now();

    let expected_text = s("text").to_string();
    let expected_selector = s("selector").to_string();
    let url_contains = s("url_contains").to_string();
    let title_contains = s("title_contains").to_string();

    let mut final_checks = loop {
        let br = browser.read().await;
        let state = page_dump_from_manager(&br, 5000).await.unwrap_or(json!({}));
        let body_text = state.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let url = state.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let title = state.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let mut checks = Vec::new();

        if !expected_text.is_empty() {
            checks.push(json!({"check": "text", "expected": expected_text, "passed": body_text.contains(&expected_text)}));
        }
        if !expected_selector.is_empty() {
            let script = format!(
                "document.querySelector({}) !== null",
                js_string(&expected_selector)
            );
            let exists = br
                .evaluate(&script)
                .await
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            checks.push(
                json!({"check": "selector", "expected": expected_selector, "passed": exists}),
            );
        }
        if !url_contains.is_empty() {
            checks.push(json!({"check": "url_contains", "expected": url_contains, "actual": url, "passed": url.contains(&url_contains)}));
        }
        if !title_contains.is_empty() {
            checks.push(json!({"check": "title_contains", "expected": title_contains, "actual": title, "passed": title.contains(&title_contains)}));
        }

        let passed = !checks.is_empty()
            && checks
                .iter()
                .all(|c| c.get("passed").and_then(|v| v.as_bool()) == Some(true));
        drop(br);

        if passed || started.elapsed().as_millis() as u64 >= timeout_ms {
            break checks;
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    };

    let mut screenshot_path = p("screenshot_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let mut ocr_text = String::new();
    if b("ocr", false) || screenshot_path.is_some() {
        let path = screenshot_path
            .clone()
            .unwrap_or_else(|| format!("C:/temp/browser_verify_{}.jpg", now_millis()));
        browser
            .read()
            .await
            .screenshot_to_file(&path, b("full_page", false), 80)
            .await
            .map_err(|e| e.to_string())?;
        if b("ocr", false) {
            ocr_text = vision_core::ocr_image(&path, "eng")
                .await
                .unwrap_or_else(|e| format!("OCR failed: {}", e));
            if !expected_text.is_empty() {
                final_checks.push(json!({"check": "ocr_text", "expected": expected_text, "passed": ocr_text.contains(&expected_text)}));
            }
        }
        screenshot_path = Some(path);
    }

    let passed = !final_checks.is_empty()
        && final_checks
            .iter()
            .all(|c| c.get("passed").and_then(|v| v.as_bool()) == Some(true));

    Ok(vec![text(&format_json_text(&json!({
        "passed": passed,
        "checks": final_checks,
        "elapsed_ms": started.elapsed().as_millis() as u64,
        "screenshot_path": screenshot_path,
        "ocr_text": if ocr_text.is_empty() { serde_json::Value::Null } else { json!(truncate_chars(&ocr_text, 4000)) }
    })))])
}

async fn handle_smart_navigate(
    browser: &SharedBrowser,
    params: &serde_json::Value,
) -> Result<Vec<ToolContent>, String> {
    let p = |key: &str| params.get(key);
    let s = |key: &str| p(key).and_then(|v| v.as_str()).unwrap_or_default();
    let b = |key: &str, def: bool| p(key).and_then(|v| v.as_bool()).unwrap_or(def);
    let i = |key: &str, def: i64| p(key).and_then(|v| v.as_i64()).unwrap_or(def);
    let url = s("url");
    if url.is_empty() {
        return Err("url is required".into());
    }

    let wait_until = {
        let raw = s("wait_until");
        if raw.is_empty() {
            "networkidle"
        } else {
            raw
        }
    };
    let mut launch_mode = "existing".to_string();
    {
        let mut bw = browser.write().await;
        if !bw.is_alive().await {
            if let Some(port) = p("attach_port").and_then(|v| v.as_u64()) {
                bw.attach(port as u16).await.map_err(|e| e.to_string())?;
                launch_mode = format!("attach:{}", port);
            } else {
                let profile = p("profile_path").and_then(|v| v.as_str()).map(String::from);
                bw.launch(b("headless", true), profile)
                    .await
                    .map_err(|e| e.to_string())?;
                launch_mode = "launch".into();
            }
        }
        bw.navigate(url, wait_until)
            .await
            .map_err(|e| e.to_string())?;
    }

    let wait_idle_ms = i("wait_idle_ms", 2000).max(0) as u64;
    if wait_idle_ms > 0 {
        let _ = browser.read().await.wait_network_idle(wait_idle_ms).await;
    }

    let mut verify = serde_json::Value::Null;
    let verify_text = s("verify_text");
    let verify_selector = s("verify_selector");
    if !verify_text.is_empty() || !verify_selector.is_empty() {
        let verify_args = json!({
            "text": verify_text,
            "selector": verify_selector,
            "timeout_ms": i("timeout_ms", 5000),
            "screenshot_path": p("screenshot_path").cloned().unwrap_or(serde_json::Value::Null)
        });
        let verify_result = handle_verify_state(browser, &verify_args).await?;
        let verify_text = verify_result
            .iter()
            .filter_map(|c| match c {
                ToolContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        verify = serde_json::from_str(&verify_text).unwrap_or(json!({"raw": verify_text}));
    } else if let Some(path) = p("screenshot_path").and_then(|v| v.as_str()) {
        browser
            .read()
            .await
            .screenshot_to_file(path, false, 80)
            .await
            .map_err(|e| e.to_string())?;
        verify = json!({"screenshot_path": path});
    }

    let metrics = browser
        .read()
        .await
        .get_metrics()
        .await
        .map_err(|e| e.to_string())?;
    Ok(vec![text(&format_json_text(&json!({
        "navigated": true,
        "url": url,
        "wait_until": wait_until,
        "launch_mode": launch_mode,
        "metrics": metrics,
        "verify": verify
    })))])
}

pub async fn handle_tool(
    browser: &SharedBrowser,
    name: &str,
    params: serde_json::Value,
) -> ToolResult {
    let result = handle_tool_inner(browser, name, params).await;
    match result {
        Ok(content) => ToolResult {
            content,
            is_error: false,
        },
        Err(e) => ToolResult {
            content: vec![ToolContent::Text {
                text: format!("Error: {}", e),
            }],
            is_error: true,
        },
    }
}

async fn handle_tool_inner(
    browser: &SharedBrowser,
    name: &str,
    params: serde_json::Value,
) -> Result<Vec<ToolContent>, String> {
    let p = |key: &str| params.get(key);
    let s = |key: &str| p(key).and_then(|v| v.as_str()).unwrap_or_default();
    let b = |key: &str, def: bool| p(key).and_then(|v| v.as_bool()).unwrap_or(def);
    let i = |key: &str, def: i64| p(key).and_then(|v| v.as_i64()).unwrap_or(def);

    match name {
        "launch" => {
            let headless = b("headless", true);
            let profile = p("profile_path").and_then(|v| v.as_str()).map(String::from);
            browser
                .write()
                .await
                .launch(headless, profile)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&format!(
                "Browser launched (headless={})",
                headless
            ))])
        }

        "attach" => {
            let port = i("port", 9222) as u16;
            let current_url = browser
                .write()
                .await
                .attach(port)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&format!(
                "Attached to Chrome on port {} (current: {})",
                port, current_url
            ))])
        }

        "debug_launch" => {
            let port = i("port", 9222) as u16;
            let url = p("url").and_then(|v| v.as_str());
            let wait_for_cdp = b("wait_for_cdp", true);
            let result = BrowserManager::debug_launch(port, url, wait_for_cdp)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&result)])
        }

        "close" => {
            browser
                .write()
                .await
                .close()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text("Browser closed")])
        }

        "navigate" => {
            let url = s("url");
            let wait = s("wait_until");
            let wait = if wait.is_empty() { "load" } else { wait };
            let title = browser
                .write()
                .await
                .navigate(url, wait)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&format!("Navigated to: {} ({})", url, title))])
        }

        "smart_navigate" => handle_smart_navigate(browser, &params).await,

        "click" => {
            let do_auto_wait = b("auto_wait", false);
            let wait_ms = i("auto_wait_ms", 5000) as u64;
            // Retry options (subsumes legacy retry_click): clamp retry to [1, 3].
            let retry_total: u64 = i("retry", 1).max(1).min(3) as u64;
            let retry_delay_ms: u64 = i("retry_delay_ms", 500).max(0) as u64;
            let mut attempt: u64 = 0;
            loop {
                attempt += 1;
                let bm = browser.read().await;
                let attempt_result: Result<Vec<ToolContent>, String> = if let Some(a11y_ref) =
                    p("a11y_ref").and_then(|v| v.as_str())
                {
                    let mut selector = lookup_a11y_selector(a11y_ref);
                    if selector.is_none() {
                        let _ = refresh_a11y_snapshot_from_manager(&bm, 200, None).await;
                        selector = lookup_a11y_selector(a11y_ref);
                    }

                    if let Some(sel) = selector {
                        match bm.click_selector(&sel).await {
                            Ok(_) => Ok(vec![text(&format!("Clicked {} via {}", a11y_ref, sel))]),
                            Err(first_err) => {
                                let _ = refresh_a11y_snapshot_from_manager(&bm, 200, None).await;
                                if let Some(fresh_sel) = lookup_a11y_selector(a11y_ref) {
                                    match bm.click_selector(&fresh_sel).await {
                                    Ok(_) => Ok(vec![text(&format!("Clicked {} via refreshed selector {}", a11y_ref, fresh_sel))]),
                                    Err(e) => Err(format!("a11y_ref {} resolved but click failed after refresh: {}; first: {}", a11y_ref, e, first_err)),
                                }
                                } else {
                                    Err(format!(
                                        "a11y_ref {} not found after snapshot refresh",
                                        a11y_ref
                                    ))
                                }
                            }
                        }
                    } else {
                        Err(format!(
                            "a11y_ref {} not found; call a11y_snapshot first",
                            a11y_ref
                        ))
                    }
                } else if let Some(xpath) = p("xpath").and_then(|v| v.as_str()) {
                    let mut wait_err: Option<String> = None;
                    if do_auto_wait {
                        if let Err(e) = bm.auto_wait_xpath(xpath, wait_ms).await {
                            wait_err = Some(e.to_string());
                        }
                    }
                    if let Some(e) = wait_err {
                        Err(e)
                    } else {
                        match bm.click_xpath(xpath).await {
                            Ok(result) => Ok(vec![text(&result)]),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                } else if let Some(sel) = p("selector").and_then(|v| v.as_str()) {
                    let mut wait_err: Option<String> = None;
                    if do_auto_wait {
                        if let Err(e) = bm.auto_wait_selector(sel, wait_ms).await {
                            wait_err = Some(e.to_string());
                        }
                    }
                    if let Some(e) = wait_err {
                        Err(e)
                    } else {
                        match bm.click_selector(sel).await {
                            Ok(_) => Ok(vec![text(&format!("Clicked: {}", sel))]),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                } else if let (Some(x), Some(y)) = (
                    p("x").and_then(|v| v.as_i64()),
                    p("y").and_then(|v| v.as_i64()),
                ) {
                    match bm.click_coords(x as i32, y as i32).await {
                        Ok(_) => Ok(vec![text(&format!("Clicked at ({}, {})", x, y))]),
                        Err(e) => Err(e.to_string()),
                    }
                } else if let Some(match_text) = p("match_text").and_then(|v| v.as_str()) {
                    // Fuzzy text matching - same logic as agent click
                    let script = r#"
                    Array.from(document.querySelectorAll('a, button, input[type="submit"], input[type="button"], [onclick], [role="button"], [role="link"], [role="tab"], [role="menuitem"], summary, label[for], select, [tabindex]'))
                        .filter(el => {
                            const rect = el.getBoundingClientRect();
                            const style = window.getComputedStyle(el);
                            return rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
                        })
                        .slice(0, 200)
                        .map(el => {
                            const rect = el.getBoundingClientRect();
                            return {
                                text: (el.innerText || '').trim().slice(0, 100),
                                aria: el.getAttribute('aria-label') || '',
                                title: el.getAttribute('title') || '',
                                href: el.getAttribute('href') || '',
                                id: el.id || '',
                                name: el.name || '',
                                selector: el.id ? '#' + CSS.escape(el.id) : el.name ? '[name="' + el.name.replace(/"/g, '\\"') + '"]' : null,
                                cx: Math.round(rect.x + rect.width/2),
                                cy: Math.round(rect.y + rect.height/2)
                            };
                        })
                "#;
                    match bm.evaluate(script).await {
                        Ok(elements) => {
                            let match_lower = match_text.to_lowercase();
                            let mut best: Option<(i32, &serde_json::Value)> = None;
                            if let Some(arr) = elements.as_array() {
                                for el in arr {
                                    let get_f = |k: &str| {
                                        el.get(k)
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_lowercase()
                                    };
                                    let (et, ea, etit, ehref, eid, ename) = (
                                        get_f("text"),
                                        get_f("aria"),
                                        get_f("title"),
                                        get_f("href"),
                                        get_f("id"),
                                        get_f("name"),
                                    );
                                    let score = if et == match_lower || ea == match_lower {
                                        100
                                    } else if et.starts_with(&match_lower)
                                        || ea.starts_with(&match_lower)
                                    {
                                        90
                                    } else if et.contains(&match_lower) {
                                        80
                                    } else if ea.contains(&match_lower)
                                        || etit.contains(&match_lower)
                                    {
                                        70
                                    } else if ehref.contains(&match_lower) {
                                        60
                                    } else if eid.contains(&match_lower)
                                        || ename.contains(&match_lower)
                                    {
                                        50
                                    } else if match_lower.split_whitespace().all(|w| et.contains(w))
                                    {
                                        35
                                    } else {
                                        0
                                    };
                                    if score > best.as_ref().map_or(0, |bst| bst.0) {
                                        best = Some((score, el));
                                    }
                                }
                                if let Some((score, el)) = best {
                                    let el_text =
                                        el.get("text").and_then(|v| v.as_str()).unwrap_or("?");
                                    let sel_str =
                                        el.get("selector").and_then(|v| v.as_str()).unwrap_or("");
                                    let cx =
                                        el.get("cx").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                                    let cy =
                                        el.get("cy").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                                    let clicked = if !sel_str.is_empty() {
                                        bm.click_selector(sel_str).await.is_ok()
                                    } else {
                                        false
                                    };
                                    if !clicked {
                                        if let Err(e) = bm.click_coords(cx, cy).await {
                                            Err(e.to_string())
                                        } else {
                                            Ok(vec![text(&format!(
                                                "Clicked '{}' at ({},{}) [score:{}]",
                                                el_text, cx, cy, score
                                            ))])
                                        }
                                    } else {
                                        Ok(vec![text(&format!(
                                            "Clicked '{}' via {} [score:{}]",
                                            el_text, sel_str, score
                                        ))])
                                    }
                                } else {
                                    let available: Vec<String> = arr
                                        .iter()
                                        .filter_map(|el| {
                                            let t = el
                                                .get("text")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            if !t.is_empty() {
                                                Some(t.to_string())
                                            } else {
                                                None
                                            }
                                        })
                                        .take(20)
                                        .collect();
                                    Err(format!(
                                        "No match for '{}'. Available: {:?}",
                                        match_text, available
                                    ))
                                }
                            } else {
                                Err("Failed to scan clickable elements".into())
                            }
                        }
                        Err(e) => Err(e.to_string()),
                    }
                } else {
                    Err("Provide selector, match_text, or x,y coordinates".into())
                };
                // Drop bm read guard before any retry sleep / wait_for that needs another read lock.
                drop(bm);

                match attempt_result {
                    Ok(v) => return Ok(v),
                    Err(e) => {
                        if attempt >= retry_total {
                            return Err(e);
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay_ms))
                            .await;
                        // If we have a selector, wait for it to appear before retrying.
                        if let Some(sel) = p("selector").and_then(|v| v.as_str()) {
                            let bm2 = browser.read().await;
                            let _ = bm2.auto_wait_selector(sel, retry_delay_ms * 2).await;
                        }
                    }
                }
            }
        }

        "page_capture" | "page_dump" => handle_page_capture(browser, name, &params).await,

        "type" | "type_text" => {
            let text_val = s("text");
            let clear = b("clear", true);
            let do_auto_wait = b("auto_wait", false);
            let wait_ms = i("auto_wait_ms", 5000) as u64;
            let sel_direct = p("selector").and_then(|v| v.as_str());
            let xpath_direct = p("xpath").and_then(|v| v.as_str());

            if let Some(xpath) = xpath_direct {
                let bm = browser.read().await;
                if do_auto_wait {
                    bm.auto_wait_xpath(xpath, wait_ms)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                let result = bm
                    .type_xpath(xpath, text_val, clear)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(vec![text(&result)])
            } else if let Some(sel) = sel_direct {
                let bm = browser.read().await;
                if do_auto_wait {
                    bm.auto_wait_selector(sel, wait_ms)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                bm.type_text(sel, text_val, clear)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(vec![text(&format!("Typed into {}", sel))])
            } else if let Some(match_text) = p("match_text").and_then(|v| v.as_str()) {
                // Fuzzy match to find input/textarea
                let script = r#"
                    Array.from(document.querySelectorAll('input:not([type="hidden"]):not([type="submit"]):not([type="button"]), textarea, [contenteditable="true"]'))
                        .filter(el => {
                            const rect = el.getBoundingClientRect();
                            const style = window.getComputedStyle(el);
                            return rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
                        })
                        .slice(0, 100)
                        .map(el => {
                            const label = document.querySelector('label[for="' + el.id + '"]');
                            return {
                                placeholder: el.getAttribute('placeholder') || '',
                                aria: el.getAttribute('aria-label') || '',
                                name: el.name || '',
                                id: el.id || '',
                                label: label ? label.innerText.trim() : '',
                                type: el.type || el.tagName.toLowerCase(),
                                selector: el.id ? '#' + CSS.escape(el.id) : el.name ? '[name="' + el.name.replace(/"/g, '\\"') + '"]' : null
                            };
                        })
                "#;
                let elements = browser
                    .read()
                    .await
                    .evaluate(script)
                    .await
                    .map_err(|e| e.to_string())?;
                let match_lower = match_text.to_lowercase();
                let mut best: Option<(i32, &serde_json::Value)> = None;

                if let Some(arr) = elements.as_array() {
                    for el in arr {
                        let get_f = |k: &str| {
                            el.get(k)
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_lowercase()
                        };
                        let (ph, ar, nm, id, lb) = (
                            get_f("placeholder"),
                            get_f("aria"),
                            get_f("name"),
                            get_f("id"),
                            get_f("label"),
                        );
                        let score = if lb == match_lower || ar == match_lower || ph == match_lower {
                            100
                        } else if lb.contains(&match_lower) || ar.contains(&match_lower) {
                            80
                        } else if ph.contains(&match_lower) {
                            70
                        } else if nm.contains(&match_lower) || id.contains(&match_lower) {
                            60
                        } else if match_lower
                            .split_whitespace()
                            .all(|w| lb.contains(w) || ph.contains(w))
                        {
                            40
                        } else {
                            0
                        };
                        if score > best.as_ref().map_or(0, |b| b.0) {
                            best = Some((score, el));
                        }
                    }
                    if let Some((_score, el)) = best {
                        let sel_str = el.get("selector").and_then(|v| v.as_str()).unwrap_or("");
                        let desc = el.get("label").and_then(|v| v.as_str()).unwrap_or(
                            el.get("placeholder")
                                .and_then(|v| v.as_str())
                                .unwrap_or(el.get("name").and_then(|v| v.as_str()).unwrap_or("?")),
                        );
                        if sel_str.is_empty() {
                            return Err("Found match but no usable selector".into());
                        }
                        browser
                            .read()
                            .await
                            .type_text(sel_str, text_val, clear)
                            .await
                            .map_err(|e| e.to_string())?;
                        Ok(vec![text(&format!("Typed into '{}' ({})", desc, sel_str))])
                    } else {
                        let available: Vec<String> = arr
                            .iter()
                            .filter_map(|el| {
                                let p =
                                    el.get("placeholder").and_then(|v| v.as_str()).unwrap_or("");
                                let l = el.get("label").and_then(|v| v.as_str()).unwrap_or("");
                                let n = el.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let desc = if !l.is_empty() {
                                    l
                                } else if !p.is_empty() {
                                    p
                                } else if !n.is_empty() {
                                    n
                                } else {
                                    return None;
                                };
                                Some(desc.to_string())
                            })
                            .take(15)
                            .collect();
                        Err(format!(
                            "No input match for '{}'. Available: {:?}",
                            match_text, available
                        ))
                    }
                } else {
                    Err("Failed to scan input elements".into())
                }
            } else {
                Err("Provide selector or match_text".into())
            }
        }

        "press" => {
            let key = s("key");
            browser
                .read()
                .await
                .press_key(key)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&format!("Pressed: {}", key))])
        }

        "screenshot" => {
            let full = b("full_page", false);
            let quality = i("quality", 80) as u8;
            let save_path = p("save_path").and_then(|v| v.as_str());
            let do_ocr = b("ocr", false);
            let max_width = p("max_width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let br = browser.read().await;

            if let Some(path) = save_path {
                // Save to file, return path (0 tokens)
                let saved = br
                    .screenshot_to_file(path, full, quality)
                    .await
                    .map_err(|e| e.to_string())?;
                // Resize if max_width specified
                if let Some(mw) = max_width {
                    let bytes =
                        std::fs::read(&saved).map_err(|e| format!("read for resize: {}", e))?;
                    let resized = resize_jpeg_bytes(&bytes, mw, quality)?;
                    std::fs::write(&saved, &resized)
                        .map_err(|e| format!("write resized: {}", e))?;
                }
                if do_ocr {
                    let ocr_text = vision_core::ocr_image(&saved, "eng")
                        .await
                        .unwrap_or_else(|e| format!("OCR failed: {}", e));
                    Ok(vec![text(
                        &json!({"saved": saved, "ocr_text": ocr_text}).to_string(),
                    )])
                } else {
                    Ok(vec![text(&format!("Screenshot saved: {}", saved))])
                }
            } else if let Some(sel) = p("selector").and_then(|v| v.as_str()) {
                let mut screenshot_b64 = br
                    .screenshot_element(sel, quality)
                    .await
                    .map_err(|e| e.to_string())?;
                if let Some(mw) = max_width {
                    screenshot_b64 = resize_b64_jpeg(&screenshot_b64, mw, quality)?;
                }
                if do_ocr {
                    let temp_path = unique_temp_image_path("browser_ocr");
                    let decoded = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        &screenshot_b64,
                    )
                    .map_err(|e| format!("base64 decode: {}", e))?;
                    std::fs::write(&temp_path, &decoded)
                        .map_err(|e| format!("write temp: {}", e))?;
                    let ocr_text = vision_core::ocr_image(&temp_path, "eng")
                        .await
                        .unwrap_or_else(|e| format!("OCR failed: {}", e));
                    let _ = std::fs::remove_file(&temp_path);
                    Ok(vec![
                        text(&json!({"ocr_text": ocr_text}).to_string()),
                        ToolContent::Image {
                            data: screenshot_b64,
                            mime_type: "image/jpeg".into(),
                        },
                    ])
                } else {
                    Ok(vec![ToolContent::Image {
                        data: screenshot_b64,
                        mime_type: "image/jpeg".into(),
                    }])
                }
            } else {
                let mut screenshot_b64 = br
                    .screenshot(full, quality)
                    .await
                    .map_err(|e| e.to_string())?;
                if let Some(mw) = max_width {
                    screenshot_b64 = resize_b64_jpeg(&screenshot_b64, mw, quality)?;
                }
                if do_ocr {
                    let temp_path = unique_temp_image_path("browser_ocr");
                    let decoded = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        &screenshot_b64,
                    )
                    .map_err(|e| format!("base64 decode: {}", e))?;
                    std::fs::write(&temp_path, &decoded)
                        .map_err(|e| format!("write temp: {}", e))?;
                    let ocr_text = vision_core::ocr_image(&temp_path, "eng")
                        .await
                        .unwrap_or_else(|e| format!("OCR failed: {}", e));
                    let _ = std::fs::remove_file(&temp_path);
                    Ok(vec![
                        text(&json!({"ocr_text": ocr_text}).to_string()),
                        ToolContent::Image {
                            data: screenshot_b64,
                            mime_type: "image/jpeg".into(),
                        },
                    ])
                } else {
                    Ok(vec![ToolContent::Image {
                        data: screenshot_b64,
                        mime_type: "image/jpeg".into(),
                    }])
                }
            }
        }

        "screenshot_burst" => {
            let count = i("count", 5) as usize;
            let interval = i("interval_ms", 200) as u64;
            let quality = i("quality", 60) as u8;
            let shots = browser
                .read()
                .await
                .screenshot_burst(count, interval, quality)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(
                &json!({"count": shots.len(), "images": shots}).to_string(),
            )])
        }

        "wait_for" => {
            let timeout = i("timeout_ms", 10000) as u64;
            let condition = p("condition").and_then(|v| v.as_str()).unwrap_or("visible");
            let check_visible = condition != "exists";
            let bm = browser.read().await;
            if let Some(xpath) = p("xpath").and_then(|v| v.as_str()) {
                bm.wait_for_xpath(xpath, timeout, check_visible)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(vec![text(&format!(
                    "XPath {} ({}): {}",
                    if check_visible { "visible" } else { "found" },
                    condition,
                    xpath
                ))])
            } else {
                let sel = s("selector");
                bm.wait_for(sel, timeout, check_visible)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(vec![text(&format!(
                    "Element {} ({}): {}",
                    if check_visible { "visible" } else { "found" },
                    condition,
                    sel
                ))])
            }
        }

        "get_html" => {
            let bm = browser.read().await;
            if let Some(xpath) = p("xpath").and_then(|v| v.as_str()) {
                let script = format!(
                    r#"(() => {{
                        const result = document.evaluate("{}", document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null);
                        const el = result.singleNodeValue;
                        return el ? el.outerHTML : null;
                    }})()"#,
                    xpath.replace('"', r#"\""#)
                );
                let val = bm.evaluate(&script).await.map_err(|e| e.to_string())?;
                let html = val.as_str().unwrap_or("Element not found");
                Ok(vec![text(html)])
            } else {
                let sel = p("selector").and_then(|v| v.as_str());
                let html = bm.get_html(sel).await.map_err(|e| e.to_string())?;
                Ok(vec![text(&html)])
            }
        }

        "get_text" => {
            let bm = browser.read().await;
            if let Some(xpath) = p("xpath").and_then(|v| v.as_str()) {
                let el = bm.resolve_xpath(xpath).await.map_err(|e| e.to_string())?;
                let txt = el.get("text").and_then(|v| v.as_str()).unwrap_or("");
                Ok(vec![text(txt)])
            } else {
                let sel = s("selector");
                let txt = bm.get_text(sel).await.map_err(|e| e.to_string())?;
                Ok(vec![text(&txt)])
            }
        }

        "eval" => {
            let script = s("script");
            let result = browser
                .read()
                .await
                .evaluate(script)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&result.to_string())])
        }

        "scroll" => {
            let dir = s("direction");
            let dir = if dir.is_empty() { "down" } else { dir };
            let amount = i("amount", 500) as i32;
            browser
                .read()
                .await
                .scroll(dir, amount)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&format!("Scrolled {} by {}", dir, amount))])
        }

        "select" => {
            let sel = s("selector");
            let val = s("value");
            browser
                .read()
                .await
                .select_option(sel, val)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&format!("Selected {} in {}", val, sel))])
        }

        "hover" => {
            let sel = s("selector");
            browser
                .read()
                .await
                .hover(sel)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&format!("Hovering: {}", sel))])
        }

        "focus" => {
            let sel = s("selector");
            browser
                .read()
                .await
                .focus(sel)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&format!("Focused: {}", sel))])
        }

        "exists" => {
            let sel = s("selector");
            let exists = browser
                .read()
                .await
                .exists(sel)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(
                &json!({"exists": exists, "selector": sel}).to_string(),
            )])
        }

        "get_bounds" => {
            let sel = s("selector");
            let (x, y, w, h) = browser
                .read()
                .await
                .get_element_bounds(sel)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&json!({"x": x, "y": y, "width": w, "height": h, "center_x": x + w/2.0, "center_y": y + h/2.0}).to_string())])
        }

        "get_clickables" => {
            let elements = browser
                .read()
                .await
                .get_clickables()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(
                &json!({"count": elements.len(), "elements": elements}).to_string(),
            )])
        }

        "a11y_snapshot" => {
            let max_nodes = i("max_nodes", 200).max(1) as usize;
            let root_selector = p("root_selector").and_then(|v| v.as_str());
            let bm = browser.read().await;
            let snapshot =
                refresh_a11y_snapshot_from_manager(&bm, max_nodes, root_selector).await?;
            Ok(vec![text(&format_json_text(&snapshot))])
        }

        "a11y_find" => {
            let role = s("role").to_lowercase();
            let name_query = s("name").to_lowercase();
            let exact = b("exact", false);
            let max_nodes = i("max_nodes", 200).max(1) as usize;
            let bm = browser.read().await;
            let snapshot = refresh_a11y_snapshot_from_manager(&bm, max_nodes, None).await?;
            let nodes = snapshot
                .get("nodes")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let matches: Vec<serde_json::Value> = nodes
                .into_iter()
                .filter(|node| {
                    let node_role = node
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let node_name = node
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let role_ok = role.is_empty() || node_role == role;
                    let name_ok = if name_query.is_empty() {
                        true
                    } else if exact {
                        node_name == name_query
                    } else {
                        node_name.contains(&name_query)
                    };
                    role_ok && name_ok
                })
                .collect();
            Ok(vec![text(&format_json_text(
                &json!({"count": matches.len(), "matches": matches}),
            ))])
        }

        "get_metrics" => {
            let metrics = browser
                .read()
                .await
                .get_metrics()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&metrics.to_string())])
        }

        "verify_state" => handle_verify_state(browser, &params).await,

        "cookies" => {
            let action = s("action");
            let action = if action.is_empty() { "get" } else { action };
            let br = browser.read().await;

            match action {
                "get" => {
                    let cookies = br.get_cookies().await.map_err(|e| e.to_string())?;
                    Ok(vec![text(&json!(cookies).to_string())])
                }
                "set" => {
                    let name = s("name");
                    let value = s("value");
                    let domain = s("domain");
                    drop(br);
                    browser
                        .read()
                        .await
                        .set_cookie(name, value, domain)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(vec![text("Cookie set")])
                }
                "clear" => {
                    br.clear_cookies().await.map_err(|e| e.to_string())?;
                    Ok(vec![text("Cookies cleared")])
                }
                _ => Err("Invalid action".into()),
            }
        }

        "status" => {
            let status = browser.read().await.status();
            Ok(vec![text(&status.to_string())])
        }

        "new_tab" | "new_page" => {
            browser
                .write()
                .await
                .new_page()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text("New page opened")])
        }

        "list_tab" | "list_tabs" => {
            let tabs = browser
                .read()
                .await
                .list_tabs()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(
                &json!({"count": tabs.len(), "tabs": tabs}).to_string(),
            )])
        }

        "switch_tab" => {
            let mut bw = browser.write().await;
            if let Some(index) = p("index").and_then(|v| v.as_u64()) {
                bw.switch_tab_by_index(index as usize)
                    .map_err(|e| e.to_string())?;
            } else if let Some(url_match) = p("url_match").and_then(|v| v.as_str()) {
                bw.switch_tab_by_url(url_match)
                    .await
                    .map_err(|e| e.to_string())?;
            } else {
                return Err("Provide index or url_match".into());
            }
            let tabs = bw.list_tabs().await.map_err(|e| e.to_string())?;
            let active = tabs
                .iter()
                .find(|t| t.get("active").and_then(|a| a.as_bool()).unwrap_or(false));
            let info = active
                .map(|t| {
                    format!(
                        "Switched to tab: {} ({})",
                        t.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                        t.get("title").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .unwrap_or_else(|| "Switched tab".into());
            Ok(vec![text(&info)])
        }

        "close_tab" => {
            let index = i("index", 0) as usize;
            browser
                .write()
                .await
                .close_tab(index)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&format!("Closed tab {}", index))])
        }

        "get_url" => {
            let url = browser
                .read()
                .await
                .get_url()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&url)])
        }

        "back" => {
            browser
                .read()
                .await
                .go_back()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text("Went back")])
        }

        "forward" => {
            browser
                .read()
                .await
                .go_forward()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text("Went forward")])
        }

        "reload" => {
            browser
                .read()
                .await
                .reload()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text("Page reloaded")])
        }

        "get_forms" => {
            let forms = browser
                .read()
                .await
                .get_forms()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&forms.to_string())])
        }

        "fill_form" => {
            let form_sel = s("form_selector");
            let data = p("data").cloned().unwrap_or(json!({}));
            browser
                .read()
                .await
                .fill_form(form_sel, &data)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text("Form filled")])
        }

        "submit_form" => {
            let sel = s("selector");
            browser
                .read()
                .await
                .submit_form(sel)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text("Form submitted")])
        }

        "inject_script" => {
            let script = s("script");
            browser
                .read()
                .await
                .inject_script(script)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text("Script injected")])
        }

        "wait_idle" => {
            let timeout = i("timeout_ms", 2000) as u64;
            browser
                .read()
                .await
                .wait_network_idle(timeout)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text("Network idle")])
        }

        "http_scrape" => {
            let url = s("url");
            let (_, text_content, _) = http_scrape_text(url).await?;
            Ok(vec![text(&text_content)])
        }

        "crawl" => {
            let url = s("url");
            if url.is_empty() {
                return Err("crawl: 'url' is required".into());
            }
            let str_array = |key: &str| -> Vec<String> {
                p(key)
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default()
            };
            let mut opts = crate::crawl::CrawlOptions::new(url);
            opts.max_depth = i("max_depth", 3).max(0) as usize;
            opts.max_pages = i("max_pages", 50).max(1) as usize;
            opts.same_domain_only = b("same_domain", true);
            opts.include = str_array("include");
            opts.exclude = str_array("exclude");
            opts.delay_ms = i("delay_ms", 1000).max(0) as u64;
            opts.wall_clock_secs = i("timeout_secs", 120).clamp(5, 1800) as u64;
            opts.respect_robots = b("respect_robots", true);
            opts.page_text_cap = i("max_chars_per_page", 4000).max(0) as usize;
            if let Some(ua) = p("user_agent").and_then(|v| v.as_str()) {
                if !ua.is_empty() {
                    opts.user_agent = ua.to_string();
                }
            }
            if let Some(px) = p("proxy").and_then(|v| v.as_str()) {
                if !px.is_empty() {
                    opts.proxy = Some(px.to_string());
                }
            }
            let report = crate::crawl::crawl(opts).await?;
            let out = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            Ok(vec![text(&out)])
        }

        "map" => {
            let url = s("url");
            if url.is_empty() {
                return Err("map: 'url' is required".into());
            }
            let max_urls = i("max_urls", 200).max(1) as usize;
            let same_domain = b("same_domain", true);
            let user_agent = p("user_agent")
                .and_then(|v| v.as_str())
                .filter(|t| !t.is_empty())
                .map(String::from)
                .unwrap_or_else(crate::crawl::default_user_agent);
            let proxy = p("proxy")
                .and_then(|v| v.as_str())
                .filter(|t| !t.is_empty())
                .map(String::from)
                .or_else(crate::crawl::proxy_from_env);
            let report =
                crate::crawl::map_site(url, max_urls, same_domain, &user_agent, proxy).await?;
            let out = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            Ok(vec![text(&out)])
        }

        "js_extract" => {
            let url = s("url");
            let selector = p("selector").and_then(|v| v.as_str());
            let engine = p("engine").and_then(|v| v.as_str()).unwrap_or("linkedom");
            let timeout_ms = i("timeout_ms", 5000) as u64;
            let result = run_js_extract(url, selector, engine, timeout_ms)?;
            Ok(vec![text(&format_json_text(&result))])
        }

        "smart_browse" => {
            let url = s("url");
            let selector = p("selector").and_then(|v| v.as_str());
            let started = Instant::now();
            let (extracted_text, tier) = smart_extract(url, selector).await;
            let mut result = json!({
                "text": extracted_text,
                "source_tier": tier,
                "elapsed_ms": started.elapsed().as_millis()
            });
            if tier == "needs_chrome" {
                result["needs_chrome"] = json!(true);
                result["note"] = json!("needs_chrome: true");
            }
            Ok(vec![text(&result.to_string())])
        }

        "scroll_collect" => {
            let max_scrolls = i("max_scrolls", 10).max(1) as usize;
            let wait_ms = i("wait_ms", 2000).max(0) as u64;
            let mut scrolls_performed = 0usize;
            let mut last_height = browser
                .read()
                .await
                .evaluate("document.documentElement.scrollHeight")
                .await
                .map_err(|e| e.to_string())?
                .as_i64()
                .unwrap_or(0);

            for _ in 0..max_scrolls {
                browser
                    .read()
                    .await
                    .evaluate("window.scrollTo(0, document.documentElement.scrollHeight)")
                    .await
                    .map_err(|e| e.to_string())?;
                browser
                    .read()
                    .await
                    .wait_network_idle(wait_ms)
                    .await
                    .map_err(|e| e.to_string())?;
                let new_height = browser
                    .read()
                    .await
                    .evaluate("document.documentElement.scrollHeight")
                    .await
                    .map_err(|e| e.to_string())?
                    .as_i64()
                    .unwrap_or(last_height);
                scrolls_performed += 1;
                if new_height <= last_height {
                    last_height = new_height;
                    break;
                }
                last_height = new_height;
            }

            let extracted = extract_page_content(browser, false, 10000).await?;
            Ok(vec![text(
                &json!({
                    "scrolls_performed": scrolls_performed,
                    "final_scroll_height": last_height,
                    "extracted": extracted
                })
                .to_string(),
            )])
        }

        "wait_stable" => {
            let interval_ms = i("interval_ms", 500).max(1) as u64;
            let max_attempts = i("max_attempts", 10).max(1) as usize;
            std::fs::create_dir_all("C:/temp").map_err(|e| format!("create temp dir: {}", e))?;

            let mut last_size: Option<u64> = None;
            for attempt in 1..=max_attempts {
                let millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::from_millis(0))
                    .as_millis();
                let path = format!("C:/temp/browser_stable_{}_{}.jpg", millis, attempt);
                browser
                    .read()
                    .await
                    .screenshot_to_file(&path, false, 70)
                    .await
                    .map_err(|e| e.to_string())?;
                let current_size = std::fs::metadata(&path)
                    .map_err(|e| format!("stat screenshot: {}", e))?
                    .len();

                if let Some(previous_size) = last_size {
                    let larger = previous_size.max(current_size).max(1);
                    let diff = previous_size.abs_diff(current_size);
                    if diff * 100 <= larger * 5 {
                        return Ok(vec![text(
                            &json!({
                                "stable": true,
                                "attempts": attempt,
                                "previous_size": previous_size,
                                "current_size": current_size,
                                "interval_ms": interval_ms
                            })
                            .to_string(),
                        )]);
                    }
                }

                last_size = Some(current_size);
                tokio::time::sleep(Duration::from_millis(interval_ms)).await;
            }

            Ok(vec![text(
                &json!({
                    "stable": false,
                    "attempts": max_attempts,
                    "last_size": last_size,
                    "interval_ms": interval_ms
                })
                .to_string(),
            )])
        }

        "extract_content" => {
            let url = p("url").and_then(|v| v.as_str());
            let use_current = b("use_current", false);
            let include_links = b("include_links", false);
            let max_length = i("max_length", 10000) as usize;

            if !use_current {
                if let Some(u) = url {
                    // Auto-launch browser if not running
                    {
                        let mut bw = browser.write().await;
                        if !bw.is_alive().await {
                            bw.launch(true, None).await.map_err(|e| e.to_string())?;
                        }
                    }
                    browser
                        .write()
                        .await
                        .navigate(u, "load")
                        .await
                        .map_err(|e| e.to_string())?;
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                } else {
                    return Err("Provide url or set use_current=true".into());
                }
            }

            let parsed = extract_page_content(browser, include_links, max_length).await?;
            Ok(vec![text(&format_json_text(&parsed))])
        }

        "agent" => {
            // DOM-based browser agent - orchestrates existing primitives
            let url = s("url");
            let headless = b("headless", true);
            let save_screenshots = b("save_screenshots", false);
            let profile_path = p("profile_path").and_then(|v| v.as_str()).map(String::from);
            let steps = p("steps")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            if save_screenshots {
                std::fs::create_dir_all("C:/temp/agent_run").ok();
            }

            let mut log: Vec<String> = Vec::new();

            // Launch/attach browser
            {
                let mut bw = browser.write().await;
                if !bw.is_alive().await {
                    bw.launch(headless, profile_path)
                        .await
                        .map_err(|e| e.to_string())?;
                    log.push("Launched browser".into());
                }
                bw.navigate(&url, "load").await.map_err(|e| e.to_string())?;
                log.push(format!("Navigated to {}", url));
            }

            // Wait for initial load
            browser.read().await.wait_network_idle(2000).await.ok();

            for (step_idx, step) in steps.iter().enumerate() {
                let intent = step
                    .get("intent")
                    .and_then(|v| v.as_str())
                    .unwrap_or("click");
                let match_text = step
                    .get("match_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let selector_override = step.get("selector").and_then(|v| v.as_str());
                let data = step.get("data");
                let timeout = step
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10000);

                let step_label = format!("Step {} ({}): '{}'", step_idx + 1, intent, match_text);

                let result: Result<String, String> = match intent {
                    "navigate" => {
                        let target = match_text;
                        browser
                            .write()
                            .await
                            .navigate(target, "load")
                            .await
                            .map(|t| format!("Navigated to {} ({})", target, t))
                            .map_err(|e| e.to_string())
                    }

                    "wait" => {
                        if let Some(sel) = selector_override {
                            browser
                                .read()
                                .await
                                .wait_for(sel, timeout, true)
                                .await
                                .map(|_| format!("Waited for selector: {}", sel))
                                .map_err(|e| e.to_string())
                        } else {
                            browser
                                .read()
                                .await
                                .wait_network_idle(timeout.min(5000))
                                .await
                                .map(|_| "Waited for network idle".into())
                                .map_err(|e| e.to_string())
                        }
                    }

                    "login" => {
                        // Find login form, fill credentials, submit
                        let br = browser.read().await;
                        let forms = br.get_forms().await.map_err(|e| e.to_string())?;

                        if let Some(form_data) = data.and_then(|d| d.as_object()) {
                            // Try to find a form with password field (login form heuristic)
                            let form_idx = forms
                                .as_array()
                                .and_then(|arr| {
                                    arr.iter().position(|f| {
                                        f.get("fields").and_then(|fl| fl.as_array()).map_or(
                                            false,
                                            |fields| {
                                                fields.iter().any(|f| {
                                                    f.get("type").and_then(|t| t.as_str())
                                                        == Some("password")
                                                })
                                            },
                                        )
                                    })
                                })
                                .unwrap_or(0);

                            let form_sel = if let Some(sel) = selector_override {
                                sel.to_string()
                            } else {
                                format!("form:nth-of-type({})", form_idx + 1)
                            };

                            // Type into each field using type_text for proper event triggering
                            for (field_name, field_val) in form_data {
                                if let Some(val) = field_val.as_str() {
                                    // Try name first, then id
                                    let name_sel = format!("[name='{}']", field_name);
                                    let id_sel = format!("#{}", field_name);

                                    // Try name, then id, then type-based fallback
                                    if br.type_text(&name_sel, val, true).await.is_err() {
                                        if br.type_text(&id_sel, val, true).await.is_err() {
                                            let broad = format!(
                                                "input[type='{}']",
                                                if field_name.contains("pass") {
                                                    "password"
                                                } else {
                                                    "text"
                                                }
                                            );
                                            br.type_text(&broad, val, true).await.ok();
                                        }
                                    }
                                }
                            }
                            drop(br);

                            // Submit
                            let br2 = browser.read().await;
                            let _submit_result = br2.evaluate(&format!(
                                "document.querySelector('{}')?.submit() || document.querySelector('button[type=submit], input[type=submit], button')?.click()",
                                form_sel.replace("'", "\'")
                            )).await;
                            drop(br2);

                            // Wait for navigation
                            tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
                            browser.read().await.wait_network_idle(3000).await.ok();

                            Ok(format!("Login attempted via {}", form_sel))
                        } else {
                            Err("login intent requires 'data' with credentials".into())
                        }
                    }

                    "fill" => {
                        if let Some(form_data) = data.and_then(|d| d.as_object()) {
                            let br = browser.read().await;
                            let mut filled = Vec::new();
                            for (field_name, field_val) in form_data {
                                if let Some(val) = field_val.as_str() {
                                    let name_sel = format!("[name='{}']", field_name);
                                    let id_sel = format!("#{}", field_name);
                                    if br.type_text(&name_sel, val, true).await.is_ok() {
                                        filled.push(field_name.clone());
                                    } else if br.type_text(&id_sel, val, true).await.is_ok() {
                                        filled.push(field_name.clone());
                                    }
                                }
                            }
                            Ok(format!("Filled fields: {}", filled.join(", ")))
                        } else {
                            Err("fill intent requires 'data'".into())
                        }
                    }

                    "click" => {
                        if let Some(sel) = selector_override {
                            browser
                                .read()
                                .await
                                .click_selector(sel)
                                .await
                                .map(|_| format!("Clicked selector: {}", sel))
                                .map_err(|e| e.to_string())
                        } else {
                            // Enhanced DOM scan with rich attributes for fuzzy matching
                            let script = r#"
                                Array.from(document.querySelectorAll('a, button, input[type="submit"], input[type="button"], [onclick], [role="button"], [role="link"], [role="tab"], [role="menuitem"], summary, label[for], select, textarea, [tabindex]'))
                                    .filter(el => {
                                        const rect = el.getBoundingClientRect();
                                        const style = window.getComputedStyle(el);
                                        return rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
                                    })
                                    .slice(0, 200)
                                    .map((el, i) => {
                                        const rect = el.getBoundingClientRect();
                                        return {
                                            tag: el.tagName.toLowerCase(),
                                            text: (el.innerText || '').trim().slice(0, 100),
                                            value: el.value || '',
                                            aria: el.getAttribute('aria-label') || '',
                                            title: el.getAttribute('title') || '',
                                            href: el.getAttribute('href') || '',
                                            id: el.id || '',
                                            name: el.name || '',
                                            cls: el.className ? String(el.className).slice(0, 80) : '',
                                            selector: el.id ? '#' + CSS.escape(el.id)
                                                : el.name ? '[name="' + el.name.replace(/"/g, '\\"') + '"]'
                                                : null,
                                            cx: Math.round(rect.x + rect.width/2),
                                            cy: Math.round(rect.y + rect.height/2)
                                        };
                                    })
                            "#;
                            let elements = browser
                                .read()
                                .await
                                .evaluate(script)
                                .await
                                .map_err(|e| e.to_string())?;

                            let match_lower = match_text.to_lowercase();
                            let mut best: Option<(i32, &serde_json::Value)> = None;

                            if let Some(arr) = elements.as_array() {
                                for el in arr {
                                    let fields: Vec<String> = [
                                        "text", "aria", "title", "value", "href", "id", "name",
                                        "cls",
                                    ]
                                    .iter()
                                    .map(|k| {
                                        el.get(k)
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_lowercase()
                                    })
                                    .collect();

                                    let [text, aria, title, value, href, id, name, cls] =
                                        <[String; 8]>::try_from(fields).unwrap_or_default();

                                    // Tiered scoring: exact > contains-primary > contains-secondary > word-match
                                    let score = if text == match_lower
                                        || aria == match_lower
                                        || value == match_lower
                                    {
                                        100
                                    } else if text.starts_with(&match_lower)
                                        || aria.starts_with(&match_lower)
                                    {
                                        90
                                    } else if text.contains(&match_lower) {
                                        80
                                    } else if aria.contains(&match_lower)
                                        || title.contains(&match_lower)
                                    {
                                        70
                                    } else if href.contains(&match_lower) {
                                        60
                                    } else if id.contains(&match_lower)
                                        || name.contains(&match_lower)
                                    {
                                        50
                                    } else if cls.contains(&match_lower) {
                                        40
                                    } else if match_lower
                                        .split_whitespace()
                                        .all(|w| text.contains(w))
                                    {
                                        35
                                    } else if !text.is_empty() && match_lower.contains(&text) {
                                        30
                                    } else {
                                        0
                                    };

                                    if score > best.as_ref().map_or(0, |b| b.0) {
                                        best = Some((score, el));
                                    }
                                }

                                if let Some((score, el)) = best {
                                    let el_text =
                                        el.get("text").and_then(|v| v.as_str()).unwrap_or("?");
                                    let cx =
                                        el.get("cx").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                                    let cy =
                                        el.get("cy").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                                    let br = browser.read().await;

                                    // Try CSS selector first (more reliable), fall back to coords
                                    let sel_str =
                                        el.get("selector").and_then(|v| v.as_str()).unwrap_or("");
                                    let clicked = if !sel_str.is_empty() {
                                        br.click_selector(sel_str).await.is_ok()
                                    } else {
                                        false
                                    };

                                    if !clicked {
                                        br.click_coords(cx, cy).await.map_err(|e| e.to_string())?;
                                    }
                                    tokio::time::sleep(tokio::time::Duration::from_millis(500))
                                        .await;
                                    Ok(format!(
                                        "Clicked '{}' {} [score:{}]",
                                        el_text,
                                        if clicked {
                                            format!("via {}", sel_str)
                                        } else {
                                            format!("at ({},{})", cx, cy)
                                        },
                                        score
                                    ))
                                } else {
                                    let available: Vec<String> = arr
                                        .iter()
                                        .filter_map(|el| {
                                            let t = el
                                                .get("text")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            let a = el
                                                .get("aria")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            let h = el
                                                .get("href")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            let label = if !t.is_empty() {
                                                t
                                            } else if !a.is_empty() {
                                                a
                                            } else if !h.is_empty() {
                                                h
                                            } else {
                                                return None;
                                            };
                                            Some(label.to_string())
                                        })
                                        .take(25)
                                        .collect();
                                    Err(format!(
                                        "No match for '{}'. Available: {:?}",
                                        match_text, available
                                    ))
                                }
                            } else {
                                Err("Failed to scan clickable elements".into())
                            }
                        }
                    }

                    "select" => {
                        if let Some(sel) = selector_override {
                            browser
                                .read()
                                .await
                                .select_option(sel, match_text)
                                .await
                                .map(|_| format!("Selected '{}' in {}", match_text, sel))
                                .map_err(|e| e.to_string())
                        } else {
                            // Find select element containing option with match text
                            let script = format!(
                                r#"(() => {{
                                    const opts = Array.from(document.querySelectorAll('option'));
                                    const match = opts.find(o => o.text.toLowerCase().includes('{}'));
                                    if (match) {{
                                        const sel = match.closest('select');
                                        if (sel) {{ sel.value = match.value; sel.dispatchEvent(new Event('change')); return 'Selected: ' + match.text; }}
                                    }}
                                    return null;
                                }})()"#,
                                match_text.to_lowercase().replace("'", "\'")
                            );
                            let result = browser
                                .read()
                                .await
                                .evaluate(&script)
                                .await
                                .map_err(|e| e.to_string())?;
                            if result.is_null() {
                                Err(format!("No select option matching '{}'", match_text))
                            } else {
                                Ok(result.as_str().unwrap_or("Selected").to_string())
                            }
                        }
                    }

                    "extract" => {
                        let br = browser.read().await;
                        if let Some(sel) = selector_override {
                            let txt = br.get_text(sel).await.map_err(|e| e.to_string())?;
                            Ok(format!(
                                "Extracted from {}: {}",
                                sel,
                                &txt[..txt.len().min(2000)]
                            ))
                        } else if match_text.to_lowercase().contains("table") {
                            // Extract tables as JSON
                            let script = r#"
                                Array.from(document.querySelectorAll('table')).slice(0, 5).map((t, i) => {
                                    const headers = Array.from(t.querySelectorAll('th')).map(h => h.innerText.trim());
                                    const rows = Array.from(t.querySelectorAll('tbody tr')).slice(0, 50).map(r =>
                                        Array.from(r.querySelectorAll('td')).map(c => c.innerText.trim())
                                    );
                                    return {table: i, headers, rows: rows.length, sample: rows.slice(0, 5)};
                                })
                            "#;
                            let tables = br.evaluate(script).await.map_err(|e| e.to_string())?;
                            Ok(format!(
                                "Tables: {}",
                                serde_json::to_string_pretty(&tables).unwrap_or_default()
                            ))
                        } else if match_text.to_lowercase().contains("link") {
                            let script = r#"
                                Array.from(document.querySelectorAll('a[href]')).slice(0, 50).map(a => ({
                                    text: a.innerText.trim().slice(0, 80),
                                    href: a.href
                                })).filter(a => a.text)
                            "#;
                            let links = br.evaluate(script).await.map_err(|e| e.to_string())?;
                            Ok(format!(
                                "Links: {}",
                                serde_json::to_string_pretty(&links).unwrap_or_default()
                            ))
                        } else {
                            // Extract visible text matching the pattern
                            let script = format!(
                                r#"(() => {{
                                    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
                                    const matches = [];
                                    while (walker.nextNode()) {{
                                        const t = walker.currentNode.textContent.trim();
                                        if (t.toLowerCase().includes('{}')) matches.push(t);
                                    }}
                                    return matches.slice(0, 20);
                                }})()"#,
                                match_text.to_lowercase().replace("'", "\'")
                            );
                            let result = br.evaluate(&script).await.map_err(|e| e.to_string())?;
                            Ok(format!(
                                "Extracted: {}",
                                serde_json::to_string_pretty(&result).unwrap_or_default()
                            ))
                        }
                    }

                    _ => Err(format!("Unknown intent: {}", intent)),
                };

                match result {
                    Ok(msg) => {
                        log.push(format!("âœ“ {} â†’ {}", step_label, msg));

                        if save_screenshots {
                            let path = format!("C:/temp/agent_run/step_{:02}.jpg", step_idx + 1);
                            browser
                                .read()
                                .await
                                .screenshot_to_file(&path, false, 60)
                                .await
                                .ok();
                        }
                    }
                    Err(e) => {
                        log.push(format!("âœ— {} â†’ ERROR: {}", step_label, e));

                        // Save failure screenshot
                        let fail_path =
                            format!("C:/temp/agent_run/fail_step_{:02}.jpg", step_idx + 1);
                        std::fs::create_dir_all("C:/temp/agent_run").ok();
                        browser
                            .read()
                            .await
                            .screenshot_to_file(&fail_path, false, 80)
                            .await
                            .ok();

                        // Return partial results with DOM snapshot
                        let clickables = browser
                            .read()
                            .await
                            .get_clickables()
                            .await
                            .unwrap_or_default();
                        let available: Vec<String> = clickables
                            .iter()
                            .filter_map(|el| el.get("text").and_then(|t| t.as_str()))
                            .filter(|t| !t.is_empty())
                            .take(30)
                            .map(|t| t.to_string())
                            .collect();

                        let current_url = browser.read().await.get_url().await.unwrap_or_default();

                        log.push(format!("ðŸ“ Stopped at: {}", current_url));
                        log.push(format!("ðŸ“¸ Screenshot: {}", fail_path));
                        log.push(format!("ðŸ” Available elements: {:?}", available));

                        return Ok(vec![text(
                            &json!({
                                "status": "partial",
                                "completed_steps": step_idx,
                                "total_steps": steps.len(),
                                "log": log,
                                "current_url": current_url,
                                "available_elements": available,
                                "screenshot": fail_path
                            })
                            .to_string(),
                        )]);
                    }
                }
            }

            // All steps complete
            let current_url = browser.read().await.get_url().await.unwrap_or_default();
            Ok(vec![text(
                &json!({
                    "status": "complete",
                    "steps_completed": steps.len(),
                    "log": log,
                    "final_url": current_url
                })
                .to_string(),
            )])
        }

        "verify_visual" => {
            let expected = s("expected_text");
            let br = browser.read().await;
            let screenshot_b64 = if let Some(sel) = p("selector").and_then(|v| v.as_str()) {
                br.screenshot_element(sel, 80)
                    .await
                    .map_err(|e| e.to_string())?
            } else {
                br.screenshot(false, 80).await.map_err(|e| e.to_string())?
            };
            let temp_path = unique_temp_image_path("browser_verify");
            let decoded =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &screenshot_b64)
                    .map_err(|e| format!("base64 decode: {}", e))?;
            std::fs::write(&temp_path, &decoded).map_err(|e| format!("write temp: {}", e))?;
            let ocr_text = vision_core::ocr_image(&temp_path, "eng")
                .await
                .unwrap_or_else(|e| format!("OCR failed: {}", e));
            let _ = std::fs::remove_file(&temp_path);
            let pass = ocr_text.to_lowercase().contains(&expected.to_lowercase());
            Ok(vec![text(
                &json!({"pass": pass, "expected": expected, "ocr_text": ocr_text}).to_string(),
            )])
        }

        "iframe_extract" => {
            let target_idx = params
                .get("target_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let include_all = params
                .get("include_all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let br = browser.read().await;
            let script = r#"
                (() => {
                    const frames = Array.from(document.querySelectorAll('iframe'));
                    return JSON.stringify(frames.map((f, i) => {
                        let text = '';
                        let crossOrigin = false;
                        try { text = f.contentDocument ? f.contentDocument.body.innerText : ''; }
                        catch(e) { crossOrigin = true; }
                        const rect = f.getBoundingClientRect();
                        return {index: i, src: f.src, crossOrigin, text: text.slice(0, 5000), x: rect.x, y: rect.y, w: rect.width, h: rect.height};
                    }));
                })()
            "#;
            let result = br.evaluate(script).await.map_err(|e| e.to_string())?;
            let result_str = result.as_str().unwrap_or(&result.to_string()).to_string();
            let cleaned = result_str
                .trim_matches('"')
                .replace("\\\"", "\"")
                .replace("\\n", "\n");
            if let Ok(frames) = serde_json::from_str::<Vec<serde_json::Value>>(&cleaned) {
                if include_all {
                    Ok(vec![text(
                        &serde_json::json!({"iframes": frames, "count": frames.len()}).to_string(),
                    )])
                } else {
                    let idx = target_idx.unwrap_or(0);
                    if let Some(frame) = frames.get(idx) {
                        Ok(vec![text(&frame.to_string())])
                    } else {
                        Ok(vec![text(&serde_json::json!({"error": "No iframe at index", "available": frames.len()}).to_string())])
                    }
                }
            } else {
                Ok(vec![text(&serde_json::json!({"raw": cleaned}).to_string())])
            }
        }

        "bulk_extract" => {
            let urls: Vec<String> = params
                .get("urls")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let max_len = params
                .get("max_length_per_page")
                .and_then(|v| v.as_u64())
                .unwrap_or(5000) as usize;
            let selector = params.get("selector").and_then(|v| v.as_str());
            let mut results: Vec<serde_json::Value> = Vec::new();
            let start = std::time::Instant::now();
            for url in &urls {
                let (mut extracted, tier) = smart_extract(url, selector).await;
                if extracted.len() > max_len {
                    extracted.truncate(max_len);
                }
                results.push(serde_json::json!({"url": url, "text": extracted, "length": extracted.len(), "tier": tier}));
            }
            let elapsed = start.elapsed().as_millis();
            Ok(vec![text(&serde_json::json!({"results": results, "total_urls": urls.len(), "elapsed_ms": elapsed}).to_string())])
        }
        "assemble" => {
            let result = crate::planner::assemble(&params);
            Ok(vec![text(
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )])
        }
        "plan" => {
            let result = crate::planner::plan(&params);
            Ok(vec![text(
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )])
        }

        // ===== P4: Script Batching =====
        "script" => {
            let steps: Vec<serde_json::Value> = params
                .get("steps")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let vars: serde_json::Map<String, serde_json::Value> = params
                .get("vars")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let stop_on_error = b("stop_on_error", true);
            let step_delay = i("step_delay_ms", 0) as u64;

            if steps.is_empty() {
                return Err("No steps provided".into());
            }

            // Variable substitution function
            fn substitute_vars(
                val: &serde_json::Value,
                vars: &serde_json::Map<String, serde_json::Value>,
            ) -> serde_json::Value {
                match val {
                    serde_json::Value::String(s) => {
                        let mut result = s.clone();
                        for (key, replacement) in vars {
                            let placeholder = format!("{{{{{}}}}}", key);
                            let rep_str = match replacement {
                                serde_json::Value::String(rs) => rs.clone(),
                                other => other.to_string(),
                            };
                            result = result.replace(&placeholder, &rep_str);
                        }
                        serde_json::Value::String(result)
                    }
                    serde_json::Value::Object(map) => {
                        let mut new_map = serde_json::Map::new();
                        for (k, v) in map {
                            new_map.insert(k.clone(), substitute_vars(v, vars));
                        }
                        serde_json::Value::Object(new_map)
                    }
                    serde_json::Value::Array(arr) => serde_json::Value::Array(
                        arr.iter().map(|v| substitute_vars(v, vars)).collect(),
                    ),
                    other => other.clone(),
                }
            }

            let mut log: Vec<serde_json::Value> = Vec::new();
            let script_start = Instant::now();

            for (step_idx, step) in steps.iter().enumerate() {
                let tool_name = step.get("tool").and_then(|v| v.as_str()).unwrap_or("");
                let step_params = step.get("params").cloned().unwrap_or(json!({}));
                let resolved_params = substitute_vars(&step_params, &vars);

                if tool_name.is_empty() {
                    log.push(json!({"step": step_idx + 1, "status": "skipped", "reason": "no tool name"}));
                    continue;
                }

                // Trace logging if active
                {
                    let mut bm_trace = browser.write().await;
                    bm_trace.trace_log(
                        "script_step",
                        tool_name,
                        Some(json!({"step": step_idx + 1, "params": &resolved_params})),
                    );
                }

                let step_start = Instant::now();
                let result = Box::pin(handle_tool_inner(browser, tool_name, resolved_params)).await;
                let step_elapsed = step_start.elapsed().as_millis();

                match result {
                    Ok(content) => {
                        let output: Vec<String> = content
                            .iter()
                            .map(|c| match c {
                                ToolContent::Text { text: t } => t.clone(),
                                _ => "[non-text]".into(),
                            })
                            .collect();
                        log.push(json!({
                            "step": step_idx + 1,
                            "tool": tool_name,
                            "status": "ok",
                            "elapsed_ms": step_elapsed,
                            "output": output.join("\n").chars().take(500).collect::<String>()
                        }));
                    }
                    Err(err) => {
                        log.push(json!({
                            "step": step_idx + 1,
                            "tool": tool_name,
                            "status": "error",
                            "error": err,
                            "elapsed_ms": step_elapsed
                        }));
                        if stop_on_error {
                            // Save failure screenshot
                            let fail_path =
                                format!("C:/temp/script_run/fail_step_{:02}.jpg", step_idx + 1);
                            std::fs::create_dir_all("C:/temp/script_run").ok();
                            browser
                                .read()
                                .await
                                .screenshot_to_file(&fail_path, false, 80)
                                .await
                                .ok();
                            let current_url =
                                browser.read().await.get_url().await.unwrap_or_default();
                            return Ok(vec![text(
                                &json!({
                                    "status": "partial",
                                    "completed": step_idx,
                                    "total": steps.len(),
                                    "elapsed_ms": script_start.elapsed().as_millis() as u64,
                                    "log": log,
                                    "current_url": current_url,
                                    "screenshot": fail_path
                                })
                                .to_string(),
                            )]);
                        }
                    }
                }

                if step_delay > 0 && step_idx < steps.len() - 1 {
                    tokio::time::sleep(Duration::from_millis(step_delay)).await;
                }
            }

            let current_url = browser.read().await.get_url().await.unwrap_or_default();
            Ok(vec![text(
                &json!({
                    "status": "complete",
                    "steps_completed": steps.len(),
                    "elapsed_ms": script_start.elapsed().as_millis() as u64,
                    "log": log,
                    "final_url": current_url
                })
                .to_string(),
            )])
        }

        // ===== P1: Network Interception =====
        "route" => {
            let pattern = s("pattern").to_string();
            let action_str = s("action");
            let action_str = if action_str.is_empty() {
                "log"
            } else {
                action_str
            };

            let action = match action_str {
                "block" => RouteAction::Block,
                "mock" => {
                    let status = i("mock_status", 200) as u16;
                    let content_type = s("mock_content_type");
                    let content_type = if content_type.is_empty() {
                        "application/json"
                    } else {
                        content_type
                    };
                    let body = s("mock_body").to_string();
                    RouteAction::Mock {
                        status,
                        content_type: content_type.to_string(),
                        body,
                    }
                }
                _ => RouteAction::Log,
            };

            let result = browser
                .write()
                .await
                .add_route(pattern, action)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&result)])
        }
        "route_remove" => {
            let pattern = s("pattern");
            let result = browser
                .write()
                .await
                .remove_route(pattern)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&result)])
        }
        "route_list" => {
            let routes = browser.read().await.list_routes();
            Ok(vec![text(
                &json!({"routes": routes, "count": routes.len()}).to_string(),
            )])
        }
        "route_clear" => {
            let result = browser
                .write()
                .await
                .disable_interception()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&result)])
        }
        "get_network_log" => {
            let do_clear = b("clear", false);
            let bm_read = browser.read().await;
            let log = bm_read
                .get_intercepted_requests()
                .await
                .map_err(|e| e.to_string())?;
            if do_clear {
                drop(bm_read);
                browser
                    .read()
                    .await
                    .clear_intercepted()
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Ok(vec![text(&log.to_string())])
        }

        // ===== P5: Browser Contexts =====
        "context_create" => {
            let name = s("name").to_string();
            let url = p("url").and_then(|v| v.as_str());
            let result = browser
                .write()
                .await
                .create_context(&name, url)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&result)])
        }
        "context_switch" => {
            let name = s("name");
            let result = browser
                .write()
                .await
                .switch_context(name)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&result)])
        }
        "context_destroy" => {
            let name = s("name");
            let result = browser
                .write()
                .await
                .destroy_context(name)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&result)])
        }
        "context_list" => {
            let contexts = browser.read().await.list_contexts();
            Ok(vec![text(&json!({"contexts": contexts}).to_string())])
        }

        // ===== P2: Trace Recording =====
        "trace_start" => {
            let result = browser
                .write()
                .await
                .trace_start()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&result)])
        }
        "trace_stop" => {
            let trace = browser
                .write()
                .await
                .trace_stop()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(
                &serde_json::to_string_pretty(&trace).unwrap_or_default(),
            )])
        }
        "trace_save" => {
            let path = s("path");
            let result = browser
                .write()
                .await
                .trace_save(path)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![text(&result)])
        }

        // ===== P6: Spec-Driven Evaluation =====
        "evaluate" => {
            let target = s("target").to_string();
            let intent = s("intent").to_string();
            let evidence = b("evidence", true);
            let spec: Vec<serde_json::Value> = p("spec")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let eval_start = Instant::now();

            // Create evidence directory
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let evidence_dir = format!("C:/temp/eval_{}", ts);
            if evidence {
                std::fs::create_dir_all(&evidence_dir).ok();
            }

            // Navigate to target
            Box::pin(handle_tool_inner(
                browser,
                "navigate",
                json!({"url": target}),
            ))
            .await
            .map_err(|e| format!("Navigate to target failed: {}", e))?;

            // Wait for page load
            Box::pin(handle_tool_inner(browser, "wait_idle", json!({})))
                .await
                .ok();

            // If no spec: auto-discovery mode
            if spec.is_empty() {
                let br = browser.read().await;
                let mut auto_checks: Vec<serde_json::Value> = Vec::new();

                // 1. Check page loaded (readyState)
                let metrics_js = r#"JSON.stringify({
                    readyState: document.readyState,
                    elementCount: document.querySelectorAll('*').length,
                    scriptCount: document.querySelectorAll('script').length,
                    styleCount: document.querySelectorAll('link[rel=stylesheet], style').length,
                    iframeCount: document.querySelectorAll('iframe').length,
                    formCount: document.querySelectorAll('form').length,
                    url: window.location.href,
                    title: document.title
                })"#;
                let metrics_val = br.evaluate(metrics_js).await.ok();
                let metrics_str = metrics_val
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");
                let metrics: serde_json::Value =
                    serde_json::from_str(metrics_str).unwrap_or(json!({}));

                let ready_state = metrics
                    .get("readyState")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let page_title = metrics.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let page_url = metrics
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&target);
                let element_count = metrics
                    .get("elementCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let page_loaded = ready_state == "complete" || ready_state == "interactive";
                auto_checks.push(json!({
                    "check": "page_loaded", "passed": page_loaded,
                    "detail": format!("readyState={}", ready_state)
                }));

                // 2. Check title
                let has_title = !page_title.is_empty();
                auto_checks.push(json!({
                    "check": "has_title", "passed": has_title,
                    "detail": if has_title { page_title.to_string() } else { "(empty)".into() }
                }));

                // 3. Check for error elements on page (proxy for console errors)
                let error_js = r#"(function(){
                    var els = document.querySelectorAll('.error, [class*=error], [class*=Error], [role=alert]');
                    return els.length;
                })()"#;
                let error_count = br
                    .evaluate(error_js)
                    .await
                    .ok()
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let no_errors = error_count == 0;
                auto_checks.push(json!({
                    "check": "no_error_elements", "passed": no_errors,
                    "detail": format!("{} error elements", error_count)
                }));

                // 4. Check broken images
                let broken_img_js = r#"JSON.stringify(
                    Array.from(document.images).filter(function(img){return !img.complete || img.naturalWidth === 0})
                    .map(function(img){return {src: img.src, alt: img.alt || '(no alt)'}})
                )"#;
                let broken_imgs_str = br
                    .evaluate(broken_img_js)
                    .await
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or("[]".into());
                let broken_imgs: Vec<serde_json::Value> =
                    serde_json::from_str(&broken_imgs_str).unwrap_or_default();
                let total_imgs = br
                    .evaluate("document.images.length")
                    .await
                    .ok()
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let imgs_ok = broken_imgs.is_empty();
                auto_checks.push(json!({
                    "check": "no_broken_images", "passed": imgs_ok,
                    "detail": format!("{}/{} broken", broken_imgs.len(), total_imgs)
                }));

                // 5. Check links without href
                let bad_links_js = r#"JSON.stringify(
                    Array.from(document.querySelectorAll('a')).filter(function(a){return !a.href || a.href === '' || a.href === window.location.href + '#'})
                    .map(function(a){return {text: a.textContent.trim().slice(0,50), href: a.href || '(none)'}})
                )"#;
                let bad_links_str = br
                    .evaluate(bad_links_js)
                    .await
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or("[]".into());
                let bad_links: Vec<serde_json::Value> =
                    serde_json::from_str(&bad_links_str).unwrap_or_default();
                let links_ok = bad_links.is_empty();
                auto_checks.push(json!({
                    "check": "links_valid", "passed": links_ok,
                    "detail": format!("{} bad links", bad_links.len())
                }));

                // 6. Check form inputs without labels
                let unlabeled_js = r#"JSON.stringify(
                    Array.from(document.querySelectorAll('input, select, textarea')).filter(function(el){
                        var id = el.id;
                        var hasLabel = id && document.querySelector('label[for="' + id + '"]');
                        var hasAriaLabel = el.getAttribute('aria-label');
                        var hasPlaceholder = el.placeholder;
                        return !hasLabel && !hasAriaLabel && !hasPlaceholder;
                    }).map(function(el){return {type: el.type || el.tagName.toLowerCase(), name: el.name || '(unnamed)'}})
                )"#;
                let unlabeled_str = br
                    .evaluate(unlabeled_js)
                    .await
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or("[]".into());
                let unlabeled: Vec<serde_json::Value> =
                    serde_json::from_str(&unlabeled_str).unwrap_or_default();
                let inputs_labeled = unlabeled.is_empty();
                let unlabeled_detail = if unlabeled.is_empty() {
                    "all labeled".to_string()
                } else {
                    let items: Vec<String> = unlabeled
                        .iter()
                        .map(|u| {
                            let t = u.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                            let n = u.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                            format!("(type={}, name={})", t, n)
                        })
                        .collect();
                    format!("{} unlabeled: {}", unlabeled.len(), items.join(", "))
                };
                auto_checks.push(json!({
                    "check": "inputs_labeled", "passed": inputs_labeled,
                    "detail": unlabeled_detail
                }));

                // Drop the read lock before calling handle_tool_inner (needs write access)
                drop(br);

                // 7. Get a11y snapshot for interactive element counts
                let snapshot = Box::pin(handle_tool_inner(browser, "get_clickables", json!({})))
                    .await
                    .unwrap_or_else(|_| vec![text("(snapshot unavailable)")]);
                let snapshot_text: String = snapshot
                    .iter()
                    .map(|c| match c {
                        ToolContent::Text { text: t } => t.clone(),
                        _ => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                // Count interactive elements from snapshot text
                let count_role = |role: &str| -> u32 {
                    snapshot_text.matches(&format!("[{}]", role)).count() as u32
                        + snapshot_text.matches(&format!("role={}", role)).count() as u32
                };
                let buttons = count_role("button");
                let links = count_role("link");
                let inputs =
                    count_role("textbox") + count_role("combobox") + count_role("searchbox");
                let checkboxes = count_role("checkbox") + count_role("radio");
                let interactive_total = buttons + links + inputs + checkboxes;

                let a11y_parts: Vec<String> = [
                    (buttons, "buttons"),
                    (links, "links"),
                    (inputs, "inputs"),
                    (checkboxes, "checkboxes"),
                ]
                .iter()
                .filter(|(n, _)| *n > 0)
                .map(|(n, label)| format!("{} {}", n, label))
                .collect();
                let a11y_summary = if a11y_parts.is_empty() {
                    "no interactive elements".into()
                } else {
                    a11y_parts.join(", ")
                };

                // 8. Screenshot
                let mut screenshot_path = String::new();
                if evidence {
                    let path = format!("{}/initial.jpg", evidence_dir);
                    browser
                        .read()
                        .await
                        .screenshot_to_file(&path, false, 80)
                        .await
                        .ok();
                    screenshot_path = path;
                }

                // Determine result
                let result_str = if !page_loaded || page_title.is_empty() {
                    "fail"
                } else if !imgs_ok || !links_ok || !inputs_labeled || !no_errors {
                    "warning"
                } else {
                    "pass"
                };

                return Ok(vec![text(&json!({
                    "result": result_str,
                    "mode": "auto_discovery",
                    "target": target,
                    "intent": intent,
                    "page": {
                        "title": page_title,
                        "url": page_url,
                        "ready_state": ready_state,
                        "element_count": element_count,
                        "script_count": metrics.get("scriptCount").and_then(|v| v.as_u64()).unwrap_or(0),
                        "style_count": metrics.get("styleCount").and_then(|v| v.as_u64()).unwrap_or(0),
                        "form_count": metrics.get("formCount").and_then(|v| v.as_u64()).unwrap_or(0)
                    },
                    "interactive_elements": {
                        "buttons": buttons,
                        "links": links,
                        "inputs": inputs,
                        "checkboxes": checkboxes,
                        "total": interactive_total
                    },
                    "checks": auto_checks,
                    "a11y_summary": a11y_summary,
                    "screenshot": screenshot_path,
                    "evidence_dir": evidence_dir,
                    "duration_ms": eval_start.elapsed().as_millis() as u64
                }).to_string())]);
            }

            // Run spec steps
            let mut checks: Vec<serde_json::Value> = Vec::new();
            let mut passed_count = 0u32;
            let mut failed_count = 0u32;

            for (step_idx, step) in spec.iter().enumerate() {
                let tool_name = step.get("tool").and_then(|v| v.as_str()).unwrap_or("");
                let step_params = step.get("params").cloned().unwrap_or(json!({}));
                let assert_def = step.get("assert");

                // Execute the tool step
                let step_result = if !tool_name.is_empty() {
                    Box::pin(handle_tool_inner(browser, tool_name, step_params)).await
                } else {
                    Ok(vec![text("(no tool specified)")])
                };

                let step_ok = step_result.is_ok();
                let step_output = match &step_result {
                    Ok(content) => content
                        .iter()
                        .map(|c| match c {
                            ToolContent::Text { text: t } => t.clone(),
                            _ => "[non-text]".into(),
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    Err(e) => format!("Error: {}", e),
                };

                // Run assertion if present
                let mut check_passed = step_ok;
                let mut expected_str = String::new();
                let mut actual_str = step_output.chars().take(500).collect::<String>();
                let mut assert_type = String::new();
                let mut assert_target = String::new();

                if let Some(assert) = assert_def {
                    assert_type = assert
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    assert_target = assert
                        .get("target")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    expected_str = assert
                        .get("expected")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    match assert_type.as_str() {
                        "text_contains" => {
                            let get_result = Box::pin(handle_tool_inner(
                                browser,
                                "get_text",
                                json!({"selector": assert_target}),
                            ))
                            .await;
                            actual_str = match &get_result {
                                Ok(c) => c
                                    .iter()
                                    .filter_map(|c| match c {
                                        ToolContent::Text { text: t } => Some(t.clone()),
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join(""),
                                Err(e) => {
                                    check_passed = false;
                                    format!("Error: {}", e)
                                }
                            };
                            if check_passed {
                                check_passed = actual_str.contains(&expected_str);
                            }
                        }
                        "element_exists" => {
                            let exists_result = Box::pin(handle_tool_inner(
                                browser,
                                "exists",
                                json!({"selector": assert_target}),
                            ))
                            .await;
                            actual_str = match &exists_result {
                                Ok(c) => c
                                    .iter()
                                    .filter_map(|c| match c {
                                        ToolContent::Text { text: t } => Some(t.clone()),
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join(""),
                                Err(e) => {
                                    check_passed = false;
                                    format!("Error: {}", e)
                                }
                            };
                            if check_passed {
                                // exists handler returns JSON with {"exists": true/false}
                                check_passed = actual_str.contains("true");
                            }
                            expected_str = "element exists".to_string();
                        }
                        "value_equals" => {
                            let get_result = Box::pin(handle_tool_inner(
                                browser,
                                "get_text",
                                json!({"selector": assert_target}),
                            ))
                            .await;
                            actual_str = match &get_result {
                                Ok(c) => c
                                    .iter()
                                    .filter_map(|c| match c {
                                        ToolContent::Text { text: t } => Some(t.clone()),
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join(""),
                                Err(e) => {
                                    check_passed = false;
                                    format!("Error: {}", e)
                                }
                            };
                            if check_passed {
                                check_passed = actual_str.trim() == expected_str.trim();
                            }
                        }
                        _ => {
                            // No recognized assert type, just check tool success
                        }
                    }
                }

                // Take evidence screenshot
                let mut screenshot_path = String::new();
                if evidence {
                    let path = format!("{}/step_{:02}.jpg", evidence_dir, step_idx + 1);
                    browser
                        .read()
                        .await
                        .screenshot_to_file(&path, false, 80)
                        .await
                        .ok();
                    screenshot_path = path;
                }

                if check_passed {
                    passed_count += 1;
                } else {
                    failed_count += 1;
                }

                checks.push(json!({
                    "step": step_idx + 1,
                    "tool": tool_name,
                    "assert_type": assert_type,
                    "target": assert_target,
                    "expected": expected_str,
                    "actual": actual_str.chars().take(500).collect::<String>(),
                    "passed": check_passed,
                    "screenshot": screenshot_path
                }));
            }

            let total = passed_count + failed_count;
            let result_str = if failed_count == 0 {
                "pass"
            } else if passed_count == 0 {
                "fail"
            } else {
                "partial"
            };

            // Build failure summary
            let failures: Vec<String> = checks
                .iter()
                .filter(|c| c.get("passed").and_then(|v| v.as_bool()) == Some(false))
                .map(|c| {
                    let step = c.get("step").and_then(|v| v.as_u64()).unwrap_or(0);
                    let target = c.get("target").and_then(|v| v.as_str()).unwrap_or("");
                    let expected = c.get("expected").and_then(|v| v.as_str()).unwrap_or("");
                    let actual = c.get("actual").and_then(|v| v.as_str()).unwrap_or("");
                    format!(
                        "step {}: {} expected '{}' got '{}'",
                        step, target, expected, actual
                    )
                })
                .collect();

            let summary = if failed_count == 0 {
                format!("{}/{} checks passed.", total, total)
            } else {
                format!(
                    "{}/{} checks passed. Failed: {}",
                    passed_count,
                    total,
                    failures.join("; ")
                )
            };

            Ok(vec![text(
                &json!({
                    "result": result_str,
                    "target": target,
                    "intent": intent,
                    "total_checks": total,
                    "passed": passed_count,
                    "failed": failed_count,
                    "checks": checks,
                    "evidence_dir": evidence_dir,
                    "duration_ms": eval_start.elapsed().as_millis() as u64,
                    "summary": summary
                })
                .to_string(),
            )])
        }

        _ => Err(format!("Unknown tool: {}", name)),
    }
}

fn text(s: &str) -> ToolContent {
    ToolContent::Text { text: s.into() }
}

// === FILE NAVIGATION ===
// Generated: 2026-06-13T19:22:13
// Total: 3247 lines | 32 functions | 0 structs | 51 constants
//
// IMPORTS: crate, image, serde_json, std
//
// CONSTANTS:
//   static A11Y_REF_CACHE: 10
//   const remove: 762
//   const doc: 766
//   const main: 769
//   const title: 773
//   const author: 776
//   const date: 779
//   const walk: 785
//   const root: 872
//   const maxNodes: 873
//   const cssPath: 874
//   const parts: 877
//   const parent: 886
//   const siblings: 888
//   const inferRole: 896
//   const explicit: 897
//   const tag: 899
//   const type: 900
//   const nameOf: 910
//   const candidates: 919
//   const rect: 923
//   const style: 924
//   const rect: 928
//   const maxText: 1026
//   const trim: 1027
//   const text: 1028
//   const headings: 1029
//   const links: 1033
//   const buttons: 1037
//   const forms: 1041
//   const rect: 1501
//   const style: 1502
//   const rect: 1507
//   const rect: 1614
//   const style: 1615
//   const label: 1620
//   const result: 1760
//   const el: 1761
//   const rect: 2358
//   const style: 2359
//   const rect: 2364
//   const opts: 2465
//   const match: 2466
//   const sel: 2468
//   const headers: 2493
//   const rows: 2494
//   const walker: 2515
//   const matches: 2516
//   const t: 2518
//   const frames: 2615
//   const rect: 2621
//
// FUNCTIONS:
//   pub +list_tools: 12-607 [LARGE]
//   tool: 609-615
//   http_scrape_text: 617-641
//   has_js_shell_signals: 643-648
//   text_is_short: 650-652
//   looks_like_shell_page: 654-656
//   run_js_extract: 658-696
//   smart_extract: 700-722
//   resize_jpeg_bytes: 724-745
//   resize_b64_jpeg: 747-752
//   format_json_text: 754-756
//   build_extract_script: 758-811 [med]
//   extract_page_content: 813-818
//   a11y_cache: 820-822
//   js_string: 824-826
//   parse_eval_json: 828-834
//   truncate_chars: 836-838
//   now_millis: 840-845
//   default_artifact_dir: 847-849
//   write_json_file: 851-855
//   lookup_a11y_selector: 857-862
//   build_a11y_snapshot_script: 864-945 [med]
//   refresh_a11y_snapshot_from_manager: 947-976
//   scroll_until_stable: 978-1021
//   build_page_dump_script: 1023-1075 [med]
//   page_dump_from_manager: 1077-1086
//   handle_page_capture: 1088-1212 [LARGE]
//   handle_verify_state: 1214-1297 [med]
//   handle_smart_navigate: 1299-1367 [med]
//   pub +handle_tool: 1369-1378
//   handle_tool_inner: 1380-3243 [LARGE]
//   text: 3245-3247
//
// === END FILE NAVIGATION ===
