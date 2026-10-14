//! Loopback JSON-lines control server: one request per line, one reply per line.
//! This is the transport the MCP server (`soundcraft-cli mcp --connect PORT`) wraps.
//!
//! Two modes (design: `docs/soundcraft-ai/control-channel-protection.md`):
//! - developer mode (no token): every control method is forwarded;
//! - assistant mode (a per-launch token): the first message must be `auth`, and only the assistant
//!   allowlist is forwarded.
//!
//! In both modes a connection whose first line is not a JSON object (an HTTP request from a web
//! page, say) is closed unanswered, reads are bounded, and the number of connections is capped.

use serde_json::{Value, json};
use soundcraft_ui_egui::ControlRequest;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

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
const MAX_CONNECTIONS: usize = 8;
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Longest line accepted before authentication, after it, and in developer mode.
const MAX_LINE_PRE_AUTH: usize = 4 * 1024;
const MAX_LINE_AUTHED: usize = 1024 * 1024;
const MAX_LINE_DEV: usize = 8 * 1024 * 1024;
const MAX_AUTH_FAILURES: usize = 3;

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
    let open = Arc::new(AtomicUsize::new(0));
    let spawned = std::thread::Builder::new().name("soundcraft-control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            if open.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                open.fetch_sub(1, Ordering::AcqRel);
                log::warn!("control connection refused: {MAX_CONNECTIONS} already open");
                continue;
            }
            let (tx, ctx, cfg, open) = (tx.clone(), ctx.clone(), Arc::clone(&cfg), Arc::clone(&open));
            let started = std::thread::Builder::new().name("soundcraft-control-conn".into()).spawn({
                let open = Arc::clone(&open);
                move || {
                    serve(stream, tx, ctx, &cfg);
                    open.fetch_sub(1, Ordering::AcqRel);
                }
            });
            if started.is_err() {
                open.fetch_sub(1, Ordering::AcqRel);
            }
        }
    });
    if let Err(e) = spawned {
        log::error!("control thread failed: {e}");
    }
    rx
}

/// Compares in time independent of where the inputs first differ.
fn same_token(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// One line of at most `max` bytes. `Ok(None)` at end of input; an error when the line is longer.
fn read_line_bounded(r: &mut BufReader<TcpStream>, max: usize) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(max as u64 + 1).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > max && buf.last() != Some(&b'\n') {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "line too long"));
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context, cfg: &ControlConfig) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    let mut input = BufReader::new(read);
    let mut authed = cfg.token.is_none();
    let (mut first, mut failures) = (true, 0usize);
    loop {
        let (timeout, max) = match (&cfg.token, authed) {
            (Some(_), false) => (Some(cfg.auth_timeout), MAX_LINE_PRE_AUTH),
            (Some(_), true) => (Some(IDLE_TIMEOUT), MAX_LINE_AUTHED),
            (None, _) => (None, MAX_LINE_DEV),
        };
        if out.set_read_timeout(timeout).is_err() {
            break;
        }
        let Ok(Some(line)) = read_line_bounded(&mut input, max) else { break };
        if line.trim().is_empty() {
            continue;
        }
        // Only a JSON object may open a conversation: anything else (an HTTP request from a web
        // page, a WebSocket upgrade) is closed without an answer or any forwarding.
        if std::mem::take(&mut first) && !line.trim_start().starts_with('{') {
            log::warn!("control connection closed: first line is not a JSON object");
            break;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
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
