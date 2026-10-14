//! Loopback JSON-lines control server: one request per line, one reply per line.
//! This is the transport the MCP server (`soundcraft-cli mcp --connect PORT`) wraps.

use serde_json::{Value, json};
use soundcraft_ui_egui::ControlRequest;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

pub fn start(port: u16, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control server failed to bind 127.0.0.1:{port}: {e}");
            return rx;
        }
    };
    let actual = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    log::info!("control server listening on 127.0.0.1:{actual}");
    println!("SOUNDCRAFT_CONTROL_PORT={actual}");
    let spawned = std::thread::Builder::new().name("soundcraft-control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            let ctx = ctx.clone();
            let _ = std::thread::Builder::new().name("soundcraft-control-conn".into()).spawn(move || serve(stream, tx, ctx));
        }
    });
    if let Err(e) = spawned {
        log::error!("control thread failed: {e}");
    }
    rx
}

fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    for line in BufReader::new(read).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let (req, rrx) = ControlRequest::new(method, params);
                if tx.send(req).is_err() {
                    break;
                }
                ctx.request_repaint();
                let mut r = rrx.recv_timeout(Duration::from_secs(90)).unwrap_or_else(|_| json!({"ok": false, "error": "timeout"}));
                if let Some(o) = r.as_object_mut() {
                    o.insert("id".into(), id);
                }
                r
            }
            Err(e) => json!({"ok": false, "error": format!("bad JSON: {e}")}),
        };
        if writeln!(out, "{reply}").is_err() {
            break;
        }
    }
}

/// Tests for the protected control channel (plan task P0-T07; design in
/// `docs/soundcraft-ai/control-channel-protection.md`).
///
/// The protection is built in P1-E03. Until then `start_with` below is a stand-in that ignores the
/// token, so the `#[ignore]` tests fail when run with `--ignored`, which is what they are for.
/// When P1-E03 lands, move `ControlConfig` and `start_with` out of this module, delete the
/// stand-in, and remove the `#[ignore]` attributes.
#[cfg(test)]
mod protection_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    const PENDING: &str = "P1-E03: control channel protection is not built yet";

    /// The configuration P1-E03 will accept. The stand-in server ignores two of the fields.
    #[allow(dead_code)]
    struct ControlConfig {
        port: u16,
        /// `Some` puts the server in assistant mode: authentication is required and only the
        /// assistant allowlist is forwarded. `None` is today's developer mode.
        token: Option<String>,
        /// How long a connection may stay silent before it has authenticated.
        auth_timeout: Duration,
    }

    /// STAND-IN: today's server, which ignores `token` and `auth_timeout`.
    fn start_with(cfg: ControlConfig, ctx: egui::Context) -> Receiver<ControlRequest> {
        start(cfg.port, ctx)
    }

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
    #[ignore = "P1-E03: control channel protection is not built yet"]
    fn without_a_token_nothing_reaches_the_ui_thread() {
        let _ = PENDING;
        let h = Harness::launch(Some(TOKEN));
        let mut c = h.client();
        for m in ["session.inspect", "engine.execute", "ui.click", "app.quit"] {
            let r = c.call(m, json!({"command": "track.delete"}));
            assert!(!ok(&r), "{m} must be refused before authentication, got {r:?}");
        }
        assert!(h.forwarded().is_empty(), "forwarded: {:?}", h.forwarded());
    }

    #[test]
    #[ignore = "P1-E03: control channel protection is not built yet"]
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
    #[ignore = "P1-E03: control channel protection is not built yet"]
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
    #[ignore = "P1-E03: control channel protection is not built yet"]
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
    #[ignore = "P1-E03: control channel protection is not built yet"]
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
    #[ignore = "P1-E03: control channel protection is not built yet"]
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
    #[ignore = "P1-E03: control channel protection is not built yet"]
    fn a_silent_unauthenticated_connection_is_closed() {
        let h = Harness::launch(Some(TOKEN));
        let mut c = h.client();
        let started = Instant::now();
        // The server should hang up on its own once `auth_timeout` (300 ms here) passes.
        assert!(c.read().is_none());
        assert!(started.elapsed() < Duration::from_millis(1500), "took {:?}", started.elapsed());
    }
}
