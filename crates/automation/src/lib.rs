//! Agent control for SoundCraft.
//!
//! - [`Backend`]: something that answers control-protocol methods (`engine.execute`,
//!   `session.inspect`, `ui.*`…). [`Headless`] runs an in-process engine; [`Remote`] talks to a
//!   running app's `--control` port.
//! - [`mcp::Server`]: a Model Context Protocol server over stdio (newline-delimited JSON-RPC 2.0)
//!   exposing SoundCraft as tools and resources.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod mcp;

use serde_json::{Value, json};
use soundcraft_engine::Engine;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Answers control-protocol methods.
pub trait Backend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// True when a UI is attached (screenshots, clicks).
    fn has_ui(&self) -> bool;
    fn describe(&self) -> String;
}

/// An in-process engine (no UI).
pub struct Headless {
    pub engine: Engine,
}

impl Headless {
    pub fn new(engine: Engine) -> Self {
        Headless { engine }
    }
}

impl Backend for Headless {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "engine.execute" | "command" => {
                let id = params.get("command").or_else(|| params.get("id")).and_then(Value::as_str).ok_or("`command` required")?;
                let p = params.get("params").cloned().unwrap_or(json!({}));
                let r = self.engine.execute(id, &p).map_err(|e| e.to_string());
                // Headless has no audio engine: drop transport requests but keep the playhead.
                self.engine.transport_requests.clear();
                r
            }
            "engine.commands" => self.engine.execute("engine.commands", &params).map_err(|e| e.to_string()),
            "engine.parity" => Ok(soundcraft_engine::catalog::parity_json()),
            "session.inspect" | "document.inspect" => {
                Ok(soundcraft_engine::inspect::session(&self.engine, params.get("detail").and_then(Value::as_str) == Some("full")))
            }
            m if m.starts_with("ui.") => {
                Err(format!("`{m}` needs the desktop app: start `soundcraft --control 0` and use `soundcraft-cli mcp --connect PORT`"))
            }
            other => Err(format!("unknown method `{other}`")),
        }
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        format!("headless engine, session `{}`", self.engine.session().name)
    }
}

/// A TCP client for a running app's control channel.
pub struct Remote {
    addr: String,
    /// Sent as the first message of every connection when the app runs in assistant mode.
    token: Option<String>,
    conn: Option<(TcpStream, BufReader<TcpStream>)>,
    next_id: u64,
}

impl Remote {
    /// `target` is a port (`7979`) or `host:port`.
    pub fn new(target: &str) -> Self {
        let addr = if target.contains(':') { target.to_string() } else { format!("127.0.0.1:{target}") };
        Remote { addr, token: None, conn: None, next_id: 1 }
    }

    /// True when the server has already closed the idle connection (a peek reads end of file), so
    /// a request is never sent into a dead socket.
    fn is_stale(&self) -> bool {
        let Some((s, _)) = &self.conn else { return false };
        if s.set_nonblocking(true).is_err() {
            return true;
        }
        let mut b = [0u8; 1];
        let stale = match s.peek(&mut b) {
            Ok(n) => n == 0,
            Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
        };
        stale || s.set_nonblocking(false).is_err()
    }

    /// Authenticate with `token` (read from the app's `--control-token-file`) on every connection.
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    fn connect(&mut self) -> Result<(), String> {
        if self.is_stale() {
            self.conn = None;
        }
        if self.conn.is_some() {
            return Ok(());
        }
        let sock = self.addr.parse().map_err(|e| format!("{}: {e}", self.addr))?;
        let s =
            TcpStream::connect_timeout(&sock, Duration::from_millis(800)).map_err(|e| format!("cannot reach SoundCraft at {}: {e}", self.addr))?;
        s.set_read_timeout(Some(Duration::from_secs(120))).map_err(|e| e.to_string())?;
        let r = s.try_clone().map_err(|e| e.to_string())?;
        self.conn = Some((s, BufReader::new(r)));
        if let Some(token) = self.token.clone() {
            // The reply carries no secret; a refusal ends the attempt with the server's reason.
            if let Err(f) = self.send("auth", &json!({"token": token})) {
                self.conn = None;
                return Err(match f {
                    Failure::Rejected(e) | Failure::NotSent(e) | Failure::Unknown(e) => format!("authentication with the app failed: {e}"),
                });
            }
        }
        Ok(())
    }

    /// One request/reply. `Err(Failure::NotSent)` means the request provably never left, so it
    /// is safe to send again; `Err(Failure::Unknown)` means the server may have executed it.
    fn roundtrip(&mut self, method: &str, params: &Value) -> Result<Value, Failure> {
        self.connect().map_err(Failure::NotSent)?;
        self.send(method, params)
    }

    /// Write one request on the open connection and read its reply.
    fn send(&mut self, method: &str, params: &Value) -> Result<Value, Failure> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        let (w, r) = self.conn.as_mut().ok_or_else(|| Failure::NotSent("not connected".into()))?;
        writeln!(w, "{line}").map_err(|e| Failure::NotSent(e.to_string()))?;
        let mut buf = String::new();
        r.read_line(&mut buf).map_err(|e| Failure::Unknown(e.to_string()))?;
        if buf.is_empty() {
            return Err(Failure::Unknown("connection closed".into()));
        }
        let v: Value = serde_json::from_str(&buf).map_err(|e| Failure::Unknown(e.to_string()))?;
        if v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(Failure::Rejected(v.get("error").and_then(Value::as_str).unwrap_or("error").to_string()))
        }
    }
}

enum Failure {
    /// The server answered with an error: nothing to retry.
    Rejected(String),
    /// The request never reached the server.
    NotSent(String),
    /// The request was sent but no reply arrived: the server may or may not have applied it.
    Unknown(String),
}

/// Methods that only read state: sending one twice is harmless.
fn is_read_only(method: &str) -> bool {
    matches!(method, "engine.commands" | "engine.parity" | "session.inspect" | "document.inspect" | "ui.inspect" | "ui.menu.list")
}

impl Remote {
    fn retry(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        self.roundtrip(method, params).map_err(|f| match f {
            Failure::Rejected(e) | Failure::NotSent(e) | Failure::Unknown(e) => e,
        })
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let first = match self.roundtrip(method, &params) {
            Ok(v) => return Ok(v),
            Err(Failure::Rejected(e)) => return Err(e),
            Err(f) => f,
        };
        self.conn = None;
        match first {
            // Never delivered, or a read: a fresh connection and one more attempt is safe.
            Failure::NotSent(_) => self.retry(method, &params),
            Failure::Unknown(_) if is_read_only(method) => self.retry(method, &params),
            Failure::Unknown(e) => Err(format!(
                "connection lost after sending `{method}` ({e}); the app may or may not have applied it. Inspect the session before repeating it"
            )),
            Failure::Rejected(e) => Err(e),
        }
    }
    fn has_ui(&self) -> bool {
        true
    }
    fn describe(&self) -> String {
        format!("SoundCraft app at {}", self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_executes_and_inspects() {
        let mut h = Headless::new(Engine::default());
        let r = h.call("engine.execute", json!({"command": "track.new", "params": {"count": 2, "format": "stereo"}}));
        assert!(r.is_ok(), "{r:?}");
        let s = h.call("session.inspect", json!({})).unwrap();
        assert_eq!(s["tracks"].as_array().map(Vec::len), Some(2));
        assert!(h.call("ui.click", json!({})).is_err());
        assert!(h.call("nope", json!({})).is_err());
    }

    #[test]
    fn remote_reports_unreachable() {
        let mut r = Remote::new("127.0.0.1:1");
        assert!(r.call("session.inspect", json!({})).is_err());
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A server that executes the first request on each connection and drops the first connection
    /// before replying.
    fn flaky() -> (String, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let applied = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&applied);
        let h = std::thread::spawn(move || {
            for attempt in 0..2 {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut text = String::new();
                if BufReader::new(stream.try_clone().unwrap()).read_line(&mut text).is_err() || text.is_empty() {
                    continue;
                }
                counter.fetch_add(1, Ordering::SeqCst);
                if attempt == 1 {
                    let msg: Value = serde_json::from_str(&text).unwrap();
                    let _ = writeln!(stream, "{}", json!({"id": msg["id"], "ok": true, "result": {}}));
                }
            }
        });
        (addr, applied, h)
    }

    #[test]
    fn a_mutation_whose_reply_is_lost_is_never_sent_twice() {
        let (addr, applied, h) = flaky();
        let mut c = Remote::new(&addr);
        let e = c.call("engine.execute", json!({"command": "track.new", "params": {"count": 1}})).unwrap_err();
        assert!(e.contains("may or may not have applied"), "{e}");
        assert_eq!(applied.load(Ordering::SeqCst), 1);
        drop(c);
        let _ = std::net::TcpStream::connect(&addr); // release the accept loop
        let _ = h.join();
    }

    #[test]
    fn a_read_is_retried_after_a_lost_reply() {
        let (addr, applied, h) = flaky();
        let mut c = Remote::new(&addr);
        assert!(c.call("session.inspect", json!({})).is_ok());
        assert_eq!(applied.load(Ordering::SeqCst), 2);
        let _ = h.join();
    }

    #[test]
    fn a_token_is_sent_first_on_every_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let h = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut r = BufReader::new(stream.try_clone().unwrap());
            let mut methods = Vec::new();
            for _ in 0..2 {
                let mut text = String::new();
                r.read_line(&mut text).unwrap();
                let msg: Value = serde_json::from_str(&text).unwrap();
                methods.push((msg["method"].as_str().unwrap_or("").to_string(), msg["params"]["token"].as_str().map(str::to_string)));
                writeln!(stream, "{}", json!({"id": msg["id"], "ok": true, "result": {}})).unwrap();
            }
            methods
        });
        let mut c = Remote::new(&addr).with_token("t0ken");
        assert!(c.call("session.inspect", json!({})).is_ok());
        let seen = h.join().unwrap();
        assert_eq!(seen[0], ("auth".to_string(), Some("t0ken".to_string())));
        assert_eq!(seen[1].0, "session.inspect");
    }
}
