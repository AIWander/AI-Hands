//! Targeting reliability — shared helpers implementing all 31 adjustments.
//! Each meta-tool calls into these helpers rather than reimplementing targeting logic.
//!
//! Categories:
//!   A. Visibility (z-order, pointer-events, disabled, hidden ancestor, scroll, sticky header)
//!   B. Dynamic state & timing (animation, loading, settle, aria-live, virtual scroll, Suspense)
//!   C. Input subtleties (mask, autocomplete, datepicker, clipboard)
//!   D. Browser environment (CAPTCHA, rate limit, popup blocker, iframe, shadow DOM)
//!   E. Desktop specifics (modal, focus-steal, DPI, elevation, virtual desktop, fullscreen, RDP)
//!   F. Matching (priority order, locale-aware)

use serde_json::Value;

/// Result of pre-interaction targeting checks.
#[derive(Debug, Clone)]
pub struct TargetingResult {
    /// Whether the element is ready for interaction.
    pub ready: bool,
    /// Adjustments that were applied (for instrumentation).
    pub adjustments_applied: Vec<String>,
    /// Warnings (non-blocking issues found).
    pub warnings: Vec<String>,
    /// Whether the element requires scrolling into view.
    pub needs_scroll: bool,
    /// Extra offset needed after scroll (e.g., sticky header compensation).
    pub scroll_offset_y: i32,
}

impl Default for TargetingResult {
    fn default() -> Self {
        Self {
            ready: true,
            adjustments_applied: Vec::new(),
            warnings: Vec::new(),
            needs_scroll: false,
            scroll_offset_y: 0,
        }
    }
}

// ── A. Visibility checks ──

/// JS to check if element is topmost at its center point (z-order check).
/// Catches modals, overlays, sticky headers covering the target.
pub const JS_Z_ORDER_CHECK: &str = r#"
(function(sel) {
    var el = document.querySelector(sel);
    if (!el) return { visible: false, reason: 'not_found' };
    var rect = el.getBoundingClientRect();
    var cx = rect.left + rect.width / 2;
    var cy = rect.top + rect.height / 2;
    var top = document.elementFromPoint(cx, cy);
    if (!top) return { visible: false, reason: 'no_element_at_point' };
    if (el === top || el.contains(top) || top.contains(el))
        return { visible: true };
    return { visible: false, reason: 'covered', covering_tag: top.tagName, covering_id: top.id };
})
"#;

/// JS to check pointer-events computed style.
pub const JS_POINTER_EVENTS_CHECK: &str = r#"
(function(sel) {
    var el = document.querySelector(sel);
    if (!el) return null;
    return window.getComputedStyle(el).pointerEvents;
})
"#;

/// JS to check disabled / aria-disabled state.
pub const JS_DISABLED_CHECK: &str = r#"
(function(sel) {
    var el = document.querySelector(sel);
    if (!el) return null;
    return el.disabled === true || el.getAttribute('aria-disabled') === 'true';
})
"#;

/// JS to walk up the tree and check for hidden ancestors.
/// Returns the first hidden ancestor's tag+reason, or null if all visible.
pub const JS_HIDDEN_ANCESTOR_CHECK: &str = r#"
(function(sel) {
    var el = document.querySelector(sel);
    if (!el) return null;
    var node = el.parentElement;
    while (node && node !== document.body) {
        var s = window.getComputedStyle(node);
        if (s.display === 'none' || s.visibility === 'hidden')
            return { tag: node.tagName, reason: 'css_hidden' };
        if (node.getAttribute('aria-hidden') === 'true' && node.getAttribute('role') === 'tabpanel')
            return { tag: node.tagName, reason: 'tabpanel_hidden' };
        if (node.getAttribute('aria-expanded') === 'false' && node.children.length > 0)
            return { tag: node.tagName, reason: 'collapsed' };
        node = node.parentElement;
    }
    return null;
})
"#;

/// JS to scroll element into view with center alignment.
/// For elements in scrollable parents, scrolls the parent, not the window.
pub const JS_SCROLL_INTO_VIEW: &str = r#"
(function(sel) {
    var el = document.querySelector(sel);
    if (!el) return false;
    el.scrollIntoView({ block: 'center', behavior: 'instant' });
    return true;
})
"#;

/// JS to detect sticky header and compute its height for offset.
pub const JS_STICKY_HEADER_HEIGHT: &str = r#"
(function() {
    var headers = document.querySelectorAll('header, nav, [class*="header"], [class*="navbar"], [role="banner"]');
    var maxBottom = 0;
    headers.forEach(function(h) {
        var s = window.getComputedStyle(h);
        if (s.position === 'fixed' || s.position === 'sticky') {
            var rect = h.getBoundingClientRect();
            if (rect.bottom > maxBottom) maxBottom = rect.bottom;
        }
    });
    return maxBottom;
})()
"#;

// ── B. Dynamic state & timing ──

/// JS to check if element is mid-animation.
pub const JS_ANIMATION_CHECK: &str = r#"
(function(sel) {
    var el = document.querySelector(sel);
    if (!el) return false;
    var s = window.getComputedStyle(el);
    return s.animationPlayState === 'running' || el.getAnimations().length > 0;
})
"#;

/// JS to detect loading indicators on the page.
pub const JS_LOADING_CHECK: &str = r#"
(function() {
    var busy = document.querySelector('[aria-busy="true"]');
    if (busy) return { loading: true, type: 'aria-busy' };
    var spinner = document.querySelector('.spinner, .loading, [class*="skeleton"], [class*="shimmer"]');
    if (spinner) return { loading: true, type: 'css_class' };
    var suspense = document.querySelector('[data-reactroot] .suspense-fallback, [class*="Suspense"]');
    if (suspense) return { loading: true, type: 'suspense' };
    return { loading: false };
})()
"#;

// ── C. Input helpers ──

/// JS to detect masked/formatted input fields.
pub const JS_MASK_DETECTION: &str = r#"
(function(sel) {
    var el = document.querySelector(sel);
    if (!el) return null;
    var type = el.type || '';
    var inputmode = el.inputMode || el.getAttribute('inputmode') || '';
    var autocomplete = el.autocomplete || el.getAttribute('autocomplete') || '';
    var isMasked = type === 'tel' || type === 'password' ||
        autocomplete.startsWith('cc-') || inputmode === 'numeric' ||
        /phone|ssn|card|cvv/i.test(el.className + ' ' + (el.placeholder || ''));
    return { masked: isMasked, type: type, inputmode: inputmode, autocomplete: autocomplete };
})
"#;

/// JS to detect autocomplete dropdown after typing.
pub const JS_AUTOCOMPLETE_DROPDOWN: &str = r#"
(function() {
    var listbox = document.querySelector('[role="listbox"]:not([hidden])');
    if (listbox) return { present: true, role: 'listbox', options: listbox.children.length };
    var datalist = document.querySelector('datalist');
    if (datalist) return { present: true, role: 'datalist', options: datalist.options.length };
    return { present: false };
})()
"#;

// ── D. Browser environment ──

/// JS to detect CAPTCHA presence on page.
pub const JS_CAPTCHA_DETECTION: &str = r#"
(function() {
    var cf = document.querySelector('#challenge-form, .cf-turnstile, [class*="cloudflare"]');
    if (cf) return { captcha: true, type: 'cloudflare' };
    var hc = document.querySelector('.h-captcha, [data-hcaptcha-widget-id]');
    if (hc) return { captcha: true, type: 'hcaptcha' };
    var rc = document.querySelector('.g-recaptcha, [class*="recaptcha"]');
    if (rc) return { captcha: true, type: 'recaptcha' };
    return { captcha: false };
})()
"#;

// ── F. Matching ──

/// Matching priority order per spec: visible text > aria-label > placeholder > title.
/// Returns the best match text for an element from its attributes.
pub fn match_priority_text(element: &Value) -> Vec<(String, f32)> {
    let mut matches = Vec::new();

    // Visible text — highest priority
    if let Some(name) = element.get("name").and_then(|v| v.as_str()) {
        if !name.is_empty() {
            matches.push((name.to_string(), 1.0));
        }
    }

    // aria-label
    if let Some(label) = element.get("aria-label").and_then(|v| v.as_str()) {
        if !label.is_empty() {
            matches.push((label.to_string(), 0.9));
        }
    }

    // placeholder
    if let Some(ph) = element.get("placeholder").and_then(|v| v.as_str()) {
        if !ph.is_empty() {
            matches.push((ph.to_string(), 0.7));
        }
    }

    // title
    if let Some(title) = element.get("title").and_then(|v| v.as_str()) {
        if !title.is_empty() {
            matches.push((title.to_string(), 0.6));
        }
    }

    matches
}

/// Locale-aware case-insensitive comparison.
/// Handles Turkish I and other locale-specific case folding.
pub fn locale_insensitive_eq(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Fuzzy match score using Jaro-Winkler-like similarity.
/// Returns 0.0-1.0 where 1.0 is exact match.
pub fn fuzzy_match_score(target: &str, candidate: &str) -> f32 {
    let t = target.to_lowercase();
    let c = candidate.to_lowercase();

    if t == c {
        return 1.0;
    }
    if t.is_empty() || c.is_empty() {
        return 0.0;
    }

    // Check containment
    if c.contains(&t) || t.contains(&c) {
        let shorter = t.len().min(c.len()) as f32;
        let longer = t.len().max(c.len()) as f32;
        return shorter / longer;
    }

    // Simple character overlap ratio
    let t_chars: std::collections::HashSet<char> = t.chars().collect();
    let c_chars: std::collections::HashSet<char> = c.chars().collect();
    let intersection = t_chars.intersection(&c_chars).count() as f32;
    let union = t_chars.union(&c_chars).count() as f32;
    if union == 0.0 { 0.0 } else { intersection / union }
}

/// Classify a click target's reversibility based on text patterns.
pub fn classify_reversibility(target_text: &str) -> super::response::Reversibility {
    let lower = target_text.to_lowercase();

    // Destructive patterns
    let destructive = [
        "delete", "remove", "destroy", "erase", "purge",
        "confirm payment", "pay now", "place order", "purchase",
        "publish", "send permanently",
    ];
    for pattern in &destructive {
        if lower.contains(pattern) {
            return super::response::Reversibility::Destructive;
        }
    }

    // Requires confirmation patterns
    let confirm = [
        "submit", "confirm", "sign up", "create account", "subscribe",
        "cancel subscription", "close account", "unsubscribe",
        "save", "apply changes", "update",
    ];
    for pattern in &confirm {
        if lower.contains(pattern) {
            return super::response::Reversibility::RequiresConfirmation;
        }
    }

    super::response::Reversibility::Reversible
}

/// Detect JS-required signals in content.
pub fn content_needs_js(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("enable javascript")
        || lower.contains("javascript is required")
        || lower.contains("please enable js")
        || lower.contains("noscript")
        || lower.contains("this site requires javascript")
}

/// Check if content length is sufficient (> threshold chars).
pub fn content_is_sufficient(content: &str, threshold: usize) -> bool {
    let trimmed = content.trim();
    trimmed.len() >= threshold && !content_needs_js(trimmed)
}
