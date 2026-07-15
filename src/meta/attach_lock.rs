#![allow(dead_code)] // public handle_* invoked via main.rs dispatch
//! `hands_attach_lock_*` — cross-process attach arbitration (Item 6, Milestone A).
//!
//! Provides a coordination primitive that lets multiple `hands.exe` instances
//! cooperate over Chrome debug ports. `browser_attach` lives in the
//! `browser-mcp` crate so hands.exe can't transparently intercept it — instead
//! agents call:
//!
//!   1. `hands_attach_lock_acquire(port)` → block until lock_id is held
//!   2. `browser_attach(port)`            → actual CDP attach
//!   3. `hands_attach_lock_release(lock_id)` → after detaching
//!
//! Lock files live in `cpc_paths::data_path("hands")/attach_locks/chrome_attach_<port>.lock`.
//! Each holds a single-line JSON record (`LockRecord`). Stale-check is
//! purely timestamp-based — no PID liveness probing — with a default TTL of 30
//! minutes (overridable via `stale_after_ms`). The PID is recorded for
//! forensics only.
//!
//! Pure local-state — no browser/session. Wired via the special-case path in
//! `main.rs::handle_tool_call_inner` (like vision_cache_stats and
//! hands_summarize_run).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

/// Default total wait for `acquire` before giving up.
const DEFAULT_TIMEOUT_MS: u64 = 10_000;
/// Default stale TTL — older locks are eligible for overtake.
const DEFAULT_STALE_AFTER_MS: u64 = 30 * 60 * 1_000; // 30 minutes
/// Sleep between retries when contending for a held lock.
const RETRY_SLEEP_MS: u64 = 50;

/// Persisted form of a held lock. Written as a single JSON line + newline.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LockRecord {
    pub lock_id: String,
    pub pid: u32,
    pub port: u16,
    /// RFC3339 timestamp (UTC) when the lock was acquired.
    pub acquired_at: String,
    pub host: String,
}

/// In-process map of `lock_id → on-disk path`. Populated on successful acquire,
/// consulted on release. Backed by a Mutex<Option<…>> so we don't pay the
/// HashMap allocation cost on cold start.
static HELD_LOCKS: Mutex<Option<HashMap<String, PathBuf>>> = Mutex::new(None);
static AUTO_ATTACH_LOCKS: Mutex<Option<HashMap<u16, String>>> = Mutex::new(None);

fn with_held_locks<F, R>(f: F) -> R
where
    F: FnOnce(&mut HashMap<String, PathBuf>) -> R,
{
    let mut guard = HELD_LOCKS.lock().expect("attach_lock HELD_LOCKS poisoned");
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    f(guard.as_mut().expect("HELD_LOCKS just initialized"))
}

fn with_auto_attach_locks<F, R>(f: F) -> R
where
    F: FnOnce(&mut HashMap<u16, String>) -> R,
{
    let mut guard = AUTO_ATTACH_LOCKS
        .lock()
        .expect("attach_lock AUTO_ATTACH_LOCKS poisoned");
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    f(guard.as_mut().expect("AUTO_ATTACH_LOCKS just initialized"))
}

fn held_lock_id_for_port(port: u16) -> Option<String> {
    with_held_locks(|locks| {
        locks.iter().find_map(|(lock_id, path)| {
            read_lock_if_exists(path)
                .filter(|record| record.port == port && record.pid == std::process::id())
                .map(|_| lock_id.clone())
        })
    })
}

// ──────────────────────────────────────────────────────────────────────
// Path helpers
// ──────────────────────────────────────────────────────────────────────

/// Default lock directory under `cpc_paths::data_path("hands")`. Returns an
/// error string on resolution failure so the caller can surface it.
fn default_lock_dir() -> Result<PathBuf, String> {
    let base = cpc_paths::data_path("hands")
        .map_err(|e| format!("cpc_paths::data_path(hands) failed: {}", e))?;
    Ok(base.join("attach_locks"))
}

fn lock_filename(port: u16) -> String {
    format!("chrome_attach_{}.lock", port)
}

fn lock_path_in(dir: &Path, port: u16) -> PathBuf {
    dir.join(lock_filename(port))
}

/// Parse a `chrome_attach_<port>.lock` filename → port. Returns None on any
/// other shape.
fn parse_port_from_filename(name: &str) -> Option<u16> {
    let stem = name.strip_suffix(".lock")?;
    let rest = stem.strip_prefix("chrome_attach_")?;
    rest.parse::<u16>().ok()
}

// ──────────────────────────────────────────────────────────────────────
// File-level primitives
// ──────────────────────────────────────────────────────────────────────

/// Try to atomically create the lock file and write the record. Returns
/// `AlreadyExists` if another process beat us to it.
fn try_create_lock(path: &Path, record: &LockRecord) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let line = serde_json::to_string(record).map_err(|e| io::Error::other(e.to_string()))?;
    let mut bytes = line.into_bytes();
    bytes.push(b'\n');
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Read and parse a lock file. Returns None on missing file, IO error, or
/// parse failure. Parse-failure callers treat the result as stale → overtake.
fn read_lock_if_exists(path: &Path) -> Option<LockRecord> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(raw.trim()).ok()
}

/// Pure timestamp-based stale check. A lock is stale when its `acquired_at`
/// (parsed as RFC3339) is older than `stale_after_ms` relative to `now`.
/// Unparseable timestamps are treated as stale (corrupt → overtake).
fn lock_record_is_stale(record: &LockRecord, stale_after_ms: u64, now: DateTime<Utc>) -> bool {
    match DateTime::parse_from_rfc3339(&record.acquired_at) {
        Ok(parsed) => {
            let age = now.signed_duration_since(parsed.with_timezone(&Utc));
            age.num_milliseconds() as i128 > stale_after_ms as i128
        }
        Err(_) => true,
    }
}

/// Walk the lock directory and return every readable lock record. Files that
/// don't parse as LockRecord are skipped silently — we don't want a single
/// corrupt lock to break `status`.
fn list_all_locks(dir: &Path) -> Vec<(PathBuf, LockRecord)> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(it) => it,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = match p.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with("chrome_attach_") || !name.ends_with(".lock") {
            continue;
        }
        if let Some(rec) = read_lock_if_exists(&p) {
            out.push((p, rec));
        }
    }
    // Stable order: by port asc.
    out.sort_by_key(|(_, r)| r.port);
    out
}

// ──────────────────────────────────────────────────────────────────────
// UUID-v4-shape generator (no external crate)
// ──────────────────────────────────────────────────────────────────────

/// Generate a 36-char string in UUID v4 shape (`xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`).
/// This is NOT a cryptographic UUID — it's a collision-resistant id for
/// in-process bookkeeping. Sources of entropy:
///   - `SystemTime::now()` nanos (host clock)
///   - `Instant::now()` elapsed nanos since boot
///   - `std::process::id()` (current PID)
///   - A monotonically increasing per-process counter
fn new_lock_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let elapsed = Instant::now().elapsed().as_nanos() as u64;
    let pid = std::process::id() as u64;

    // splitmix64 mix
    let mut state = nanos
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(elapsed)
        .wrapping_add(pid << 17)
        .wrapping_add(counter.wrapping_mul(0xBF58476D1CE4E5B9));

    fn next(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    let mut bytes = [0u8; 16];
    let a = next(&mut state);
    let b = next(&mut state);
    bytes[0..8].copy_from_slice(&a.to_le_bytes());
    bytes[8..16].copy_from_slice(&b.to_le_bytes());

    // Set version (4) and variant (10xx) per RFC 4122.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

fn host_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown-host".to_string())
}

// ──────────────────────────────────────────────────────────────────────
// Public handlers
// ──────────────────────────────────────────────────────────────────────

/// `hands_attach_lock_acquire` handler.
pub fn handle_acquire(args: &Value) -> Value {
    let port = match args.get("port").and_then(|v| v.as_u64()) {
        Some(p) if p > 0 && p <= u16::MAX as u64 => p as u16,
        _ => {
            return json!({
                "ok": false,
                "error": "missing or invalid required parameter: port (u16 > 0)"
            });
        }
    };
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_MS);
    let stale_after_ms = args
        .get("stale_after_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_STALE_AFTER_MS);

    let dir = match default_lock_dir() {
        Ok(d) => d,
        Err(e) => return json!({"ok": false, "error": e}),
    };

    acquire_inner(&dir, port, timeout_ms, stale_after_ms)
}

/// Acquire and retain the debug-port lock used by the canonical
/// `browser_attach` path. The lock remains held until `browser_close` or a
/// failed attach releases it. An existing manual lock held by this process is
/// respected and is not taken over by automatic lifecycle management.
pub fn auto_acquire_for_attach(port: u16) -> Result<&'static str, Value> {
    let dir = default_lock_dir().map_err(|error| json!({"ok": false, "error": error}))?;
    auto_acquire_inner(&dir, port, DEFAULT_TIMEOUT_MS, DEFAULT_STALE_AFTER_MS)
}

fn auto_acquire_inner(
    dir: &Path,
    port: u16,
    timeout_ms: u64,
    stale_after_ms: u64,
) -> Result<&'static str, Value> {
    if with_auto_attach_locks(|locks| locks.contains_key(&port)) {
        return Ok("already_automatic");
    }
    if held_lock_id_for_port(port).is_some() {
        return Ok("manual_lock_already_held");
    }

    let acquired = acquire_inner(dir, port, timeout_ms, stale_after_ms);
    if acquired.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(acquired);
    }
    let Some(lock_id) = acquired
        .get("lock_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Err(json!({
            "ok": false,
            "error": "automatic attach lock acquired without a lock_id"
        }));
    };
    with_auto_attach_locks(|locks| locks.insert(port, lock_id));
    Ok("acquired")
}

/// Release an automatically managed lock for one debug port.
pub fn auto_release_for_port(port: u16) -> Value {
    let dir = match default_lock_dir() {
        Ok(dir) => dir,
        Err(error) => return json!({"ok": false, "error": error}),
    };
    auto_release_inner(&dir, port)
}

fn auto_release_inner(dir: &Path, port: u16) -> Value {
    let lock_id = with_auto_attach_locks(|locks| locks.remove(&port));
    match lock_id {
        Some(lock_id) => release_inner(dir, &lock_id),
        None => json!({
            "ok": true,
            "released": false,
            "port": port,
            "note": "no automatically managed attach lock was held"
        }),
    }
}

/// Release every automatically managed debug-port lock after browser close.
pub fn auto_release_all() -> Value {
    let dir = match default_lock_dir() {
        Ok(dir) => dir,
        Err(error) => return json!({"ok": false, "error": error}),
    };
    let held: Vec<(u16, String)> = with_auto_attach_locks(|locks| locks.drain().collect());
    let releases: Vec<Value> = held
        .into_iter()
        .map(|(port, lock_id)| {
            let result = release_inner(&dir, &lock_id);
            json!({"port": port, "result": result})
        })
        .collect();
    json!({"ok": true, "releases": releases})
}

/// Testable inner with injected `dir`.
fn acquire_inner(dir: &Path, port: u16, timeout_ms: u64, stale_after_ms: u64) -> Value {
    let lock_path = lock_path_in(dir, port);
    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);

    loop {
        let now = Utc::now();
        let acquired_at = now.to_rfc3339();
        let lock_id = new_lock_id();
        let record = LockRecord {
            lock_id: lock_id.clone(),
            pid: std::process::id(),
            port,
            acquired_at: acquired_at.clone(),
            host: host_name(),
        };

        match try_create_lock(&lock_path, &record) {
            Ok(()) => {
                with_held_locks(|m| m.insert(lock_id.clone(), lock_path.clone()));
                return json!({
                    "ok": true,
                    "lock_id": lock_id,
                    "port": port,
                    "acquired_at": acquired_at,
                    "waited_ms": start.elapsed().as_millis() as u64,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // Inspect existing.
                match read_lock_if_exists(&lock_path) {
                    Some(existing) => {
                        if lock_record_is_stale(&existing, stale_after_ms, now) {
                            // Overtake: remove and retry immediately.
                            let _ = fs::remove_file(&lock_path);
                            continue;
                        }
                        if start.elapsed() >= timeout {
                            let age_ms = DateTime::parse_from_rfc3339(&existing.acquired_at)
                                .map(|p| {
                                    now.signed_duration_since(p.with_timezone(&Utc))
                                        .num_milliseconds()
                                        .max(0)
                                })
                                .unwrap_or(0);
                            return json!({
                                "ok": false,
                                "error": "timeout waiting for lock",
                                "current_holder": {
                                    "lock_id": existing.lock_id,
                                    "pid": existing.pid,
                                    "acquired_at": existing.acquired_at,
                                    "age_ms": age_ms,
                                    "host": existing.host,
                                },
                                "waited_ms": start.elapsed().as_millis() as u64,
                            });
                        }
                    }
                    None => {
                        // File exists but unreadable / corrupt → overtake.
                        let _ = fs::remove_file(&lock_path);
                        continue;
                    }
                }
                if start.elapsed() >= timeout {
                    return json!({
                        "ok": false,
                        "error": "timeout waiting for lock",
                        "current_holder": Value::Null,
                        "waited_ms": start.elapsed().as_millis() as u64,
                    });
                }
                thread::sleep(Duration::from_millis(RETRY_SLEEP_MS));
            }
            Err(e) => {
                return json!({
                    "ok": false,
                    "error": format!("failed to create lock file {}: {}", lock_path.display(), e),
                    "waited_ms": start.elapsed().as_millis() as u64,
                });
            }
        }
    }
}

/// `hands_attach_lock_release` handler.
pub fn handle_release(args: &Value) -> Value {
    let lock_id = match args.get("lock_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return json!({
                "ok": false,
                "error": "missing required parameter: lock_id"
            });
        }
    };

    let dir = match default_lock_dir() {
        Ok(d) => d,
        Err(e) => return json!({"ok": false, "error": e}),
    };

    release_inner(&dir, &lock_id)
}

/// Testable inner with injected `dir`.
fn release_inner(dir: &Path, lock_id: &str) -> Value {
    // 1. Check the in-process map first.
    let mapped_path = with_held_locks(|m| m.remove(lock_id));

    if let Some(path) = mapped_path {
        let existing = read_lock_if_exists(&path);
        match existing {
            Some(rec) if rec.lock_id == lock_id => {
                let _ = fs::remove_file(&path);
                return json!({
                    "ok": true,
                    "lock_id": lock_id,
                    "released": true,
                });
            }
            Some(_) => {
                // Lock at this path now belongs to someone else — do NOT delete.
                return json!({
                    "ok": true,
                    "lock_id": lock_id,
                    "released": false,
                    "note": "lock file at recorded path now held by a different lock_id; not deleting",
                });
            }
            None => {
                // Already gone — idempotent success.
                return json!({
                    "ok": true,
                    "lock_id": lock_id,
                    "released": false,
                    "note": "lock file already removed",
                });
            }
        }
    }

    // 2. Not in-process — search on disk for a matching lock_id owned by us.
    let me = std::process::id();
    for (path, rec) in list_all_locks(dir) {
        if rec.lock_id == lock_id {
            if rec.pid == me {
                let _ = fs::remove_file(&path);
                return json!({
                    "ok": true,
                    "lock_id": lock_id,
                    "released": true,
                    "note": "released from disk (not in process map)",
                });
            } else {
                return json!({
                    "ok": false,
                    "error": "lock_id matches a lock held by a different PID",
                    "holder_pid": rec.pid,
                });
            }
        }
    }

    // 3. Not found anywhere — treat as idempotent already-released.
    json!({
        "ok": true,
        "lock_id": lock_id,
        "released": false,
        "note": "lock_id not found (already released or unknown)",
    })
}

/// `hands_attach_lock_status` handler.
pub fn handle_status(args: &Value) -> Value {
    let port_filter = args.get("port").and_then(|v| v.as_u64()).and_then(|p| {
        if p <= u16::MAX as u64 {
            Some(p as u16)
        } else {
            None
        }
    });

    let dir = match default_lock_dir() {
        Ok(d) => d,
        Err(e) => return json!({"ok": false, "error": e}),
    };

    status_inner(&dir, port_filter)
}

/// Testable inner with injected `dir`.
fn status_inner(dir: &Path, port_filter: Option<u16>) -> Value {
    let now = Utc::now();
    let me = std::process::id();
    let owned_set = with_held_locks(|m| m.keys().cloned().collect::<Vec<_>>());

    let mut locks = Vec::new();
    for (path, rec) in list_all_locks(dir) {
        if let Some(p) = port_filter {
            if rec.port != p {
                continue;
            }
        }
        let age_ms = DateTime::parse_from_rfc3339(&rec.acquired_at)
            .map(|p| {
                now.signed_duration_since(p.with_timezone(&Utc))
                    .num_milliseconds()
                    .max(0)
            })
            .unwrap_or(0);
        let owned_by_this_process = owned_set.contains(&rec.lock_id) || rec.pid == me;
        locks.push(json!({
            "port": rec.port,
            "lock_id": rec.lock_id,
            "pid": rec.pid,
            "acquired_at": rec.acquired_at,
            "age_ms": age_ms,
            "host": rec.host,
            "owned_by_this_process": owned_by_this_process,
            "path": path.display().to_string(),
        }));
    }

    let count = locks.len();
    json!({
        "ok": true,
        "locks": locks,
        "count": count,
        "lock_dir": dir.display().to_string(),
    })
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Drop any in-process held-lock entries created by a prior test.
    fn reset_held() {
        with_held_locks(|m| m.clear());
    }

    #[test]
    fn acquire_creates_lock_file() {
        reset_held();
        let dir = TempDir::new().unwrap();
        let out = acquire_inner(dir.path(), 9101, 1_000, 60_000);
        assert_eq!(out["ok"], json!(true), "got: {}", out);
        let lock_path = lock_path_in(dir.path(), 9101);
        assert!(
            lock_path.exists(),
            "lock file should exist at {}",
            lock_path.display()
        );
        let raw = fs::read_to_string(&lock_path).unwrap();
        let rec: LockRecord = serde_json::from_str(raw.trim()).unwrap();
        assert_eq!(rec.port, 9101);
        assert_eq!(rec.pid, std::process::id());
    }

    #[test]
    fn acquire_returns_lock_id_uuid_v4_shape() {
        reset_held();
        let dir = TempDir::new().unwrap();
        let out = acquire_inner(dir.path(), 9102, 1_000, 60_000);
        let lock_id = out["lock_id"].as_str().unwrap();
        assert_eq!(lock_id.len(), 36, "expected 36-char UUID, got {}", lock_id);
        let parts: Vec<&str> = lock_id.split('-').collect();
        assert_eq!(parts.len(), 5, "expected 5 dash-separated parts");
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        // Version nibble = 4.
        assert!(
            parts[2].starts_with('4'),
            "version nibble should be 4: {}",
            lock_id
        );
        // Variant nibble in {8,9,a,b}.
        let variant = parts[3].chars().next().unwrap();
        assert!(
            matches!(variant, '8' | '9' | 'a' | 'b'),
            "variant nibble unexpected: {}",
            lock_id
        );
    }

    #[test]
    fn acquire_succeeds_when_no_existing_lock() {
        reset_held();
        let dir = TempDir::new().unwrap();
        let out = acquire_inner(dir.path(), 9103, 1_000, 60_000);
        assert_eq!(out["ok"], json!(true));
        assert_eq!(out["port"], json!(9103));
        assert!(out["acquired_at"].as_str().unwrap().len() > 10);
        assert!(out["waited_ms"].as_u64().unwrap() < 1_000);
    }

    #[test]
    fn acquire_blocks_when_lock_held_then_succeeds_after_release() {
        reset_held();
        let dir = TempDir::new().unwrap();
        let out1 = acquire_inner(dir.path(), 9104, 5_000, 60_000);
        assert_eq!(out1["ok"], json!(true));
        let lock_id1 = out1["lock_id"].as_str().unwrap().to_string();

        // Spawn a thread that releases after 150ms.
        let dir_path = dir.path().to_path_buf();
        let lock_id_clone = lock_id1.clone();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            release_inner(&dir_path, &lock_id_clone)
        });

        // Try to acquire again — should block ~150ms then succeed.
        let out2 = acquire_inner(dir.path(), 9104, 5_000, 60_000);
        assert_eq!(out2["ok"], json!(true), "second acquire failed: {}", out2);
        let waited = out2["waited_ms"].as_u64().unwrap();
        assert!(waited >= 100, "expected to wait >=100ms, got {}ms", waited);

        let _ = releaser.join().unwrap();
    }

    #[test]
    fn acquire_times_out_when_lock_held_by_other() {
        reset_held();
        let dir = TempDir::new().unwrap();
        // Write a fresh lock file by another (fake) PID so release_inner won't
        // remove it from disk and the holder won't appear in our process map.
        let path = lock_path_in(dir.path(), 9105);
        fs::create_dir_all(dir.path()).unwrap();
        let other_record = LockRecord {
            lock_id: "11111111-1111-4111-8111-111111111111".to_string(),
            pid: 999_999_999,
            port: 9105,
            acquired_at: Utc::now().to_rfc3339(),
            host: "other-host".to_string(),
        };
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        f.write_all(serde_json::to_string(&other_record).unwrap().as_bytes())
            .unwrap();

        // Try to acquire with a short timeout — should fail.
        let out = acquire_inner(dir.path(), 9105, 250, 60_000);
        assert_eq!(out["ok"], json!(false), "expected timeout, got: {}", out);
        assert_eq!(out["error"], json!("timeout waiting for lock"));
        let holder = &out["current_holder"];
        assert_eq!(holder["pid"], json!(999_999_999));
        assert_eq!(holder["host"], json!("other-host"));
        assert!(out["waited_ms"].as_u64().unwrap() >= 250);
    }

    #[test]
    fn acquire_overtakes_stale_lock_by_timestamp() {
        reset_held();
        let dir = TempDir::new().unwrap();
        let path = lock_path_in(dir.path(), 9106);
        fs::create_dir_all(dir.path()).unwrap();

        // Write a stale lock dated 2 hours ago.
        let stale_at = (Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        let stale_record = LockRecord {
            lock_id: "deadbeef-dead-4eef-8eef-deadbeefdead".to_string(),
            pid: 999_999_998,
            port: 9106,
            acquired_at: stale_at,
            host: "ghost-host".to_string(),
        };
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        f.write_all(serde_json::to_string(&stale_record).unwrap().as_bytes())
            .unwrap();
        drop(f);

        // With a 30-min staleness window, the 2h-old lock is stale → overtake.
        let out = acquire_inner(dir.path(), 9106, 1_000, 30 * 60 * 1_000);
        assert_eq!(out["ok"], json!(true), "expected overtake, got: {}", out);
        let new_lock_id = out["lock_id"].as_str().unwrap();
        // Our new lock should be on disk now.
        let raw = fs::read_to_string(&path).unwrap();
        let rec: LockRecord = serde_json::from_str(raw.trim()).unwrap();
        assert_eq!(rec.lock_id, new_lock_id);
        assert_eq!(rec.pid, std::process::id());
    }

    #[test]
    fn release_removes_lock_file() {
        reset_held();
        let dir = TempDir::new().unwrap();
        let out = acquire_inner(dir.path(), 9107, 1_000, 60_000);
        let lock_id = out["lock_id"].as_str().unwrap().to_string();
        let path = lock_path_in(dir.path(), 9107);
        assert!(path.exists());

        let rel = release_inner(dir.path(), &lock_id);
        assert_eq!(rel["ok"], json!(true), "release failed: {}", rel);
        assert_eq!(rel["released"], json!(true));
        assert!(!path.exists(), "lock file should be gone after release");
    }

    #[test]
    fn release_unknown_lock_id_returns_error() {
        reset_held();
        let dir = TempDir::new().unwrap();
        // No locks on disk, no map entries.
        let rel = release_inner(dir.path(), "no-such-lock-id");
        // Idempotent success per spec: lock_id not found → ok:true, released:false.
        assert_eq!(
            rel["ok"],
            json!(true),
            "expected idempotent success: {}",
            rel
        );
        assert_eq!(rel["released"], json!(false));
        assert!(rel["note"].as_str().unwrap().contains("not found"));
    }

    #[test]
    fn release_idempotent_for_already_removed_lock_file() {
        reset_held();
        let dir = TempDir::new().unwrap();
        let out = acquire_inner(dir.path(), 9108, 1_000, 60_000);
        let lock_id = out["lock_id"].as_str().unwrap().to_string();
        let path = lock_path_in(dir.path(), 9108);

        // Simulate someone deleting the lock file out from under us.
        fs::remove_file(&path).unwrap();

        let rel = release_inner(dir.path(), &lock_id);
        assert_eq!(rel["ok"], json!(true), "expected idempotent ok: {}", rel);
        // released=false because file was already gone.
        assert_eq!(rel["released"], json!(false));

        // Second release of the same lock_id (already removed from map) →
        // also idempotent success.
        let rel2 = release_inner(dir.path(), &lock_id);
        assert_eq!(rel2["ok"], json!(true));
        assert_eq!(rel2["released"], json!(false));
    }

    #[test]
    fn status_lists_all_locks_in_dir() {
        reset_held();
        let dir = TempDir::new().unwrap();
        let o1 = acquire_inner(dir.path(), 9201, 1_000, 60_000);
        let o2 = acquire_inner(dir.path(), 9202, 1_000, 60_000);
        let o3 = acquire_inner(dir.path(), 9203, 1_000, 60_000);
        assert_eq!(o1["ok"], json!(true));
        assert_eq!(o2["ok"], json!(true));
        assert_eq!(o3["ok"], json!(true));

        // No port filter → all three.
        let status = status_inner(dir.path(), None);
        assert_eq!(status["ok"], json!(true));
        assert_eq!(status["count"], json!(3));
        let locks = status["locks"].as_array().unwrap();
        let ports: Vec<u16> = locks
            .iter()
            .map(|l| l["port"].as_u64().unwrap() as u16)
            .collect();
        assert_eq!(ports, vec![9201, 9202, 9203]);
        for lock in locks {
            assert_eq!(lock["owned_by_this_process"], json!(true));
            assert!(lock["age_ms"].as_u64().unwrap() < 60_000);
        }

        // Port filter narrows.
        let status2 = status_inner(dir.path(), Some(9202));
        assert_eq!(status2["count"], json!(1));
        assert_eq!(status2["locks"][0]["port"], json!(9202));
    }

    #[test]
    fn lock_record_serialization_roundtrip() {
        let rec = LockRecord {
            lock_id: "abcdef01-1234-4567-89ab-cdef01234567".to_string(),
            pid: 12345,
            port: 9222,
            acquired_at: "2026-05-31T11:53:21.123Z".to_string(),
            host: "josep-pc".to_string(),
        };
        let s = serde_json::to_string(&rec).unwrap();
        let parsed: LockRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, rec);
        // Field ordering check — make sure all fields survive.
        assert!(s.contains("\"lock_id\""));
        assert!(s.contains("\"pid\""));
        assert!(s.contains("\"port\""));
        assert!(s.contains("\"acquired_at\""));
        assert!(s.contains("\"host\""));
    }

    #[test]
    fn lock_record_is_stale_respects_ttl() {
        let now = Utc::now();
        // Fresh lock: 5 minutes old, TTL 30 minutes → not stale.
        let fresh = LockRecord {
            lock_id: "x".into(),
            pid: 1,
            port: 9000,
            acquired_at: (now - chrono::Duration::minutes(5)).to_rfc3339(),
            host: "h".into(),
        };
        assert!(!lock_record_is_stale(&fresh, 30 * 60 * 1_000, now));

        // Old lock: 1 hour old, TTL 30 minutes → stale.
        let old = LockRecord {
            lock_id: "x".into(),
            pid: 1,
            port: 9000,
            acquired_at: (now - chrono::Duration::hours(1)).to_rfc3339(),
            host: "h".into(),
        };
        assert!(lock_record_is_stale(&old, 30 * 60 * 1_000, now));

        // Corrupt timestamp → stale.
        let bad = LockRecord {
            lock_id: "x".into(),
            pid: 1,
            port: 9000,
            acquired_at: "not-a-timestamp".into(),
            host: "h".into(),
        };
        assert!(lock_record_is_stale(&bad, 30 * 60 * 1_000, now));
    }

    #[test]
    fn parse_port_from_filename_accepts_valid_names() {
        assert_eq!(
            parse_port_from_filename("chrome_attach_9222.lock"),
            Some(9222)
        );
        assert_eq!(parse_port_from_filename("chrome_attach_1.lock"), Some(1));
        assert_eq!(
            parse_port_from_filename("chrome_attach_65535.lock"),
            Some(65535)
        );
        assert_eq!(parse_port_from_filename("not_a_lock.txt"), None);
        assert_eq!(parse_port_from_filename("chrome_attach_99999.lock"), None);
        assert_eq!(parse_port_from_filename("chrome_attach_abc.lock"), None);
    }

    #[test]
    fn automatic_attach_lock_lifecycle_holds_until_release() {
        let dir = TempDir::new().unwrap();
        with_auto_attach_locks(|locks| locks.clear());

        let state = auto_acquire_inner(dir.path(), 9309, 1_000, 60_000).unwrap();
        assert_eq!(state, "acquired");
        assert!(lock_path_in(dir.path(), 9309).exists());

        let repeated = auto_acquire_inner(dir.path(), 9309, 1_000, 60_000).unwrap();
        assert_eq!(repeated, "already_automatic");

        let released = auto_release_inner(dir.path(), 9309);
        assert_eq!(released["ok"], json!(true));
        assert_eq!(released["released"], json!(true));
        assert!(!lock_path_in(dir.path(), 9309).exists());
    }
}
