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
use soundcraft_engine::file_access::{self, AutomationRoots};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

/// Longest line either side of the control protocol or MCP will read (bytes): room for the largest
/// plugin state a command carries (about 85 MiB as base64) plus the JSON around it.
pub const MAX_LINE: usize = 96 << 20;

/// Read one `\n`-terminated line of at most `max` bytes (the newline and a `\r` stripped).
/// `Ok(None)` at end of input; an `InvalidData` error when the line is longer, without reading the
/// rest of it, so a peer cannot make this buffer more than `max + 1` bytes.
pub fn read_line_bounded(reader: &mut impl BufRead, max: usize) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    if reader.take(limit).read_until(b'\n', &mut buf)? == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    } else if buf.len() > max {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("line longer than {max} bytes")));
    }
    String::from_utf8(buf).map(Some).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

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
    roots: Option<Arc<AutomationRoots>>,
}

impl Headless {
    pub fn new(engine: Engine) -> Self {
        Headless { engine, roots: None }
    }

    /// Confine every file the session's commands read and write to `roots`
    /// (`soundcraft-cli mcp --automation-read-root … --automation-write-root …`).
    pub fn with_automation_roots(mut self, roots: Option<Arc<AutomationRoots>>) -> Self {
        self.roots = roots;
        self
    }

    fn answer(&mut self, method: &str, params: Value) -> Result<Value, String> {
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
}

impl Backend for Headless {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let roots = self.roots.clone();
        file_access::confine(roots.as_ref(), || self.answer(method, params))
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
            if let Err((e, _)) = self.exchange("auth", &json!({"token": token})) {
                self.conn = None;
                return Err(format!("authentication with the app failed: {e}"));
            }
        }
        Ok(())
    }

    /// One request/reply exchange. On failure the flag says whether the request line was already
    /// written to the socket (so the app may have executed it).
    fn roundtrip(&mut self, method: &str, params: &Value) -> Result<Value, (String, bool)> {
        self.connect().map_err(|e| (e, false))?;
        self.exchange(method, params)
    }

    /// Write one request on the open connection and read its reply (no connecting).
    fn exchange(&mut self, method: &str, params: &Value) -> Result<Value, (String, bool)> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        let (w, r) = self.conn.as_mut().ok_or_else(|| ("not connected".to_string(), false))?;
        writeln!(w, "{line}").map_err(|e| (e.to_string(), false))?;
        let buf = read_line_bounded(r, MAX_LINE).map_err(|e| (e.to_string(), true))?.ok_or_else(|| ("connection closed".to_string(), true))?;
        let v: Value = serde_json::from_str(&buf).map_err(|e| (e.to_string(), true))?;
        if v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err((v.get("error").and_then(Value::as_str).unwrap_or("error").to_string(), true))
        }
    }
}

/// Methods that only read state, so repeating them after a lost reply is harmless.
fn is_read_only(method: &str) -> bool {
    matches!(method, "session.inspect" | "document.inspect" | "engine.commands" | "engine.parity" | "ui.inspect" | "ui.menu.list")
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match self.roundtrip(method, &params) {
            Ok(v) => Ok(v),
            Err((e, sent)) if e.contains("closed") || e.contains("Broken pipe") || e.contains("reset") => {
                self.conn = None;
                if sent && !is_read_only(method) {
                    // The app may already have executed the request; repeating it could apply it twice.
                    return Err(format!("{e}; `{method}` may or may not have been executed, check the session before repeating it"));
                }
                // Retry once on a fresh connection.
                self.roundtrip(method, &params).map_err(|(e, _)| e)
            }
            Err((e, _)) => Err(e),
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

    /// Fake app: accepts one connection per entry in `replies`, counts the request line it reads,
    /// then either drops the connection (`false`) or answers ok (`true`).
    fn fake_app(replies: Vec<bool>) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = seen.clone();
        std::thread::spawn(move || {
            for answer in replies {
                let Ok((mut sock, _)) = listener.accept() else { return };
                let mut line = String::new();
                if BufReader::new(sock.try_clone().unwrap()).read_line(&mut line).unwrap_or(0) > 0 {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if answer {
                        let _ = writeln!(sock, "{}", json!({"id": 1, "ok": true, "result": {"done": true}}));
                    }
                }
            }
        });
        (addr, seen)
    }

    #[test]
    fn remote_does_not_repeat_a_mutation_after_a_lost_reply() {
        let (addr, seen) = fake_app(vec![false, true]);
        let mut r = Remote::new(&addr);
        let err = r.call("engine.execute", json!({"command": "track.new"})).unwrap_err();
        assert!(err.contains("may or may not have been executed"), "{err}");
        assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn remote_retries_a_read_after_a_lost_reply() {
        let (addr, seen) = fake_app(vec![false, true]);
        let mut r = Remote::new(&addr);
        assert!(r.call("session.inspect", json!({})).is_ok());
        assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 2);
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
        assert!(e.contains("may or may not have been executed"), "{e}");
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
