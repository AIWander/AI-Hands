//! Conservative redaction for browser network and dashboard output.
//!
//! Network capture is an observation surface, not a credential transport. These
//! helpers preserve endpoint and payload shape while removing captured values.

use regex::{Captures, Regex};
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

const REDACTED: &str = "[REDACTED]";
const REDACTED_BODY: &str = "[REDACTED_BODY]";

fn compact_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_sensitive_key(key: &str) -> bool {
    let key = compact_key(key);
    matches!(
        key.as_str(),
        "authorization"
            | "proxyauthorization"
            | "cookie"
            | "setcookie"
            | "apikey"
            | "xapikey"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "secret"
            | "clientsecret"
            | "password"
            | "passwd"
            | "pwd"
            | "credential"
            | "credentials"
            | "credentialname"
            | "username"
            | "userid"
            | "email"
            | "accountid"
            | "otp"
            | "totp"
            | "totpname"
            | "onetimecode"
            | "pin"
            | "passcode"
            | "session"
            | "sessionid"
            | "csrf"
            | "csrftoken"
            | "xsrf"
            | "xsrftoken"
            | "signature"
            | "privatekey"
            | "accesskey"
            | "typedtext"
            | "typedvalue"
            | "inputvalue"
            | "currentvalue"
            | "previousvalue"
            | "passwordvalue"
    ) || key.ends_with("token")
        || key.ends_with("secret")
        || key.ends_with("password")
        || key.ends_with("apikey")
        || key.ends_with("cookie")
}

fn is_body_like_key(key: &str) -> bool {
    let key = compact_key(key);
    key.ends_with("body")
        || key.contains("bodytemplate")
        || key.contains("postdata")
        || key.contains("formdata")
        || key == "payload"
        || key == "params"
        || key == "resolvedparams"
        || key == "query"
        || key == "queryparams"
        || key == "searchparams"
}

fn is_url_key(key: &str) -> bool {
    let key = compact_key(key);
    key == "url" || key.ends_with("url") || key == "uri" || key == "href"
}

fn is_cookie_container_key(key: &str) -> bool {
    matches!(compact_key(key).as_str(), "cookies" | "cookiejar")
}

fn is_header_container_key(key: &str) -> bool {
    let key = compact_key(key);
    key == "header" || key == "headers" || key.ends_with("headers")
}

fn is_safe_header_metadata(key: &str) -> bool {
    matches!(
        compact_key(key).as_str(),
        "accept" | "contenttype" | "contentlength"
    )
}

fn redact_authorization(value: &Value) -> Value {
    let Some(raw) = value.as_str() else {
        return Value::String(REDACTED.into());
    };
    let trimmed = raw.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    for scheme in ["bearer", "basic", "digest", "negotiate"] {
        if lower.starts_with(&format!("{scheme} ")) {
            let original_scheme = trimmed.split_whitespace().next().unwrap_or(scheme);
            return Value::String(format!("{original_scheme} {REDACTED}"));
        }
    }
    Value::String(REDACTED.into())
}

fn shape_only(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let shaped = if is_sensitive_key(key) {
                        Value::String(REDACTED.into())
                    } else {
                        shape_only(value)
                    };
                    (key.clone(), shaped)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(shape_only).collect()),
        Value::String(_) => Value::String("<string>".into()),
        Value::Number(_) => Value::String("<number>".into()),
        Value::Bool(_) => Value::String("<boolean>".into()),
        Value::Null => Value::String("<null>".into()),
    }
}

fn redact_body(value: &Value) -> Value {
    match value {
        Value::String(raw) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                return Value::String(shape_only(&parsed).to_string());
            }
            if raw.contains('=') {
                let shape = raw
                    .split('&')
                    .map(|pair| {
                        let key = pair.split_once('=').map(|(key, _)| key).unwrap_or(pair);
                        format!("{key}={REDACTED}")
                    })
                    .collect::<Vec<_>>()
                    .join("&");
                return Value::String(shape);
            }
            Value::String(REDACTED_BODY.into())
        }
        other => shape_only(other),
    }
}

fn redact_high_entropy_path_segments(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.is_empty() {
                segment
            } else {
                // A path segment can be a short reset code, OTP, user id, or
                // opaque resource key. Preserve path shape, never the value.
                "{segment}"
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn safe_query_key(raw: &str) -> String {
    let normalized = raw.trim().to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "page"
            | "limit"
            | "offset"
            | "sort"
            | "order"
            | "filter"
            | "q"
            | "query"
            | "search"
            | "cursor"
            | "after"
            | "before"
            | "fields"
            | "include"
            | "expand"
            | "lang"
            | "locale"
            | "format"
            | "version"
            | "id"
            | "code"
            | "state"
            | "token"
            | "key"
    ) {
        normalized
    } else {
        "parameter".to_string()
    }
}

/// Preserve URL structure while removing user-info, every query value, the
/// fragment, and every path-segment value. The segment count remains visible,
/// but endpoint names are intentionally not persisted because short segments
/// can also be reset codes, user ids, or one-time tokens.
pub(crate) fn redact_url(raw: &str) -> String {
    let lower = raw.trim_start().to_ascii_lowercase();
    if lower.starts_with("file://") {
        return "file://[PRIVATE_PATH]".to_string();
    }
    for scheme in ["data:", "mailto:"] {
        if lower.starts_with(scheme) {
            return format!("{scheme}{REDACTED}");
        }
    }
    let (without_fragment, had_fragment) = raw
        .split_once('#')
        .map(|(base, _)| (base, true))
        .unwrap_or((raw, false));
    let (base, query) = without_fragment
        .split_once('?')
        .map(|(base, query)| (base, Some(query)))
        .unwrap_or((without_fragment, None));

    let mut safe_base = base.to_string();
    if let Some(scheme_end) = safe_base.find("://") {
        let authority_start = scheme_end + 3;
        let authority_end = safe_base[authority_start..]
            .find('/')
            .map(|offset| authority_start + offset)
            .unwrap_or(safe_base.len());
        if let Some(at_offset) = safe_base[authority_start..authority_end].rfind('@') {
            let at = authority_start + at_offset;
            safe_base.replace_range(authority_start..at, REDACTED);
        }
    }

    let path_start = if let Some(scheme_end) = safe_base.find("://") {
        safe_base[scheme_end + 3..]
            .find('/')
            .map(|p| scheme_end + 3 + p)
            .unwrap_or(safe_base.len())
    } else {
        0
    };
    if path_start < safe_base.len() {
        let safe_path = redact_high_entropy_path_segments(&safe_base[path_start..]);
        safe_base.replace_range(path_start.., &safe_path);
    }

    if let Some(query) = query {
        let safe_query = query
            .split('&')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let raw_key = part
                    .split_once('=')
                    .map(|(key, _)| key)
                    .unwrap_or("parameter");
                let key = safe_query_key(raw_key);
                format!("{key}={REDACTED}")
            })
            .collect::<Vec<_>>()
            .join("&");
        safe_base.push('?');
        safe_base.push_str(&safe_query);
    }
    if had_fragment {
        safe_base.push('#');
        safe_base.push_str(REDACTED);
    }
    safe_base
}

fn redact_string_literal(raw: &str) -> Value {
    static URL_RE: OnceLock<Regex> = OnceLock::new();
    static AUTH_RE: OnceLock<Regex> = OnceLock::new();
    static SECRET_ASSIGNMENT_RE: OnceLock<Regex> = OnceLock::new();
    static JWT_RE: OnceLock<Regex> = OnceLock::new();
    static OTP_RE: OnceLock<Regex> = OnceLock::new();
    static PRIVATE_PATH_RE: OnceLock<Regex> = OnceLock::new();
    static UNC_PATH_RE: OnceLock<Regex> = OnceLock::new();
    let trimmed = raw.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
            return Value::String(redact_network_value(&parsed).to_string());
        }
    }
    if (trimmed.starts_with('/') || trimmed.starts_with("./") || trimmed.starts_with("../"))
        && trimmed.contains('?')
        && !trimmed.chars().any(char::is_whitespace)
    {
        return Value::String(redact_url(raw));
    }

    let url_re = URL_RE.get_or_init(|| {
        Regex::new(
            r#"(?i)(?:(?:https?|wss?|ftp|file|otpauth|ssh|git)://[^\s\"'<>]+|(?:data|mailto):[^\s\"'<>]+)"#,
        )
        .unwrap()
    });
    let auth_re = AUTH_RE.get_or_init(|| {
        Regex::new(r#"(?i)\b(bearer|basic|digest|negotiate)\s+[^\s,;\"']+"#).unwrap()
    });
    let secret_assignment_re = SECRET_ASSIGNMENT_RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\b(password|passwd|pwd|token|access[_-]?token|refresh[_-]?token|secret|api[_-]?key|cookie|authorization|otp|one[-_ ]?time[-_ ]?code|username|user[_-]?id|email|credential[_-]?name|totp[_-]?name)\b(\s*[:=]\s*)([^\s,;\"']+)"#,
        )
        .unwrap()
    });
    let jwt_re = JWT_RE.get_or_init(|| {
        Regex::new(r#"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b"#).unwrap()
    });
    let otp_re = OTP_RE.get_or_init(|| Regex::new(r#"\b\d{6}\b"#).unwrap());
    let private_path_re = PRIVATE_PATH_RE.get_or_init(|| {
        Regex::new(r#"(?i)\b[A-Z]:\\Users\\[^\\\s\"']+(?:\\[^\\\s\"']+)*"#).unwrap()
    });
    let unc_path_re = UNC_PATH_RE
        .get_or_init(|| Regex::new(r#"\\\\[^\\\s\"']+\\[^\\\s\"']+(?:\\[^\\\s\"']+)*"#).unwrap());

    let urls_redacted = url_re.replace_all(raw, |captures: &Captures<'_>| {
        redact_url(captures.get(0).unwrap().as_str())
    });
    let auth_redacted = auth_re.replace_all(&urls_redacted, |captures: &Captures<'_>| {
        format!("{} {REDACTED}", &captures[1])
    });
    let assignments_redacted = secret_assignment_re
        .replace_all(&auth_redacted, |captures: &Captures<'_>| {
            format!("{}{}{REDACTED}", &captures[1], &captures[2])
        });
    let jwts_redacted = jwt_re.replace_all(&assignments_redacted, REDACTED);
    let otps_redacted = otp_re.replace_all(&jwts_redacted, REDACTED);
    let paths_redacted = private_path_re.replace_all(&otps_redacted, "[PRIVATE_PATH]");
    let unc_redacted = unc_path_re.replace_all(&paths_redacted, "[PRIVATE_PATH]");
    Value::String(unc_redacted.into_owned())
}

fn redact_object(map: &Map<String, Value>) -> Value {
    let pair_is_sensitive = map
        .get("name")
        .or_else(|| map.get("key"))
        .and_then(Value::as_str)
        .is_some_and(is_sensitive_key);
    // Name/value is the common minimal cookie representation. Conservatively
    // hide its value even when optional cookie metadata was omitted.
    let looks_like_cookie = map.contains_key("name") && map.contains_key("value");
    let control_kinds = ["type", "tag", "tag_name", "control_type", "role"]
        .iter()
        .filter_map(|key| map.get(*key).and_then(Value::as_str))
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let control_identity = ["name", "id", "label", "autocomplete"]
        .iter()
        .filter_map(|key| map.get(*key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let looks_like_editable_control = control_kinds.iter().any(|kind| {
        matches!(
            kind.as_str(),
            "input"
                | "textarea"
                | "edit"
                | "textbox"
                | "password"
                | "hidden"
                | "text"
                | "email"
                | "tel"
                | "search"
                | "url"
                | "number"
                | "date"
                | "datetime-local"
                | "select"
                | "select-one"
                | "select-multiple"
                | "option"
                | "combobox"
                | "listbox"
                | "checkbox"
                | "radio"
        )
    });
    let looks_like_sensitive_control = control_kinds
        .iter()
        .any(|kind| matches!(kind.as_str(), "password" | "hidden"))
        || [
            "password",
            "passwd",
            "secret",
            "token",
            "otp",
            "one-time-code",
            "current-password",
            "new-password",
            "cc-number",
            "cc-csc",
        ]
        .iter()
        .any(|needle| control_identity.contains(needle));

    Value::Object(
        map.iter()
            .map(|(key, value)| {
                let compact = compact_key(key);
                let safe = if ((pair_is_sensitive
                    || looks_like_cookie
                    || looks_like_editable_control
                    || looks_like_sensitive_control)
                    && compact == "value")
                    || (looks_like_sensitive_control && compact == "text")
                {
                    Value::String(REDACTED.into())
                } else if is_cookie_container_key(key) {
                    redact_cookie_container(value)
                } else if is_header_container_key(key) {
                    redact_header_container(value)
                } else if is_sensitive_key(key) {
                    if compact.contains("authorization") {
                        redact_authorization(value)
                    } else {
                        Value::String(REDACTED.into())
                    }
                } else if is_body_like_key(key) {
                    redact_body(value)
                } else if is_url_key(key) {
                    value
                        .as_str()
                        .map(redact_url)
                        .map(Value::String)
                        .unwrap_or_else(|| redact_network_value(value))
                } else {
                    redact_network_value(value)
                };
                (key.clone(), safe)
            })
            .collect(),
    )
}

fn redact_header_container(value: &Value) -> Value {
    match value {
        Value::Object(headers) => Value::Object(
            headers
                .iter()
                .map(|(key, value)| {
                    let safe = if is_safe_header_metadata(key) {
                        redact_network_value(value)
                    } else if compact_key(key).contains("authorization") {
                        redact_authorization(value)
                    } else {
                        Value::String(REDACTED.into())
                    };
                    (key.clone(), safe)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| match item {
                    Value::Array(pair) if pair.len() == 2 => {
                        let key = pair[0].as_str().unwrap_or("");
                        let safe_value = if is_safe_header_metadata(key) {
                            redact_network_value(&pair[1])
                        } else if compact_key(key).contains("authorization") {
                            redact_authorization(&pair[1])
                        } else {
                            Value::String(REDACTED.into())
                        };
                        Value::Array(vec![pair[0].clone(), safe_value])
                    }
                    _ => Value::String(REDACTED.into()),
                })
                .collect(),
        ),
        _ => Value::String(REDACTED.into()),
    }
}

fn redact_cookie_container(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| match item {
                    Value::Object(cookie) => Value::Object(
                        cookie
                            .iter()
                            .map(|(key, value)| {
                                let safe = if compact_key(key) == "value" {
                                    Value::String(REDACTED.into())
                                } else {
                                    redact_network_value(value)
                                };
                                (key.clone(), safe)
                            })
                            .collect(),
                    ),
                    _ => Value::String(REDACTED.into()),
                })
                .collect(),
        ),
        _ => Value::String(REDACTED.into()),
    }
}

/// Return a redacted copy suitable for model output, logs, or dashboards.
pub(crate) fn redact_network_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => redact_object(map),
        Value::Array(items) => {
            if items.len() == 2 && items[0].as_str().is_some_and(is_sensitive_key) {
                return Value::Array(vec![items[0].clone(), Value::String(REDACTED.into())]);
            }
            Value::Array(items.iter().map(redact_network_value).collect())
        }
        Value::String(raw) => redact_string_literal(raw),
        other => other.clone(),
    }
}

pub(crate) fn redact_network_value_in_place(value: &mut Value) {
    *value = redact_network_value(value);
}

fn safe_identifier(raw: &str) -> Option<String> {
    let mut chars = raw.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_')
        || raw.len() > 64
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return None;
    }
    Some(raw.to_string())
}

fn project_trace_entry(value: &Value) -> Value {
    let Some(map) = value.as_object() else {
        return json!({"type": "event"});
    };
    let raw_type = map
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let event_type = match raw_type.as_str() {
        "trace_start"
        | "navigation"
        | "resource"
        | "longtask"
        | "paint"
        | "largest-contentful-paint"
        | "click"
        | "script_step"
        | "event" => raw_type.as_str(),
        _ => "event",
    };

    let mut safe = Map::new();
    safe.insert("type".into(), Value::String(event_type.to_string()));
    for key in ["time", "start", "duration", "x", "y"] {
        if let Some(number) = map.get(key).filter(|item| item.is_number()) {
            safe.insert(key.into(), number.clone());
        }
    }

    match event_type {
        "trace_start" => {
            safe.insert(
                "url".into(),
                Value::String("[TRACE_URL_WITHHELD]".to_string()),
            );
        }
        "click" => {
            if let Some(tag) = map
                .get("tag")
                .and_then(Value::as_str)
                .map(str::to_ascii_uppercase)
                .filter(|tag| {
                    !tag.is_empty()
                        && tag.len() <= 20
                        && tag
                            .chars()
                            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '-')
                })
            {
                safe.insert("tag".into(), Value::String(tag));
            }
        }
        "navigation" | "resource" => {
            for key in ["name", "from"] {
                if let Some(url) = map.get(key).and_then(Value::as_str) {
                    safe.insert(key.into(), Value::String(redact_url(url)));
                }
            }
        }
        "paint" | "largest-contentful-paint" => {
            if let Some(name) = map.get("name").and_then(Value::as_str).filter(|name| {
                matches!(
                    *name,
                    "self" | "first-paint" | "first-contentful-paint" | "largest-contentful-paint"
                )
            }) {
                safe.insert("name".into(), Value::String(name.to_string()));
            }
        }
        "script_step" => {
            if let Some(name) = map
                .get("name")
                .and_then(Value::as_str)
                .and_then(safe_identifier)
            {
                safe.insert("name".into(), Value::String(name));
            }
            if let Some(details) = map.get("details").and_then(Value::as_object) {
                let mut safe_details = Map::new();
                if let Some(step) = details.get("step").filter(|item| item.is_number()) {
                    safe_details.insert("step".into(), step.clone());
                }
                if let Some(keys) = details.get("parameter_keys").and_then(Value::as_array) {
                    let keys = keys
                        .iter()
                        .filter_map(Value::as_str)
                        .filter_map(safe_identifier)
                        .map(Value::String)
                        .collect::<Vec<_>>();
                    safe_details.insert("parameter_keys".into(), Value::Array(keys));
                }
                if !safe_details.is_empty() {
                    safe.insert("details".into(), Value::Object(safe_details));
                }
            }
        }
        _ => {}
    }

    Value::Object(safe)
}

/// Project page-owned trace data onto a closed server-owned schema before any
/// value is returned or written. A hostile page can replace the JavaScript
/// trace array, so deny-list redaction alone is not a sufficient boundary.
pub(crate) fn project_trace_value(value: &Value) -> Value {
    let entries = value
        .get("entries")
        .and_then(Value::as_array)
        .or_else(|| value.as_array())
        .map(|items| items.iter().map(project_trace_entry).collect::<Vec<_>>())
        .unwrap_or_default();
    json!({
        "count": entries.len(),
        "entries": entries,
    })
}

fn safe_performance_kind(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    (!normalized.is_empty()
        && normalized.len() <= 32
        && normalized
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-'))
    .then_some(normalized)
}

/// Performance API results are also page-controlled. Keep only fields whose
/// types and meaning are fixed by the server contract.
pub(crate) fn project_performance_log_value(value: &Value) -> Value {
    let items = value
        .get("entries")
        .and_then(Value::as_array)
        .or_else(|| value.as_array());
    let entries = items
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let Some(map) = item.as_object() else {
                        return json!({"entry_type": "event"});
                    };
                    let mut safe = Map::new();
                    for key in ["url", "name"] {
                        if let Some(name) = map.get(key).and_then(Value::as_str) {
                            safe.insert(key.into(), Value::String(redact_url(name)));
                        }
                    }
                    for key in [
                        "type",
                        "entry_type",
                        "entryType",
                        "initiator_type",
                        "initiatorType",
                    ] {
                        if let Some(kind) = map
                            .get(key)
                            .and_then(Value::as_str)
                            .and_then(safe_performance_kind)
                        {
                            let output_key = match key {
                                "entryType" => "entry_type",
                                "initiatorType" => "initiator_type",
                                other => other,
                            };
                            safe.insert(output_key.into(), Value::String(kind));
                        }
                    }
                    for key in [
                        "start_time",
                        "startTime",
                        "duration",
                        "response_end",
                        "responseEnd",
                        "transfer_size",
                        "transferSize",
                        "encoded_body_size",
                        "encodedBodySize",
                        "decoded_body_size",
                        "decodedBodySize",
                        "duration_ms",
                        "size_bytes",
                        "start_ms",
                    ] {
                        if let Some(number) = map.get(key).filter(|item| item.is_number()) {
                            let output_key = match key {
                                "startTime" => "start_time",
                                "responseEnd" => "response_end",
                                "transferSize" => "transfer_size",
                                "encodedBodySize" => "encoded_body_size",
                                "decodedBodySize" => "decoded_body_size",
                                other => other,
                            };
                            safe.insert(output_key.into(), number.clone());
                        }
                    }
                    Value::Object(safe)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({"entries": entries, "count": entries.len()})
}

/// Trace data must never retain typed/clicked text, script source, evaluated
/// expressions, or resolved parameter values even when their field names do
/// not look credential-specific.
pub(crate) fn redact_trace_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let compact = compact_key(key);
                    let safe = if matches!(
                        compact.as_str(),
                        "text" | "value" | "expression" | "script" | "code" | "params"
                    ) {
                        Value::String(REDACTED.into())
                    } else {
                        redact_trace_value(value)
                    };
                    (key.clone(), safe)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_trace_value).collect()),
        other => redact_network_value(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_headers_cookies_and_pair_arrays() {
        let raw = json!({
            "request_headers": {
                "Authorization": "Bearer top-secret-token",
                "Cookie": "session=private",
                "X-Auth-Widget": "CUSTOM_HEADER_PRIVATE",
                "Accept": "application/json"
            },
            "headers": [["X-API-Key", "private-key"], ["Accept", "text/plain"]]
        });
        let safe = redact_network_value(&raw);
        let text = safe.to_string();
        assert!(!text.contains("top-secret-token"));
        assert!(!text.contains("session=private"));
        assert!(!text.contains("private-key"));
        assert!(!text.contains("CUSTOM_HEADER_PRIVATE"));
        assert!(text.contains("application/json"));
        assert!(text.contains("Bearer [REDACTED]"));
    }

    #[test]
    fn redacts_url_userinfo_query_fragment_and_long_path_token() {
        let raw = "https://user:pass@example.com/reset/abcdefghijklmnopqrstuvwxyz123456?code=secret&state=private#token";
        let safe = redact_url(raw);
        assert_eq!(
            safe,
            "https://[REDACTED]@example.com/{segment}/{segment}?code=[REDACTED]&state=[REDACTED]#[REDACTED]"
        );
    }

    #[test]
    fn body_shape_preserves_keys_and_types_not_values() {
        let raw = json!({
            "request_body": {
                "email": "person@example.com",
                "password": "private-password",
                "display_name": "Joseph",
                "count": 42,
                "enabled": true
            }
        });
        let safe = redact_network_value(&raw);
        let text = safe.to_string();
        assert!(!text.contains("person@example.com"));
        assert!(!text.contains("private-password"));
        assert!(!text.contains("Joseph"));
        assert!(!text.contains("42"));
        assert!(text.contains("email"));
        assert!(text.contains("<string>"));
        assert!(text.contains("<number>"));
        assert!(text.contains("<boolean>"));
    }

    #[test]
    fn stringified_json_and_form_bodies_keep_only_shape() {
        let raw = json!({
            "response_body": "{\"access_token\":\"private\",\"name\":\"Joseph\"}",
            "post_data": "username=person&password=private"
        });
        let safe = redact_network_value(&raw).to_string();
        assert!(!safe.contains("private"));
        assert!(!safe.contains("Joseph"));
        assert!(!safe.contains("person"));
        assert!(safe.contains("access_token"));
        assert!(safe.contains("username=[REDACTED]"));
    }

    #[test]
    fn leaves_non_sensitive_metadata_unchanged() {
        let raw = json!({
            "method": "POST",
            "status": 200,
            "content_type": "application/json",
            "duration_ms": 12
        });
        assert_eq!(redact_network_value(&raw), raw);
    }

    #[test]
    fn preserves_generic_two_string_arrays_outside_sensitive_containers() {
        let raw = json!({"methods": ["GET", "POST"]});
        assert_eq!(redact_network_value(&raw), raw);
    }

    #[test]
    fn redacts_urls_and_authorization_embedded_in_error_text() {
        let raw = Value::String(
            "failed https://example.com/cb?code=private#frag with Bearer private-token".into(),
        );
        let safe = redact_network_value(&raw).as_str().unwrap().to_string();
        assert!(!safe.contains("code=private"));
        assert!(!safe.contains("private-token"));
        assert!(safe.contains("code=[REDACTED]"));
    }

    #[test]
    fn redacts_relative_and_otpauth_urls() {
        let relative = redact_network_value(&Value::String(
            "/callback?code=private&state=also-private#fragment".into(),
        ));
        assert_eq!(
            relative.as_str().unwrap(),
            "/{segment}?code=[REDACTED]&state=[REDACTED]#[REDACTED]"
        );

        let otp = redact_network_value(&Value::String(
            "otpauth://totp/Example:user?secret=VERYPRIVATE&issuer=Example".into(),
        ));
        let text = otp.as_str().unwrap();
        assert!(!text.contains("VERYPRIVATE"));
        assert!(text.contains("parameter=[REDACTED]"));
    }

    #[test]
    fn redacts_websocket_ftp_and_file_urls_in_free_text() {
        let raw = Value::String(
            "ws://example.test/socket?token=WS_PRIVATE ftp://user:pass@example.test/file?key=FTP_PRIVATE file:///C:/Users/LOCAL_USER_SENTINEL/private.txt".into(),
        );
        let safe = redact_network_value(&raw).as_str().unwrap().to_string();
        for sentinel in [
            "WS_PRIVATE",
            "FTP_PRIVATE",
            "user:pass",
            "LOCAL_USER_SENTINEL",
        ] {
            assert!(!safe.contains(sentinel), "leaked sentinel: {sentinel}");
        }
        assert!(safe.contains("ws://example.test/{segment}?token=[REDACTED]"));
        assert!(safe.contains("file://[PRIVATE_PATH]"));
    }

    #[test]
    fn redacts_no_path_origin_and_non_hierarchical_sensitive_uris() {
        let origin = redact_url("https://example.test?token=NO_PATH_SECRET");
        assert_eq!(origin, "https://example.test?token=[REDACTED]");

        let raw = Value::String(
            "data:text/plain;base64,DATA_SECRET mailto:person@example.test ssh://user@example.test/private"
                .into(),
        );
        let safe = redact_network_value(&raw).as_str().unwrap().to_string();
        for sentinel in ["DATA_SECRET", "person@example.test", "user@", "private"] {
            assert!(!safe.contains(sentinel), "leaked sentinel: {sentinel}");
        }
        assert!(safe.contains("data:[REDACTED]"));
        assert!(safe.contains("mailto:[REDACTED]"));
        assert!(safe.contains("ssh://[REDACTED]@example.test/{segment}"));
    }

    #[test]
    fn redacts_short_path_codes_and_bare_query_tokens() {
        let safe = redact_url("https://example.test/reset/123456?BARE_PRIVATE_CODE");
        assert_eq!(
            safe,
            "https://example.test/{segment}/{segment}?parameter=[REDACTED]"
        );
        assert!(!safe.contains("reset"));
        assert!(!safe.contains("123456"));
        assert!(!safe.contains("BARE_PRIVATE_CODE"));

        let secret_key = redact_url("https://example.test/api?BARE_PRIVATE_CODE=ordinary&limit=25");
        assert_eq!(
            secret_key,
            "https://example.test/{segment}?parameter=[REDACTED]&limit=[REDACTED]"
        );
        assert!(!secret_key.contains("BARE_PRIVATE_CODE"));
    }

    #[test]
    fn redacts_assignments_otp_jwt_and_private_paths_in_free_text() {
        let jwt = "eyJabcdefghijk.abcdefghijklmnop.abcdefghijklmnop";
        let raw = Value::String(format!(
            "password=hunter2 otp: 123456 jwt {jwt} at C:\\Users\\josep\\secret\\token.txt and \\\\server\\private\\token.txt"
        ));
        let safe = redact_network_value(&raw).as_str().unwrap().to_string();
        for sentinel in ["hunter2", "123456", jwt, "josep", "server"] {
            assert!(!safe.contains(sentinel), "leaked sentinel: {sentinel}");
        }
        assert!(safe.contains("password=[REDACTED]"));
        assert!(safe.contains("[PRIVATE_PATH]"));
    }

    #[test]
    fn redacts_editable_and_sensitive_control_values() {
        let raw = json!({
            "controls": [
                {"type": "input", "name": "email", "value": "person@example.com"},
                {"type": "email", "tag": "input", "value": "type-first@example.com"},
                {"role": "textbox", "autocomplete": "one-time-code", "value": "654321", "text": "654321"},
                {"role": "combobox", "value": "PRIVATE_SELECTION"},
                {"type": "checkbox", "value": "PRIVATE_CHECKBOX"}
            ]
        });
        let safe = redact_network_value(&raw).to_string();
        assert!(!safe.contains("person@example.com"));
        assert!(!safe.contains("type-first@example.com"));
        assert!(!safe.contains("654321"));
        assert!(!safe.contains("PRIVATE_SELECTION"));
        assert!(!safe.contains("PRIVATE_CHECKBOX"));
    }

    #[test]
    fn redacts_minimal_name_value_pairs_and_login_variable_keys() {
        let raw = json!({
            "minimal_cookie": {"name": "session", "value": "COOKIE_WITHOUT_METADATA"},
            "variables_final": {
                "username": "person@example.com",
                "credential_name": "primary-vault-entry",
                "totp_name": "primary-totp",
                "otp": "147258",
                "pin": "1357"
            }
        });
        let safe = redact_network_value(&raw).to_string();
        for sentinel in [
            "COOKIE_WITHOUT_METADATA",
            "person@example.com",
            "primary-vault-entry",
            "primary-totp",
            "147258",
            "1357",
        ] {
            assert!(!safe.contains(sentinel), "leaked sentinel: {sentinel}");
        }
    }

    #[test]
    fn redacts_sensitive_values_inside_stringified_nested_json() {
        let raw = Value::String(
            r#"{"outer":{"headers":{"Authorization":"Bearer STRING_SENTINEL"},"cookies":[{"name":"session","value":"COOKIE_SENTINEL","domain":"example.test"}]}}"#
                .into(),
        );
        let safe = redact_network_value(&raw).as_str().unwrap().to_string();
        assert!(!safe.contains("STRING_SENTINEL"));
        assert!(!safe.contains("COOKIE_SENTINEL"));
        assert!(safe.contains("Authorization"));
        assert!(safe.contains("[REDACTED]"));
    }

    #[test]
    fn cookie_lists_keep_metadata_but_never_values() {
        let raw = json!({
            "cookies": [{
                "name": "ordinary_name",
                "value": "private-cookie-value",
                "domain": "example.com",
                "httpOnly": true
            }]
        });
        let safe = redact_network_value(&raw).to_string();
        assert!(safe.contains("ordinary_name"));
        assert!(safe.contains("example.com"));
        assert!(!safe.contains("private-cookie-value"));
    }

    #[test]
    fn trace_redaction_removes_click_text_and_resolved_params() {
        let raw = json!({
            "entries": [
                {"type": "click", "text": "private button text", "name": "BUTTON#submit"},
                {"type": "script_step", "details": {"params": {"text": "private password"}}}
            ]
        });
        let safe = redact_trace_value(&raw).to_string();
        assert!(!safe.contains("private button text"));
        assert!(!safe.contains("private password"));
        assert!(safe.contains("BUTTON#submit"));
    }

    #[test]
    fn strict_trace_projection_drops_hostile_page_fields() {
        let raw = json!({
            "count": 999,
            "arbitrary": "TOP_LEVEL_SECRET",
            "entries": [
                {
                    "type": "click",
                    "tag": "button",
                    "text": "CLICK_TEXT_SECRET",
                    "method": "COOKIE_SENTINEL",
                    "name": "BUTTON#private"
                },
                {
                    "type": "script_step",
                    "name": "browser_click",
                    "details": {
                        "step": 2,
                        "parameter_keys": ["selector", "bad key", "text"],
                        "params": {"text": "PARAM_SECRET"}
                    },
                    "extra": "EXTRA_SECRET"
                },
                {
                    "type": "navigation",
                    "name": "https://user:pass@example.test/reset/123456?token=URL_SECRET#private"
                }
            ]
        });
        let projected = project_trace_value(&raw);
        let text = projected.to_string();
        for sentinel in [
            "TOP_LEVEL_SECRET",
            "CLICK_TEXT_SECRET",
            "COOKIE_SENTINEL",
            "BUTTON#private",
            "PARAM_SECRET",
            "EXTRA_SECRET",
            "URL_SECRET",
            "user:pass",
            "123456",
        ] {
            assert!(!text.contains(sentinel), "leaked sentinel: {sentinel}");
        }
        assert_eq!(projected["count"], json!(3));
        assert_eq!(projected["entries"][0]["tag"], json!("BUTTON"));
        assert_eq!(
            projected["entries"][1]["details"]["parameter_keys"],
            json!(["selector", "text"])
        );
    }

    #[test]
    fn performance_projection_drops_page_owned_arbitrary_fields() {
        let raw = json!({
            "entries": [{
                "name": "https://user:pass@example.test/api/secret?token=PERF_SECRET#private",
                "entryType": "resource",
                "initiatorType": "fetch",
                "duration": 12.5,
                "note": "NOTE_SECRET",
                "headers": {"cookie": "COOKIE_SECRET"}
            }],
            "arbitrary": "TOP_SECRET"
        });
        let safe = project_performance_log_value(&raw);
        let text = safe.to_string();
        for sentinel in [
            "user:pass",
            "secret?",
            "PERF_SECRET",
            "NOTE_SECRET",
            "COOKIE_SECRET",
            "TOP_SECRET",
        ] {
            assert!(!text.contains(sentinel), "leaked sentinel: {sentinel}");
        }
        assert_eq!(safe["count"], json!(1));
        assert_eq!(safe["entries"][0]["entry_type"], json!("resource"));
        assert_eq!(safe["entries"][0]["initiator_type"], json!("fetch"));
    }
}
