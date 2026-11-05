//! Small HTTP listener for the live screen.
//!
//! Headers are checked before any body byte is read. Each request has one
//! deadline for the headers and one for the body, unfinished sockets are
//! closed when the handler returns, and stop shuts the rest down.

use super::policy::access_log_line;
use super::{plan, reply_route, reply_with_body, Incoming, Live, Plan, Reply};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub(super) const MAX_VIEWERS: usize = 4;
const MAX_WORKERS: usize = 32;
const HEADER_LIMIT: usize = 16 * 1024;
const ACCEPT_POLL: Duration = Duration::from_millis(50);
const READ_POLL: Duration = Duration::from_millis(25);

pub(super) fn serve(live: Arc<Live>, listener: TcpListener) {
    let _ = listener.set_nonblocking(true);
    while !live.stop.load(Ordering::SeqCst) && !live.expired() {
        match listener.accept() {
            Ok((stream, _)) => accept_one(&live, stream),
            Err(error) if error.kind() == ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    signal_stop(&live);
}

pub(super) fn signal_stop(live: &Live) {
    live.stop.store(true, Ordering::SeqCst);
    let clients = match live.clients.lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    };
    for (_, stream) in clients {
        let _ = stream.shutdown(Shutdown::Both);
    }
}

/// Returns true only when every connection thread has exited.
pub(super) fn wait_idle(live: &Live) -> bool {
    let Ok(mut count) = live.workers.lock() else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if *count == 0 {
            return true;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        count = match live.workers_cv.wait_timeout(count, remaining) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
}

fn accept_one(live: &Arc<Live>, stream: TcpStream) {
    if !begin_count(&live.workers, MAX_WORKERS) {
        refuse_busy(stream);
        return;
    }
    let worker = Arc::clone(live);
    let spawned = thread::Builder::new()
        .name("hands-live-conn".to_string())
        .spawn(move || {
            let _worker = CountGuard(Arc::clone(&worker));
            handle_connection(worker, stream);
        });
    if spawned.is_err() {
        end_count(&live.workers, &live.workers_cv);
    }
}

fn begin_count(count: &Mutex<usize>, limit: usize) -> bool {
    let Ok(mut count) = count.lock() else {
        return false;
    };
    if *count >= limit {
        return false;
    }
    *count += 1;
    true
}

fn end_count(count: &Mutex<usize>, wake: &Condvar) {
    if let Ok(mut count) = count.lock() {
        *count = count.saturating_sub(1);
        wake.notify_all();
    }
}

struct CountGuard(Arc<Live>);

impl Drop for CountGuard {
    fn drop(&mut self) {
        end_count(&self.0.workers, &self.0.workers_cv);
    }
}

struct SocketGuard {
    live: Arc<Live>,
    peer: Option<std::net::SocketAddr>,
}

impl SocketGuard {
    fn track(live: &Arc<Live>, stream: &TcpStream) -> Self {
        let peer = stream.peer_addr().ok();
        if let (Some(peer), Ok(clone)) = (peer, stream.try_clone()) {
            if let Ok(mut clients) = live.clients.lock() {
                clients.push((Some(peer), clone));
            }
        }
        Self {
            live: Arc::clone(live),
            peer,
        }
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let Some(peer) = self.peer else {
            return;
        };
        let Ok(mut clients) = self.live.clients.lock() else {
            return;
        };
        let mut kept = Vec::with_capacity(clients.len());
        for (addr, stream) in clients.drain(..) {
            if addr == Some(peer) {
                drop(stream);
            } else {
                kept.push((addr, stream));
            }
        }
        *clients = kept;
    }
}

fn refuse_busy(mut stream: TcpStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let _ = write_reply(
        &mut stream,
        &Reply {
            status: 503,
            content_type: "application/json; charset=utf-8",
            body: br#"{"error":"busy"}"#.to_vec(),
            extra: Vec::new(),
            count_viewer: false,
        },
    );
    let _ = stream.shutdown(Shutdown::Both);
}

fn handle_connection(live: Arc<Live>, mut stream: TcpStream) {
    let _socket = SocketGuard::track(&live, &stream);
    let _ = stream.set_nonblocking(true);
    let client_ip = stream.peer_addr().ok().map(|addr| addr.ip());
    let mut logged = None;
    let header_deadline = Instant::now() + live.io_deadline;
    let outcome = (|| {
        let (head, rest) = read_head(&mut stream, &live, header_deadline)?;
        let parsed = parse_request(&head)?;
        logged = Some((parsed.method.clone(), parsed.path.clone()));
        let incoming = Incoming {
            method: &parsed.method,
            path: &parsed.path,
            query: &parsed.query,
            host: parsed.host.as_deref(),
            origin: parsed.origin.as_deref(),
            content_type: parsed.content_type.as_deref(),
            action_header: parsed.action_header.as_deref(),
            if_none_match: parsed.if_none_match.as_deref(),
            content_length: parsed.content_length,
            chunked: parsed.chunked,
        };
        match plan(&live, &incoming, client_ip) {
            Plan::Reply(reply) => Ok(reply),
            Plan::Serve(route) => {
                let _viewer = ViewerGuard::enter(&live)?;
                Ok(reply_route(&live, route, &incoming))
            }
            Plan::ReadBody { length } => {
                let _viewer = ViewerGuard::enter(&live)?;
                let body_deadline = Instant::now() + live.io_deadline;
                let body = read_body(&mut stream, &live, length, rest, body_deadline)?;
                Ok(reply_with_body(&live, &incoming, client_ip, &body))
            }
        }
    })();
    let reply = match outcome {
        Ok(reply) => reply,
        Err(ReadFail::Timeout) => error_reply(408, "timed_out"),
        Err(ReadFail::TooLarge) => error_reply(413, "too_large"),
        Err(ReadFail::Busy) => error_reply(503, "busy"),
        Err(ReadFail::Stopped | ReadFail::Closed) => return,
        Err(ReadFail::Bad) => error_reply(400, "bad_input"),
    };
    if reply.count_viewer {
        live.note_viewer();
    }
    enable_linger(&stream);
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let _ = write_reply(&mut stream, &reply);
    // Half-close, then drop unread bytes. A full shutdown while the client is
    // still sending makes Windows abort the connection and discard this reply.
    let _ = stream.shutdown(Shutdown::Write);
    discard_unread(&mut stream);
    if let Some((method, path)) = logged {
        eprintln!(
            "[hands/live-screen] {}",
            access_log_line(&method, &path, reply.status, &client_label(client_ip))
        );
    }
}

fn client_label(client_ip: Option<std::net::IpAddr>) -> String {
    client_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

enum ReadFail {
    Timeout,
    TooLarge,
    Stopped,
    Closed,
    Bad,
    Busy,
}

struct ViewerGuard(Arc<Live>);

impl ViewerGuard {
    fn enter(live: &Arc<Live>) -> Result<Self, ReadFail> {
        if begin_count(&live.viewers, MAX_VIEWERS) {
            Ok(Self(Arc::clone(live)))
        } else {
            Err(ReadFail::Busy)
        }
    }
}

impl Drop for ViewerGuard {
    fn drop(&mut self) {
        end_count(&self.0.viewers, &self.0.workers_cv);
    }
}

fn read_head(
    stream: &mut TcpStream,
    live: &Live,
    deadline: Instant,
) -> Result<(Vec<u8>, Vec<u8>), ReadFail> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        if let Some(split) = header_split(&buf) {
            let head = buf[..split].to_vec();
            let rest = buf[split + 4..].to_vec();
            return Ok((head, rest));
        }
        if buf.len() >= HEADER_LIMIT {
            return Err(ReadFail::TooLarge);
        }
        let count = read_some(stream, live, deadline, &mut tmp)?;
        buf.extend_from_slice(&tmp[..count]);
    }
}

fn header_split(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn read_body(
    stream: &mut TcpStream,
    live: &Live,
    length: usize,
    mut already: Vec<u8>,
    deadline: Instant,
) -> Result<Vec<u8>, ReadFail> {
    if already.len() > length {
        already.truncate(length);
        return Ok(already);
    }
    let mut tmp = [0u8; 4096];
    while already.len() < length {
        let want = (length - already.len()).min(tmp.len());
        let count = read_some(stream, live, deadline, &mut tmp[..want])?;
        already.extend_from_slice(&tmp[..count]);
    }
    Ok(already)
}

#[cfg(windows)]
fn enable_linger(stream: &TcpStream) {
    use std::os::windows::io::AsRawSocket;
    use windows::Win32::Networking::WinSock::{setsockopt, LINGER, SOCKET, SOL_SOCKET, SO_LINGER};
    let linger = LINGER {
        l_onoff: 1,
        l_linger: 2,
    };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref(&linger).cast::<u8>(),
            std::mem::size_of::<LINGER>(),
        )
    };
    let socket = SOCKET(stream.as_raw_socket() as usize);
    unsafe {
        let _ = setsockopt(socket, SOL_SOCKET, SO_LINGER, Some(bytes));
    }
}

#[cfg(not(windows))]
fn enable_linger(_stream: &TcpStream) {}

fn discard_unread(stream: &mut TcpStream) {
    let _ = stream.set_nonblocking(true);
    let end = Instant::now() + Duration::from_millis(500);
    let mut quiet = Instant::now();
    let mut buf = [0u8; 8192];
    let mut total = 0usize;
    while Instant::now() < end && total < 256 * 1024 {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(count) => {
                total += count;
                quiet = Instant::now();
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error)
                if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::TimedOut =>
            {
                if quiet.elapsed() >= Duration::from_millis(40) {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }
}

fn read_some(
    stream: &mut TcpStream,
    live: &Live,
    deadline: Instant,
    buf: &mut [u8],
) -> Result<usize, ReadFail> {
    loop {
        if live.stop.load(Ordering::SeqCst) {
            return Err(ReadFail::Stopped);
        }
        if Instant::now() >= deadline {
            return Err(ReadFail::Timeout);
        }
        match stream.read(buf) {
            Ok(0) => return Err(ReadFail::Closed),
            Ok(count) => return Ok(count),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error)
                if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::TimedOut =>
            {
                thread::sleep(READ_POLL);
            }
            Err(_) => return Err(ReadFail::Closed),
        }
    }
}

struct Parsed {
    method: String,
    path: String,
    query: String,
    host: Option<String>,
    origin: Option<String>,
    content_type: Option<String>,
    action_header: Option<String>,
    if_none_match: Option<String>,
    content_length: Option<usize>,
    chunked: bool,
}

fn parse_request(head: &[u8]) -> Result<Parsed, ReadFail> {
    let text = std::str::from_utf8(head).map_err(|_| ReadFail::Bad)?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or(ReadFail::Bad)?;
    let mut parts = request_line.split(' ');
    let method = parts.next().ok_or(ReadFail::Bad)?.to_string();
    let target = parts.next().ok_or(ReadFail::Bad)?;
    let version = parts.next().ok_or(ReadFail::Bad)?;
    if parts.next().is_some() || !version.starts_with("HTTP/") || !target.starts_with('/') {
        return Err(ReadFail::Bad);
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (target.to_string(), String::new()),
    };
    let mut parsed = Parsed {
        method,
        path,
        query,
        host: None,
        origin: None,
        content_type: None,
        action_header: None,
        if_none_match: None,
        content_length: None,
        chunked: false,
    };
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(ReadFail::Bad);
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("host") {
            parsed.host = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("origin") {
            parsed.origin = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("content-type") {
            parsed.content_type = Some(value.to_string());
        } else if name.eq_ignore_ascii_case(super::policy::ACTION_HEADER) {
            parsed.action_header = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("if-none-match") {
            parsed.if_none_match = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("content-length") {
            parsed.content_length = Some(value.parse().map_err(|_| ReadFail::Bad)?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            parsed.chunked = true;
        }
    }
    Ok(parsed)
}

fn write_reply(stream: &mut TcpStream, reply: &Reply) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'none'; img-src 'self' blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\n",
        reply.status,
        reason(reply.status),
        reply.content_type,
        reply.body.len()
    );
    for (name, value) in &reply.extra {
        if value
            .bytes()
            .all(|byte| byte.is_ascii() && byte != b'\r' && byte != b'\n')
        {
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&reply.body)?;
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        304 => "Not Modified",
        400 => "Bad Request",
        403 => "Forbidden",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

fn error_reply(status: u16, error: &str) -> Reply {
    Reply {
        status,
        content_type: "application/json; charset=utf-8",
        body: format!(r#"{{"error":"{error}"}}"#).into_bytes(),
        extra: Vec::new(),
        count_viewer: false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::READ_TIMEOUT;
    use super::super::{new_live, spawn_ttl, PAGE};
    use super::{serve, signal_stop, wait_idle};
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    struct Running {
        live: Arc<super::super::Live>,
        http: JoinHandle<()>,
        timer: Option<JoinHandle<()>>,
        addr: SocketAddr,
    }

    impl Running {
        fn open(ttl: Option<Duration>, jpeg: Option<Vec<u8>>) -> Self {
            Self::start(ttl, jpeg, READ_TIMEOUT)
        }

        fn start(ttl: Option<Duration>, jpeg: Option<Vec<u8>>, io_deadline: Duration) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let live = new_live(
                TOKEN.to_string(),
                addr.port(),
                addr.ip(),
                None,
                ttl,
                true,
                jpeg,
                io_deadline,
            );
            let worker = Arc::clone(&live);
            let http = std::thread::spawn(move || serve(worker, listener));
            let timer = spawn_ttl(Arc::clone(&live));
            Self {
                live,
                http,
                timer,
                addr,
            }
        }

        fn stop(self) -> (Duration, bool) {
            let started = Instant::now();
            signal_stop(&self.live);
            let _ = self.http.join();
            if let Some(timer) = self.timer {
                let _ = timer.join();
            }
            let idle = wait_idle(&self.live);
            (started.elapsed(), idle)
        }
    }

    fn exchange(addr: SocketAddr, request: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf);
        let split = buf
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("http response");
        let head = std::str::from_utf8(&buf[..split]).unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        let headers = head
            .split("\r\n")
            .skip(1)
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
            })
            .collect();
        (status, headers, buf[split + 4..].to_vec())
    }

    fn host_line(addr: SocketAddr) -> String {
        format!("127.0.0.1:{}", addr.port())
    }

    #[cfg(windows)]
    fn process_handle_count() -> u32 {
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
        let mut count = 0u32;
        unsafe {
            GetProcessHandleCount(GetCurrentProcess(), &mut count).expect("GetProcessHandleCount");
        }
        count
    }

    #[test]
    fn stalled_body_times_out_without_blocking_other_viewers_and_stop_interrupts() {
        let running = Running::open(None, None);
        let host = host_line(running.addr);
        let stall = format!(
            "POST /{TOKEN}/action HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nX-Hands-Live-Action: 1\r\nOrigin: http://{host}\r\nContent-Length: 65536\r\nConnection: close\r\n\r\n"
        );
        let mut stalled = TcpStream::connect(running.addr).unwrap();
        stalled
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        stalled.write_all(stall.as_bytes()).unwrap();
        let started = Instant::now();
        let page = format!("GET /{TOKEN}/ HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        let (status, _, body) = exchange(running.addr, &page);
        assert_eq!(status, 200);
        assert!(body.windows(11).any(|window| window == b"Live screen"));
        assert!(started.elapsed() < Duration::from_secs(2));

        let mut response = Vec::new();
        let _ = stalled.read_to_end(&mut response);
        let elapsed = started.elapsed();
        assert!(
            elapsed >= READ_TIMEOUT.saturating_sub(Duration::from_millis(500)),
            "timed out too fast: {elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(8), "hung: {elapsed:?}");
        let text = String::from_utf8_lossy(&response);
        assert!(text.contains("408"), "{text}");

        let bad = format!(
            "POST /{TOKEN}/action HTTP/1.1\r\nHost: evil.example\r\nContent-Type: application/json\r\nContent-Length: 65536\r\nConnection: close\r\n\r\n"
        );
        let rejected_at = Instant::now();
        let (status, _, _) = exchange(running.addr, &bad);
        assert_eq!(status, 403);
        assert!(rejected_at.elapsed() < Duration::from_secs(1));

        let mut hung = TcpStream::connect(running.addr).unwrap();
        hung.write_all(stall.as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let (stopped, idle) = running.stop();
        assert!(
            idle,
            "stop reported success while a worker was still running"
        );
        assert!(
            stopped < Duration::from_millis(1000),
            "stop waited on the body: {stopped:?}"
        );
    }

    #[test]
    fn each_connection_gets_a_full_frame_until_its_own_etag_matches() {
        let jpeg = b"jpeg-frame-one".to_vec();
        let running = Running::open(None, Some(jpeg.clone()));
        let host = host_line(running.addr);
        let get = format!(
            "GET /{TOKEN}/frame?w=80 HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        );
        let (status, headers, body) = exchange(running.addr, &get);
        assert_eq!(status, 200);
        assert_eq!(body, jpeg);
        let etag = headers
            .iter()
            .find(|(name, _)| name == "etag")
            .map(|(_, value)| value.clone())
            .expect("etag");
        let (status, _, body) = exchange(running.addr, &get);
        assert_eq!(status, 200, "a viewer with no tag still gets the JPEG");
        assert_eq!(body, jpeg);
        let cached = format!(
            "GET /{TOKEN}/frame?w=80 HTTP/1.1\r\nHost: {host}\r\nIf-None-Match: {etag}\r\nConnection: close\r\n\r\n"
        );
        let (status, _, body) = exchange(running.addr, &cached);
        assert_eq!(status, 304);
        assert!(body.is_empty());
        assert!(PAGE.contains("let etag = ''"));
        assert!(PAGE.contains("if (etag)"));
        assert!(PAGE.contains("if (res.status === 304) return"));
        assert!(!PAGE.contains("unchanged"));
        running.stop();
    }

    #[test]
    fn ttl_stops_the_listener_without_a_request() {
        let running = Running::open(Some(Duration::from_millis(200)), None);
        let addr = running.addr;
        std::thread::sleep(Duration::from_millis(800));
        assert!(running.live.stop.load(Ordering::SeqCst));
        assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err());
        running.stop();
    }

    #[test]
    fn slow_headers_do_not_fill_viewer_slots() {
        let deadline = Duration::from_millis(600);
        let running = Running::start(None, None, deadline);
        let host = host_line(running.addr);
        let page = format!("GET /{TOKEN}/ HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        let mut readers = Vec::new();
        for _ in 0..4 {
            let reader = TcpStream::connect(running.addr).unwrap();
            reader
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut writer = reader.try_clone().unwrap();
            let started = Instant::now();
            std::thread::spawn(move || {
                while started.elapsed() < deadline + Duration::from_millis(1500) {
                    if writer.write_all(b"X").is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
            readers.push((reader, started));
        }
        std::thread::sleep(Duration::from_millis(80));
        let (status, _, _) = exchange(running.addr, &page);
        assert_eq!(
            status, 200,
            "a drip of header bytes must not crowd out a viewer"
        );
        for (mut reader, started) in readers {
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf);
            let elapsed = started.elapsed();
            let text = String::from_utf8_lossy(&buf);
            assert!(
                text.contains("408"),
                "slow header should be rejected at the deadline: {text} after {elapsed:?}"
            );
            assert!(
                elapsed + Duration::from_millis(100) >= deadline,
                "408 arrived too early ({elapsed:?}, deadline {deadline:?})"
            );
            assert!(
                elapsed <= deadline + Duration::from_millis(500),
                "408 arrived late ({elapsed:?}, deadline {deadline:?}); dripping did not hit an overall deadline"
            );
        }
        running.stop();
    }

    #[cfg(windows)]
    #[test]
    fn finished_connections_drop_their_socket_handles() {
        let running = Running::open(None, None);
        let host = host_line(running.addr);
        let page = format!("GET /{TOKEN}/ HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        for _ in 0..10 {
            let (status, _, _) = exchange(running.addr, &page);
            assert_eq!(status, 200);
        }
        let before = process_handle_count();
        for _ in 0..120 {
            let (status, _, _) = exchange(running.addr, &page);
            assert_eq!(status, 200);
        }
        let after = process_handle_count();
        let growth = after.saturating_sub(before);
        assert!(
            growth < 40,
            "process handles grew by {growth} across 120 requests ({before} -> {after})"
        );
        running.stop();
    }

    #[cfg(windows)]
    #[test]
    fn injected_send_failure_returns_typed_count_and_releases_modifiers() {
        let running = Running::open(None, None);
        let host = host_line(running.addr);
        crate::live_screen::platform::arm_send_script(2);
        let payload = r#"{"op":"type","text":"abcd"}"#;
        let typed = format!(
            "POST /{TOKEN}/action HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nX-Hands-Live-Action: 1\r\nOrigin: http://{host}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        );
        let (status, _, body) = exchange(running.addr, &typed);
        assert_eq!(status, 409, "{}", String::from_utf8_lossy(&body));
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"], "input_rejected");
        assert_eq!(value["typed"], 2);
        let hint = value["hint"].as_str().unwrap();
        assert!(hint.contains("does not say why"));
        assert!(hint.contains("secure desktop"));
        assert!(!hint.contains("elevated_window"));

        crate::live_screen::platform::arm_send_script(1);
        let combo = r#"{"op":"combo","keys":"ctrl+shift+t"}"#;
        let request = format!(
            "POST /{TOKEN}/action HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nX-Hands-Live-Action: 1\r\nOrigin: http://{host}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{combo}",
            combo.len()
        );
        let (status, _, body) = exchange(running.addr, &request);
        assert_eq!(status, 409, "{}", String::from_utf8_lossy(&body));
        let log = crate::live_screen::platform::take_send_log();
        assert!(
            log.iter().any(|(key, up)| *up && *key == 0x11),
            "ctrl was not released: {log:?}"
        );
        running.stop();
    }
}
