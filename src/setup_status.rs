//! First-run status for the model. Paths and Tailscale state are read live.
//! Hostnames and IP addresses are not stored.

use crate::paths::{self, HomeSource};
use regex::Regex;
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

pub const SERVER_INSTRUCTIONS: &str = "\
AI-Hands is a local stdio MCP server for the browser, Windows UI Automation, and vision/OCR on the machine where this process is running. \
On every new session, call hands_setup_status before any other Hands tool. \
Use only the install location, configured-host list, Tailscale state, and MagicDNS name that tool returns. \
Never guess install paths, host config locations, hostnames, or IP addresses. \
Never invent, request creation of, or store a Tailscale auth key; the user signs in themselves. \
If a host has no hands entry, tell the user to run Register-HandsClients.ps1. It writes the installed absolute path and skips hosts that already have a hands entry. \
If Tailscale is missing or logged out, tell the user to run Initialize-HandsOnboarding.ps1. \
ChatGPT desktop does not launch local stdio servers. Follow docs/chatgpt-desktop-mcp.md. Do not open a public URL, Tailscale Funnel, or an unauthenticated tunnel. \
When a sign-in, captcha, UAC prompt, or unexpected dialog blocks you and the owner is away from the computer, call live_screen_request_help. Never type passwords or other secrets for the user. Hand the sign-in to the owner through that live screen, and keep using normal Hands tools while it is open.";

const HANDS_KEYS: &[&str] = &["hands", "ai-hands", "ai_hands"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TailscaleState {
    pub installed: bool,
    pub logged_in: bool,
    pub backend_state: String,
    pub magic_dns_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct HostConfigFact {
    pub id: String,
    pub config_path: PathBuf,
    pub exists: bool,
    pub has_hands: bool,
    pub command: Option<String>,
    pub parse_error: bool,
}

#[derive(Clone, Debug)]
pub struct SetupFacts {
    pub home: PathBuf,
    pub home_source: HomeSource,
    pub executable: Option<PathBuf>,
    pub readme_present: bool,
    pub agents_present: bool,
    pub skill_present: bool,
    pub hosts: Vec<HostConfigFact>,
    pub tailscale: TailscaleState,
}

pub fn tool_definition() -> Value {
    json!({
        "name": "hands_setup_status",
        "description": "Report the Hands install location, which AI host configs already contain a hands stdio entry, and live Tailscale state. Call this before any other Hands tool. Does not guess paths. Does not store hostnames, IP addresses, or auth keys. The MagicDNS name is read from `tailscale status --json` on each call.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    })
}

pub fn handle() -> Value {
    build_setup_status(&collect_setup_facts())
}

pub fn build_setup_status(facts: &SetupFacts) -> Value {
    let hosts: Vec<Value> = facts
        .hosts
        .iter()
        .map(|host| {
            json!({
                "id": host.id,
                "config_path": host.config_path.display().to_string(),
                "config_exists": host.exists,
                "has_hands": host.has_hands,
                "command": host.command,
                "parse_error": host.parse_error,
            })
        })
        .collect();

    let mut value = json!({
        "ok": true,
        "install": {
            "home": facts.home.display().to_string(),
            "source": facts.home_source.as_str(),
            "executable": facts.executable.as_ref().map(|path| path.display().to_string()),
            "readme": facts.readme_present,
            "agents": facts.agents_present,
            "getting_started_skill": facts.skill_present,
        },
        "hosts": hosts,
        "chatgpt_desktop": {
            "stdio": false,
            "transport": "remote_https",
            "note": "ChatGPT desktop does not launch a local stdio server. Use the authenticated tunnel plan. Do not open a public URL."
        },
        "tailscale": {
            "installed": facts.tailscale.installed,
            "logged_in": facts.tailscale.logged_in,
            "backend_state": empty_as_null(&facts.tailscale.backend_state),
            "magic_dns_name": facts.tailscale.magic_dns_name,
            "source": "tailscale status --json"
        },
        "next_step": next_step(facts),
    });
    scrub_tailscale_coordinates(&mut value);
    value
}

fn empty_as_null(value: &str) -> Value {
    if value.is_empty() {
        Value::Null
    } else {
        Value::String(value.to_string())
    }
}

pub fn next_step(facts: &SetupFacts) -> String {
    if !facts.readme_present || !facts.agents_present || !facts.skill_present {
        return "Run Register-HandsClients.ps1. It copies README.md, AGENTS.md, and skills/hands-getting-started/SKILL.md into the Hands home and registers stdio hosts that do not already have a hands entry.".to_string();
    }
    if !facts.tailscale.installed {
        return "Run Initialize-HandsOnboarding.ps1. It can install Tailscale with one UAC prompt (winget id Tailscale.Tailscale, or an MSI you supply with /quiet). Sign in yourself with the login URL from `tailscale up`, or pass your own auth key on that invocation. Hands never creates or stores keys. If you decline Tailscale, the script prints an authenticated Cloudflare Tunnel or ngrok flow. Do not open a public URL or Tailscale Funnel.".to_string();
    }
    if !facts.tailscale.logged_in {
        return "Tailscale is installed but not logged in. Run `tailscale up` and open the login URL it prints, or pass your own auth key to Initialize-HandsOnboarding.ps1 -AuthKey for that invocation only. The key is not written to disk or logs.".to_string();
    }
    if facts
        .hosts
        .iter()
        .any(|host| host.exists && !host.has_hands)
    {
        return "At least one installed host config has no hands entry. Run Register-HandsClients.ps1. It writes this executable's absolute path and skips hosts that already have a hands entry.".to_string();
    }
    if facts.hosts.iter().all(|host| !host.exists) {
        return "No AI host config file was found. Install and launch a host once, then run Register-HandsClients.ps1. Do not invent a config path.".to_string();
    }
    "Hands is registered with the host configs that already exist. The MagicDNS name in this response was read live and is not stored. ChatGPT desktop still needs the authenticated tunnel plan in docs/chatgpt-desktop-mcp.md. Do not publish a public URL.".to_string()
}

pub fn collect_setup_facts() -> SetupFacts {
    let (home, home_source) = paths::resolve_from_process();
    SetupFacts {
        readme_present: home.join("README.md").is_file(),
        agents_present: home.join("AGENTS.md").is_file(),
        skill_present: home
            .join("skills")
            .join("hands-getting-started")
            .join("SKILL.md")
            .is_file(),
        home,
        home_source,
        executable: std::env::current_exe().ok(),
        hosts: discover_host_configs(),
        tailscale: probe_tailscale(),
    }
}

pub fn discover_host_configs() -> Vec<HostConfigFact> {
    let appdata = env_path("APPDATA");
    let local = env_path("LOCALAPPDATA");
    let profile = env_path("USERPROFILE").or_else(|| env_path("HOME"));
    let claude_config = env_path("CLAUDE_CONFIG_DIR");
    let grok_home = env_path("GROK_HOME");
    discover_host_configs_from(&HostRoots {
        appdata: appdata.as_deref(),
        local_app_data: local.as_deref(),
        user_profile: profile.as_deref(),
        claude_config_dir: claude_config.as_deref(),
        grok_home: grok_home.as_deref(),
    })
}

struct HostRoots<'a> {
    appdata: Option<&'a Path>,
    local_app_data: Option<&'a Path>,
    user_profile: Option<&'a Path>,
    claude_config_dir: Option<&'a Path>,
    grok_home: Option<&'a Path>,
}

fn discover_host_configs_from(roots: &HostRoots<'_>) -> Vec<HostConfigFact> {
    let mut hosts = Vec::new();
    if let Some(appdata) = roots.appdata {
        hosts.push(read_json_host(
            "claude_desktop",
            appdata.join("Claude").join("claude_desktop_config.json"),
        ));
    }
    if let Some(local) = roots.local_app_data {
        let packages = local.join("Packages");
        if let Ok(entries) = fs::read_dir(&packages) {
            let mut names: Vec<_> = entries
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("Claude_"))
                .map(|entry| entry.path())
                .collect();
            names.sort();
            for package in names {
                hosts.push(read_json_host(
                    "claude_desktop_msix",
                    package
                        .join("LocalCache")
                        .join("Roaming")
                        .join("Claude")
                        .join("claude_desktop_config.json"),
                ));
            }
        }
    }
    if let Some(profile) = roots.user_profile {
        let claude_json = match roots.claude_config_dir {
            Some(dir) => dir.join(".claude.json"),
            None => profile.join(".claude.json"),
        };
        hosts.push(read_json_host("claude_code", claude_json));
        hosts.push(read_toml_host(
            "codex",
            profile.join(".codex").join("config.toml"),
        ));
        hosts.push(read_json_host(
            "gemini_cli",
            profile.join(".gemini").join("settings.json"),
        ));
        let grok = match roots.grok_home {
            Some(dir) => dir.join("config.toml"),
            None => profile.join(".grok").join("config.toml"),
        };
        hosts.push(read_toml_host("grok_cli", grok));
        hosts.push(read_json_host(
            "cursor",
            profile.join(".cursor").join("mcp.json"),
        ));
    }
    hosts
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn read_json_host(id: &str, path: PathBuf) -> HostConfigFact {
    if !path.is_file() {
        return HostConfigFact {
            id: id.to_string(),
            config_path: path,
            exists: false,
            has_hands: false,
            command: None,
            parse_error: false,
        };
    }
    match fs::read_to_string(&path) {
        Ok(text) => {
            let parsed = json_hands_entry(&text);
            HostConfigFact {
                id: id.to_string(),
                config_path: path,
                exists: true,
                has_hands: parsed.has_hands,
                command: parsed.command,
                parse_error: parsed.parse_error,
            }
        }
        Err(_) => HostConfigFact {
            id: id.to_string(),
            config_path: path,
            exists: true,
            has_hands: false,
            command: None,
            parse_error: true,
        },
    }
}

fn read_toml_host(id: &str, path: PathBuf) -> HostConfigFact {
    if !path.is_file() {
        return HostConfigFact {
            id: id.to_string(),
            config_path: path,
            exists: false,
            has_hands: false,
            command: None,
            parse_error: false,
        };
    }
    match fs::read_to_string(&path) {
        Ok(text) => {
            let parsed = toml_hands_entry(&text);
            HostConfigFact {
                id: id.to_string(),
                config_path: path,
                exists: true,
                has_hands: parsed.has_hands,
                command: parsed.command,
                parse_error: false,
            }
        }
        Err(_) => HostConfigFact {
            id: id.to_string(),
            config_path: path,
            exists: true,
            has_hands: false,
            command: None,
            parse_error: true,
        },
    }
}

struct HandsEntry {
    has_hands: bool,
    command: Option<String>,
    parse_error: bool,
}

fn json_hands_entry(text: &str) -> HandsEntry {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return HandsEntry {
            has_hands: false,
            command: None,
            parse_error: true,
        };
    };
    let Some(servers) = value.get("mcpServers").and_then(Value::as_object) else {
        return HandsEntry {
            has_hands: false,
            command: None,
            parse_error: false,
        };
    };
    for (key, entry) in servers {
        if HANDS_KEYS
            .iter()
            .any(|candidate| key.eq_ignore_ascii_case(candidate))
        {
            let command = entry
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_string);
            return HandsEntry {
                has_hands: true,
                command,
                parse_error: false,
            };
        }
    }
    HandsEntry {
        has_hands: false,
        command: None,
        parse_error: false,
    }
}

fn toml_hands_entry(text: &str) -> HandsEntry {
    let Some(section) = hands_toml_section(text) else {
        return HandsEntry {
            has_hands: false,
            command: None,
            parse_error: false,
        };
    };
    let command_re = toml_command_pattern();
    let command = command_re
        .captures(section)
        .and_then(|caps| caps.get(1).or_else(|| caps.get(2)))
        .map(|value| unescape_toml_basic(value.as_str()));
    HandsEntry {
        has_hands: true,
        command,
        parse_error: false,
    }
}

fn hands_toml_section(text: &str) -> Option<&str> {
    let header = toml_header_pattern();
    let start = header.find(text)?;
    let rest = &text[start.end()..];
    let next = toml_any_header()
        .find(rest)
        .map(|found| found.start())
        .unwrap_or(rest.len());
    Some(&rest[..next])
}

fn toml_header_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?im)^\s*\[mcp_servers\.(?:"hands"|"ai-hands"|"ai_hands"|hands|ai-hands|ai_hands)\]\s*(?:#.*)?$"#,
        )
        .expect("hands toml header pattern")
    })
}

fn toml_any_header() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"(?m)^\s*\[").expect("toml header pattern"))
}

fn toml_command_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r#"(?m)^\s*command\s*=\s*(?:"([^"\\]*(?:\\.[^"\\]*)*)"|'([^']*)')"#)
            .expect("toml command pattern")
    })
}

fn unescape_toml_basic(value: &str) -> String {
    value.replace("\\\\", "\\").replace("\\\"", "\"")
}

pub fn parse_tailscale_status(text: &str) -> Result<TailscaleState, ()> {
    let value: Value = serde_json::from_str(text).map_err(|_| ())?;
    let backend = value
        .get("BackendState")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let logged_in = backend.eq_ignore_ascii_case("Running");
    let dns = value
        .pointer("/Self/DNSName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .trim_end_matches('.')
        .to_string();
    let magic_dns_name = if logged_in && !dns.is_empty() {
        Some(dns)
    } else {
        None
    };
    Ok(TailscaleState {
        installed: true,
        logged_in,
        backend_state: backend,
        magic_dns_name,
    })
}

fn probe_tailscale() -> TailscaleState {
    match run_tailscale_status() {
        Ok(stdout) => parse_tailscale_status(&stdout).unwrap_or(TailscaleState {
            installed: true,
            logged_in: false,
            backend_state: String::new(),
            magic_dns_name: None,
        }),
        Err(TailscaleError::NotInstalled) => TailscaleState {
            installed: false,
            logged_in: false,
            backend_state: String::new(),
            magic_dns_name: None,
        },
        Err(TailscaleError::StatusFailed) => TailscaleState {
            installed: true,
            logged_in: false,
            backend_state: String::new(),
            magic_dns_name: None,
        },
    }
}

enum TailscaleError {
    NotInstalled,
    StatusFailed,
}

pub(crate) fn tailscale_status_text() -> Result<String, ()> {
    run_tailscale_status().map_err(|_| ())
}

fn run_tailscale_status() -> Result<String, TailscaleError> {
    let mut candidates = vec![PathBuf::from("tailscale")];
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        candidates.push(
            PathBuf::from(program_files)
                .join("Tailscale")
                .join(tailscale_file_name()),
        );
    }
    if let Ok(program_files) = std::env::var("ProgramW6432") {
        candidates.push(
            PathBuf::from(program_files)
                .join("Tailscale")
                .join(tailscale_file_name()),
        );
    }
    let mut saw_binary = false;
    for candidate in candidates {
        if candidate != Path::new("tailscale") && !candidate.is_file() {
            continue;
        }
        match spawn_tailscale_status(&candidate) {
            Ok(stdout) => return Ok(stdout),
            Err(TailscaleError::NotInstalled) => continue,
            Err(TailscaleError::StatusFailed) => {
                saw_binary = true;
            }
        }
    }
    if saw_binary {
        Err(TailscaleError::StatusFailed)
    } else {
        Err(TailscaleError::NotInstalled)
    }
}

fn spawn_tailscale_status(candidate: &Path) -> Result<String, TailscaleError> {
    let mut child = Command::new(candidate)
        .args(["status", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                TailscaleError::NotInstalled
            } else {
                TailscaleError::StatusFailed
            }
        })?;
    let Some(pipe) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(TailscaleError::StatusFailed);
    };
    // Drain stdout while waiting: a status larger than the pipe buffer would
    // otherwise block the process until the timeout killed it.
    let reader = thread::spawn(move || {
        let mut stdout = String::new();
        let mut limited = pipe.take(TAILSCALE_STATUS_MAX_BYTES + 1);
        limited.read_to_string(&mut stdout).map(|_| stdout)
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let stdout = reader
                    .join()
                    .map_err(|_| TailscaleError::StatusFailed)?
                    .map_err(|_| TailscaleError::StatusFailed)?;
                return validated_tailscale_status(&stdout);
            }
            Ok(None) if started.elapsed() > Duration::from_secs(5) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(TailscaleError::StatusFailed);
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(TailscaleError::StatusFailed);
            }
        }
    }
}

const TAILSCALE_STATUS_MAX_BYTES: u64 = 1_000_000;

fn validated_tailscale_status(stdout: &str) -> Result<String, TailscaleError> {
    let trimmed = stdout.trim();
    if trimmed.starts_with('{') && trimmed.len() as u64 <= TAILSCALE_STATUS_MAX_BYTES {
        Ok(trimmed.to_string())
    } else {
        Err(TailscaleError::StatusFailed)
    }
}

fn tailscale_file_name() -> &'static str {
    if cfg!(windows) {
        "tailscale.exe"
    } else {
        "tailscale"
    }
}

/// Drop Tailscale CGNAT and ULA addresses if one ever reaches a string field.
fn scrub_tailscale_coordinates(value: &mut Value) {
    match value {
        Value::String(text) => {
            if contains_tailscale_address(text) {
                *text = "[redacted]".to_string();
            }
        }
        Value::Array(items) => {
            for item in items {
                scrub_tailscale_coordinates(item);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                scrub_tailscale_coordinates(item);
            }
        }
        _ => {}
    }
}

fn contains_tailscale_address(text: &str) -> bool {
    if text.to_ascii_lowercase().contains("fd7a:115c") {
        return true;
    }
    for token in text.split(|ch: char| !ch.is_ascii_digit() && ch != '.') {
        let mut parts = token.split('.');
        let Some(first) = parts.next().and_then(|part| part.parse::<u8>().ok()) else {
            continue;
        };
        let Some(second) = parts.next().and_then(|part| part.parse::<u8>().ok()) else {
            continue;
        };
        let third = parts.next().and_then(|part| part.parse::<u8>().ok());
        let fourth = parts.next().and_then(|part| part.parse::<u8>().ok());
        if first == 100
            && (64..128).contains(&second)
            && third.is_some()
            && fourth.is_some()
            && parts.next().is_none()
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn base_facts() -> SetupFacts {
        SetupFacts {
            home: PathBuf::from(r"C:\Users\anyone\AppData\Local\AIWander\Hands"),
            home_source: HomeSource::Default,
            executable: Some(PathBuf::from(
                r"C:\Users\anyone\AppData\Local\AIWander\Hands\bin\hands.exe",
            )),
            readme_present: true,
            agents_present: true,
            skill_present: true,
            hosts: vec![HostConfigFact {
                id: "cursor".into(),
                config_path: PathBuf::from(r"C:\Users\anyone\.cursor\mcp.json"),
                exists: true,
                has_hands: true,
                command: Some(r"C:\Users\anyone\AppData\Local\AIWander\Hands\bin\hands.exe".into()),
                parse_error: false,
            }],
            tailscale: TailscaleState {
                installed: true,
                logged_in: true,
                backend_state: "Running".into(),
                magic_dns_name: Some("this-machine.tailnet.ts.net".into()),
            },
        }
    }

    #[test]
    fn tailscale_status_reports_magic_dns_without_addresses_or_peers() {
        let raw = r#"{
            "BackendState": "Running",
            "Self": {
                "DNSName": "this-machine.tailnet.ts.net.",
                "Online": true,
                "HostName": "personal-laptop",
                "TailscaleIPs": ["100.64.1.2", "fd7a:115c:a1e0::1"]
            },
            "Peer": {
                "node": {
                    "HostName": "other-peer",
                    "DNSName": "other-peer.tailnet.ts.net.",
                    "TailscaleIPs": ["100.99.8.7"]
                }
            }
        }"#;
        let state = parse_tailscale_status(raw).unwrap();
        assert!(state.logged_in);
        assert_eq!(
            state.magic_dns_name.as_deref(),
            Some("this-machine.tailnet.ts.net")
        );
        let facts = SetupFacts {
            tailscale: state,
            ..base_facts()
        };
        let rendered = build_setup_status(&facts).to_string();
        assert!(rendered.contains("this-machine.tailnet.ts.net"));
        for forbidden in [
            "100.64.1.2",
            "100.99.8.7",
            "fd7a:115c",
            "other-peer",
            "personal-laptop",
            "TailscaleIPs",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "status leaked {forbidden}: {rendered}"
            );
        }
    }

    #[test]
    fn logged_out_tailscale_does_not_surface_a_stale_name() {
        let raw = r#"{
            "BackendState": "NeedsLogin",
            "Self": {
                "DNSName": "stale-name.tailnet.ts.net.",
                "TailscaleIPs": ["100.70.0.5"]
            }
        }"#;
        let state = parse_tailscale_status(raw).unwrap();
        assert!(!state.logged_in);
        assert!(state.magic_dns_name.is_none());
        let facts = SetupFacts {
            tailscale: state,
            ..base_facts()
        };
        let rendered = build_setup_status(&facts).to_string();
        assert!(!rendered.contains("stale-name"));
        assert!(!rendered.contains("100.70.0.5"));
        assert!(
            rendered.contains("not logged in")
                || rendered.contains("Sign in")
                || rendered.contains("tailscale up")
        );
    }

    #[test]
    fn json_and_toml_host_entries_are_detected_without_duplicating() {
        let json_text = r#"{"mcpServers":{"hands":{"command":"C:\\Hands\\hands.exe","args":[]}}}"#;
        let parsed = json_hands_entry(json_text);
        assert!(parsed.has_hands);
        assert_eq!(parsed.command.as_deref(), Some(r"C:\Hands\hands.exe"));

        let other = r#"{"mcpServers":{"filesystem":{"command":"npx"}}}"#;
        assert!(!json_hands_entry(other).has_hands);

        let toml_text = r#"
model = "gpt-5.4"
[mcp_servers.hands]
command = "C:\\Hands\\hands.exe"
args = []

[mcp_servers.other]
command = "npx"
"#;
        let toml_parsed = toml_hands_entry(toml_text);
        assert!(toml_parsed.has_hands);
        assert_eq!(toml_parsed.command.as_deref(), Some(r"C:\Hands\hands.exe"));
        assert!(!toml_hands_entry("command = \"hands.exe\"\n").has_hands);
    }

    #[test]
    fn discovery_reads_only_files_under_the_supplied_roots() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("profile");
        let appdata = root.path().join("appdata");
        fs::create_dir_all(profile.join(".cursor")).unwrap();
        fs::create_dir_all(profile.join(".codex")).unwrap();
        fs::create_dir_all(appdata.join("Claude")).unwrap();
        fs::write(
            profile.join(".cursor").join("mcp.json"),
            r#"{"mcpServers":{"AI-Hands":{"command":"D:\\bin\\hands.exe"}}}"#,
        )
        .unwrap();
        fs::write(
            profile.join(".codex").join("config.toml"),
            "[mcp_servers.hands]\ncommand = 'D:\\bin\\hands.exe'\n",
        )
        .unwrap();

        let hosts = discover_host_configs_from(&HostRoots {
            appdata: Some(&appdata),
            local_app_data: None,
            user_profile: Some(&profile),
            claude_config_dir: None,
            grok_home: None,
        });
        let cursor = hosts.iter().find(|host| host.id == "cursor").unwrap();
        assert!(cursor.has_hands);
        assert_eq!(cursor.command.as_deref(), Some(r"D:\bin\hands.exe"));
        let claude = hosts
            .iter()
            .find(|host| host.id == "claude_desktop")
            .unwrap();
        assert!(!claude.exists);
        assert!(!claude.has_hands);
        let codex = hosts.iter().find(|host| host.id == "codex").unwrap();
        assert!(codex.has_hands);
    }

    #[test]
    fn next_step_prefers_guidance_then_tailscale_then_registration() {
        let mut facts = base_facts();
        facts.skill_present = false;
        assert!(next_step(&facts).contains("Register-HandsClients.ps1"));

        facts.skill_present = true;
        facts.tailscale.installed = false;
        facts.tailscale.logged_in = false;
        facts.tailscale.magic_dns_name = None;
        let step = next_step(&facts);
        assert!(step.contains("Initialize-HandsOnboarding.ps1"));
        assert!(step.contains("never creates or stores"));

        facts.tailscale.installed = true;
        assert!(next_step(&facts).contains("tailscale up"));

        facts.tailscale.logged_in = true;
        facts.hosts[0].has_hands = false;
        assert!(next_step(&facts).contains("no hands entry"));
    }

    #[test]
    fn instructions_tell_the_model_to_call_setup_status_first() {
        assert!(SERVER_INSTRUCTIONS.contains("hands_setup_status"));
        assert!(SERVER_INSTRUCTIONS.contains("Never guess"));
        assert!(
            SERVER_INSTRUCTIONS.contains("never creates or stores")
                || SERVER_INSTRUCTIONS.contains("Never invent")
        );
        assert!(SERVER_INSTRUCTIONS.contains("public URL"));
        assert!(SERVER_INSTRUCTIONS.contains("live_screen_request_help"));
        assert!(SERVER_INSTRUCTIONS.contains("Never type passwords"));
    }

    #[test]
    fn toml_header_with_trailing_comment_is_detected() {
        let toml = "[mcp_servers.hands] # local tools\ncommand = 'D:/bin/hands.exe'\n";
        assert!(toml_header_pattern().is_match(toml));
        assert!(hands_toml_section(toml).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn tailscale_status_larger_than_the_pipe_buffer_completes() {
        let dir = std::env::temp_dir().join(format!("hands-ts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("big_status.cmd");
        std::fs::write(
            &script,
            "@powershell -NoProfile -Command \"'{' + ('x' * 300000) + '}'\"\r\n",
        )
        .unwrap();
        let status = spawn_tailscale_status(&script);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(status, Ok(text) if text.len() > 300_000));
    }
}
