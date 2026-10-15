//! Loopback JSON-lines control server: one request per line, one reply per line.
//! This is the transport the MCP server (`soundcraft-cli mcp --connect PORT`) wraps.
//!
//! Two modes (design: `docs/soundcraft-ai/control-channel-protection.md`):
//! - developer mode (no token): every control method is forwarded;
//! - assistant mode (a per-launch token): the first message must be `auth`, and only the assistant
//!   allowlist is forwarded.
//!
//! In both modes a connection whose first line is not a JSON object (an HTTP request from a web
//! page, say) is closed unanswered. Lines are bounded ([`MAX_LINE`] in developer mode, far less in
//! assistant mode): a longer one is refused and the connection closed, without buffering the rest.
//! So is the number of connections served at once ([`MAX_CONNECTIONS`]).

use serde_json::{Value, json};
use soundcraft_ui_egui::ControlRequest;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

/// Longest accepted request line (bytes, without the newline). The largest payload a request
/// carries is a plugin state for `mix.insert_state`: base64 of at most `b64::MAX_DECODED_BYTES`
/// (64 MiB, so about 85 MiB of text). Room for that and 1 MiB of JSON around it; replies
/// (screenshots) aren't bounded by this.
const MAX_LINE: usize = soundcraft_engine::model::b64::MAX_DECODED_BYTES.div_ceil(3) * 4 + (1 << 20);
// With MAX_CONNECTIONS, at most 1.5 GiB of request lines are buffered at once.
const _: () = assert!(MAX_LINE <= 96 << 20);
/// Connections served at once; further ones get an error line and are closed.
const MAX_CONNECTIONS: usize = 16;

/// How the server is started.
pub struct ControlConfig {
    pub port: u16,
    /// `Some` puts the server in assistant mode: authentication is required and only the assistant
    /// allowlist is forwarded. `None` is developer mode.
    pub token: Option<String>,
    /// How long a connection may stay silent before it has authenticated.
    pub auth_timeout: Duration,
}

/// Methods an authenticated assistant connection may call (`auth` is answered by the server).
const ASSISTANT_ALLOWLIST: [&str; 3] = ["session.inspect", "session.apply", "session.execution_status"];
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Longest line an assistant connection may send, before and after authenticating.
const MAX_LINE_PRE_AUTH: usize = 4 * 1024;
const MAX_LINE_AUTHED: usize = 1024 * 1024;
const MAX_AUTH_FAILURES: usize = 3;

/// Compares in time independent of where the inputs first differ.
fn same_token(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

pub fn start(port: u16, ctx: egui::Context) -> Receiver<ControlRequest> {
    start_with(ControlConfig { port, token: None, auth_timeout: AUTH_TIMEOUT }, ctx)
}

pub fn start_with(cfg: ControlConfig, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let port = cfg.port;
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control server failed to bind 127.0.0.1:{port}: {e}");
            return rx;
        }
    };
    let actual = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    log::info!("control server listening on 127.0.0.1:{actual} ({} mode)", if cfg.token.is_some() { "assistant" } else { "developer" });
    println!("SOUNDCRAFT_CONTROL_PORT={actual}");
    let cfg = Arc::new(cfg);
    let spawned = std::thread::Builder::new().name("soundcraft-control".into()).spawn(move || accept_loop(listener, tx, ctx, cfg));
    if let Err(e) = spawned {
        log::error!("control thread failed: {e}");
    }
    rx
}

/// A connection's place among [`MAX_CONNECTIONS`], given back when the connection ends (also when
/// its thread can't be started, or panics).
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_loop(listener: TcpListener, tx: Sender<ControlRequest>, ctx: egui::Context, cfg: Arc<ControlConfig>) {
    let open = Arc::new(AtomicUsize::new(0));
    for mut stream in listener.incoming().flatten() {
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            let _ = writeln!(stream, "{}", json!({"ok": false, "error": "too many control connections"}));
            continue;
        }
        let slot = Slot(Arc::clone(&open));
        let tx = tx.clone();
        let ctx = ctx.clone();
        let cfg = Arc::clone(&cfg);
        let _ = std::thread::Builder::new().name("soundcraft-control-conn".into()).spawn(move || {
            let _slot = slot;
            serve(stream, tx, ctx, &cfg);
        });
    }
}

/// Read one `\n`-terminated line of at most `max` bytes into `buf` (newline stripped).
/// `Ok(false)` at end of stream; `Err` when the line is too long or the read fails.
fn read_bounded_line(reader: &mut impl BufRead, buf: &mut Vec<u8>, max: usize) -> Result<bool, String> {
    buf.clear();
    // `take` bounds the bytes buffered for one line: a peer can't make us allocate more than `max`.
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let n = match reader.take(limit).read_until(b'\n', buf) {
        Ok(n) => n,
        // A connection that went quiet past its read timeout (see `ControlConfig::auth_timeout`) is
        // closed without a word, like a peer that hung up.
        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    if n == 0 {
        return Ok(false);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        return Ok(true);
    }
    if buf.len() > max {
        return Err(format!("request line longer than {max} bytes; closing the connection"));
    }
    // Last line without a newline, then end of stream.
    Ok(true)
}

fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context, cfg: &ControlConfig) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
    let mut authed = cfg.token.is_none();
    let (mut first, mut failures) = (true, 0usize);
    loop {
        let (timeout, max) = match (&cfg.token, authed) {
            (Some(_), false) => (Some(cfg.auth_timeout), MAX_LINE_PRE_AUTH),
            (Some(_), true) => (Some(IDLE_TIMEOUT), MAX_LINE_AUTHED),
            (None, _) => (None, MAX_LINE),
        };
        if out.set_read_timeout(timeout).is_err() {
            break;
        }
        match read_bounded_line(&mut reader, &mut buf, max) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok": false, "error": e}));
                break;
            }
        }
        // Bytes that aren't UTF-8 end the connection, as `BufRead::lines` did.
        let Ok(line) = std::str::from_utf8(&buf) else { break };
        if line.trim().is_empty() {
            continue;
        }
        // Only a JSON object may open a conversation: anything else (an HTTP request from a web
        // page, a WebSocket upgrade) is closed without an answer or any forwarding.
        if std::mem::take(&mut first) && !line.trim_start().starts_with('{') {
            log::warn!("control connection closed: first line is not a JSON object");
            break;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(msg) => {
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let mut r = if let (Some(token), "auth") = (&cfg.token, method.as_str()) {
                    let given = params.get("token").and_then(Value::as_str).unwrap_or("");
                    if same_token(token, given) && failures < MAX_AUTH_FAILURES {
                        authed = true;
                        json!({"ok": true, "result": {"scope": "assistant"}})
                    } else {
                        failures += 1;
                        log::warn!("control authentication refused ({failures}/{MAX_AUTH_FAILURES})");
                        json!({"ok": false, "error": "authentication failed"})
                    }
                } else if !authed {
                    json!({"ok": false, "error": "unauthenticated"})
                } else if cfg.token.is_some() && !ASSISTANT_ALLOWLIST.contains(&method.as_str()) {
                    json!({"ok": false, "error": format!("`{method}` is not available to this connection")})
                } else {
                    let (req, rrx) = ControlRequest::new(method, params);
                    if tx.send(req).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                    rrx.recv_timeout(Duration::from_secs(90)).unwrap_or_else(|_| json!({"ok": false, "error": "timeout"}))
                };
                if let Some(o) = r.as_object_mut() {
                    o.insert("id".into(), id);
                }
                r
            }
            Err(e) => json!({"ok": false, "error": format!("bad JSON: {e}")}),
        };
        if writeln!(out, "{reply}").is_err() || failures >= MAX_AUTH_FAILURES {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_read_up_to_the_limit() {
        let mut buf = Vec::new();
        let mut input: &[u8] = b"aaaaaaaaaa\nbb\r\ncc";
        assert_eq!(read_bounded_line(&mut input, &mut buf, 10), Ok(true));
        assert_eq!(buf, b"aaaaaaaaaa", "a line exactly at the limit is accepted");
        assert_eq!(read_bounded_line(&mut input, &mut buf, 10), Ok(true));
        assert_eq!(buf, b"bb", "CRLF is stripped");
        assert_eq!(read_bounded_line(&mut input, &mut buf, 10), Ok(true));
        assert_eq!(buf, b"cc", "a last line without a newline is still read");
        assert_eq!(read_bounded_line(&mut input, &mut buf, 10), Ok(false), "then end of stream");
    }

    #[test]
    fn an_overlong_line_is_refused_without_reading_the_rest() {
        let mut buf = Vec::new();
        let long = [b'x'; 100];
        let mut input: &[u8] = &long;
        assert!(read_bounded_line(&mut input, &mut buf, 10).is_err());
        assert!(buf.len() <= 11, "at most limit + 1 bytes are buffered, got {}", buf.len());
        assert_eq!(input.len(), 100 - 11, "the rest of the line is left unread");
    }

    #[test]
    fn the_largest_plugin_state_fits_in_a_line() {
        let state = soundcraft_engine::model::b64::MAX_DECODED_BYTES.div_ceil(3) * 4;
        let request = json!({"id": 1, "method": "engine.execute", "params": {"command": "mix.insert_state", "params": {"track": "Audio 1", "slot": 0, "state": ""}}});
        assert!(state + request.to_string().len() <= MAX_LINE);
    }

    /// The control server on a loopback port, with a stand-in UI that echoes each method.
    fn serve_echo() -> std::net::SocketAddr {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let cfg = Arc::new(ControlConfig { port: 0, token: None, auth_timeout: AUTH_TIMEOUT });
        std::thread::spawn(move || accept_loop(listener, tx, egui::Context::default(), cfg));
        std::thread::spawn(move || {
            for req in rx {
                let _ = req.reply.send(json!({"ok": true, "method": req.method}));
            }
        });
        addr
    }

    #[test]
    fn requests_are_answered_with_their_ids() {
        let addr = serve_echo();
        let mut c = TcpStream::connect(addr).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        c.write_all(b"{\"id\":3,\"method\":\"ping\"}\r\n\n{\"id\":4,\"method\":\"pong\"}\n").unwrap();
        let mut r = BufReader::new(c);
        for (id, method) in [(3, "ping"), (4, "pong")] {
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let v: Value = serde_json::from_str(&line).unwrap();
            assert_eq!((v["id"].as_i64(), v["method"].as_str()), (Some(id), Some(method)), "{line}");
        }
    }

    #[test]
    fn extra_connections_are_refused() {
        let addr = serve_echo();
        // Idle connections are accepted in order and held open.
        let held: Vec<TcpStream> = (0..MAX_CONNECTIONS).map(|_| TcpStream::connect(addr).unwrap()).collect();
        let mut extra = TcpStream::connect(addr).unwrap();
        extra.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut line = String::new();
        BufReader::new(&mut extra).read_line(&mut line).unwrap();
        assert!(line.contains("too many control connections"), "{line}");
        drop(held);
    }
}

/// Tests for the protected control channel (design in `docs/soundcraft-ai/control-channel-protection.md`).
#[cfg(test)]
mod protection_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// A running server plus a stand-in for the UI thread that records every method it is asked to
    /// run. `seen` is exactly what got past the gate.
    struct Harness {
        port: u16,
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl Harness {
        fn launch(token: Option<&str>) -> Harness {
            let port = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
            let cfg = ControlConfig { port, token: token.map(str::to_string), auth_timeout: Duration::from_millis(300) };
            let rx = start_with(cfg, egui::Context::default());
            let seen: Arc<Mutex<Vec<String>>> = Arc::default();
            let log = Arc::clone(&seen);
            std::thread::spawn(move || {
                for req in rx {
                    log.lock().unwrap().push(req.method.clone());
                    let _ = req.reply.send(json!({"ok": true, "result": {}}));
                }
            });
            let deadline = Instant::now() + Duration::from_secs(3);
            while TcpStream::connect(("127.0.0.1", port)).is_err() {
                assert!(Instant::now() < deadline, "server did not start");
                std::thread::sleep(Duration::from_millis(20));
            }
            Harness { port, seen }
        }

        fn client(&self) -> Client {
            let s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            Client { r: BufReader::new(s.try_clone().unwrap()), w: s }
        }

        /// Methods that reached the UI thread, after letting any late arrivals land.
        fn forwarded(&self) -> Vec<String> {
            std::thread::sleep(Duration::from_millis(150));
            self.seen.lock().unwrap().clone()
        }
    }

    struct Client {
        w: TcpStream,
        r: BufReader<TcpStream>,
    }

    impl Client {
        /// Read one reply line. `None` means the server closed the connection or stayed silent.
        fn read(&mut self) -> Option<Value> {
            let mut buf = String::new();
            match self.r.read_line(&mut buf) {
                Ok(n) if n > 0 => serde_json::from_str(&buf).ok(),
                _ => None,
            }
        }

        fn call(&mut self, method: &str, params: Value) -> Option<Value> {
            writeln!(self.w, "{}", json!({"id": 1, "method": method, "params": params})).ok()?;
            self.read()
        }

        fn auth(&mut self, token: &str) -> Option<Value> {
            self.call("auth", json!({"token": token}))
        }
    }

    fn ok(v: &Option<Value>) -> bool {
        v.as_ref().and_then(|v| v.get("ok")).and_then(Value::as_bool).unwrap_or(false)
    }

    const TOKEN: &str = "5f0c3a9e4b7d21a86c0e93b5d7f41a2c8e6b0d3957a1c4f2e8b6d0a39c5e7f14";

    /// Guard: developer mode (no token configured) must keep working for `soundcraft-cli app`,
    /// `mcp --connect` and the agent workflow in AGENTS.md. Runs today and must keep passing.
    #[test]
    fn developer_mode_without_a_token_still_forwards_json_lines() {
        let h = Harness::launch(None);
        let mut c = h.client();
        assert!(ok(&c.call("session.inspect", json!({}))));
        assert_eq!(h.forwarded(), vec!["session.inspect"]);
    }

    #[test]
    fn without_a_token_nothing_reaches_the_ui_thread() {
        let h = Harness::launch(Some(TOKEN));
        let mut c = h.client();
        for m in ["session.inspect", "engine.execute", "ui.click", "app.quit"] {
            let r = c.call(m, json!({"command": "track.delete"}));
            assert!(!ok(&r), "{m} must be refused before authentication, got {r:?}");
        }
        assert!(h.forwarded().is_empty(), "forwarded: {:?}", h.forwarded());
    }

    #[test]
    fn a_wrong_token_is_refused_and_three_failures_close_the_connection() {
        let h = Harness::launch(Some(TOKEN));
        let mut c = h.client();
        for _ in 0..3 {
            assert!(!ok(&c.auth("not-the-token")));
        }
        assert!(c.auth(TOKEN).is_none(), "the connection must be closed after three failures, even for the right token");
        assert!(h.forwarded().is_empty());
    }

    #[test]
    fn a_token_from_a_previous_launch_is_refused() {
        let old = "a".repeat(64);
        let new = "b".repeat(64);
        let _previous = Harness::launch(Some(&old));
        let h = Harness::launch(Some(&new));
        let mut c = h.client();
        assert!(!ok(&c.auth(&old)));
        assert!(!ok(&c.call("session.inspect", json!({}))));
        assert!(h.forwarded().is_empty());
    }

    #[test]
    fn the_right_token_unlocks_the_assistant_allowlist_and_nothing_else() {
        let h = Harness::launch(Some(TOKEN));
        let mut c = h.client();
        let r = c.auth(TOKEN);
        assert!(ok(&r), "{r:?}");
        assert!(!r.unwrap().to_string().contains(TOKEN), "the reply must not echo the token");
        // Positive control: allowed methods get through, so the refusals below are not vacuous.
        for m in ["session.inspect", "session.execution_status"] {
            assert!(ok(&c.call(m, json!({}))), "{m} should be allowed");
        }
        // Everything else is refused at the gate, including what developer mode allows.
        for m in [
            "engine.execute",
            "command",
            "engine.commands",
            "ui.click",
            "ui.key",
            "ui.text",
            "ui.menu.invoke",
            "ui.set",
            "ui.screenshot",
            "document.inspect",
            "file.bounce_mix",
            "app.quit",
            "no.such.method",
        ] {
            let r = c.call(m, json!({}));
            assert!(!ok(&r), "{m} must not be reachable with an assistant token, got {r:?}");
        }
        assert_eq!(h.forwarded(), vec!["session.inspect", "session.execution_status"]);
    }

    /// A web page can make the browser send `POST / HTTP/1.1` plus a body to `127.0.0.1:<port>`.
    /// The body is a line of valid JSON. Today's server answers the HTTP header lines with
    /// "bad JSON" and then runs the body line, so any page the user visits can drive the app.
    #[test]
    fn an_http_request_carrying_a_json_body_is_not_executed() {
        for token in [Some(TOKEN), None] {
            let h = Harness::launch(token);
            let mut c = h.client();
            let body = json!({"id": 1, "method": "engine.execute", "params": {"command": "track.delete"}}).to_string();
            let request = format!(
                "POST / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}\n",
                h.port,
                body.len()
            );
            let _ = c.w.write_all(request.as_bytes());
            let _ = c.w.flush();
            // Drain whatever the server says; the point is what reached the UI thread.
            while c.read().is_some() {}
            assert!(h.forwarded().is_empty(), "token={token:?}: an HTTP-framed body was executed: {:?}", h.forwarded());
        }
    }

    #[test]
    fn an_oversized_line_closes_the_connection() {
        let h = Harness::launch(Some(TOKEN));
        let mut c = h.client();
        let big = vec![b'x'; 2 * 1024 * 1024];
        let _ = c.w.write_all(&big);
        let _ = c.w.write_all(b"\n");
        assert!(c.read().is_none() || c.auth(TOKEN).is_none(), "an unauthenticated 2 MiB line must end the connection");
        assert!(h.forwarded().is_empty());
    }

    #[test]
    fn a_silent_unauthenticated_connection_is_closed() {
        let h = Harness::launch(Some(TOKEN));
        let mut c = h.client();
        let started = Instant::now();
        // The server should hang up on its own once `auth_timeout` (300 ms here) passes.
        assert!(c.read().is_none());
        assert!(started.elapsed() < Duration::from_millis(1500), "took {:?}", started.elapsed());
    }
}
