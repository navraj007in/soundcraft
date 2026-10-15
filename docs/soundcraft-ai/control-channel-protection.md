# Control channel protection: design and tests

**Task:** P0-T07 in the [execution plan](execution-plan.md) · **Implements later in:** P1-E03 · **Feeds:** PRD §10.2, G12 (unauthorised IPC)
**Version:** 0.2 · **Updated:** 15 October 2026 · **Status:** §2.2–§2.5 implemented in the server, `--control-token-file` and the client `--token-file`; companion-side token generation (§6 item 4) and the real allowlist methods (§6 item 6) are still open.
**Code:** branch `spike/control-auth`, `apps/soundcraft/src/control_server.rs` (`protection_tests`).

## 1. Why this is needed, measured

The control channel is a loopback TCP server that reads one line at a time and runs any line that
parses as JSON (`apps/soundcraft/src/control_server.rs:36-66`). It checks no credential, and the
methods it forwards include `engine.execute` (any command, including file writes and deletes),
`ui.click`, `ui.key`, `ui.text` and `app.quit` (`crates/ui-egui/src/control.rs:5-13`).

Two ways it can be reached by something the user didn't intend:

1. **Any local process**, including other users' processes, if the port is known or scanned.
2. **Any web page the user has open.** A browser will send `POST / HTTP/1.1` with a plain-text body
   to `127.0.0.1:<port>`. The server answers each HTTP header line with "bad JSON" and then reads
   the body line, which is valid JSON, and runs it. **Measured:** test
   `an_http_request_carrying_a_json_body_is_not_executed`, run against the server before this change, ended with
   `an HTTP-framed body was executed: ["engine.execute"]`, even with a token configured (the stand-in
   ignored it). The port is random with `--control 0`, but a page can probe ports, and the port is
   printed to stdout and logs.

PRD §10.2 requires an authenticated endpoint scoped to the launched session before the assistant
mutation path is exposed. This is also a defect for developer use, so one part of the fix applies
to both modes (§2.5).

## 2. Design

### 2.1 Two modes

| | Developer mode (today) | Assistant mode (new) |
|---|---|---|
| Started by | `--control <port>` alone | `--control <port> --control-token-file <path>` |
| Authentication | None | Required: first message on every connection |
| Methods forwarded | All current methods | Only the assistant allowlist (§2.4) |
| Used by | `soundcraft-cli app`, `mcp --connect`, agents driving the app (`AGENTS.md`) | The companion process |

Developer mode keeps working unchanged apart from §2.5, so existing workflows and docs don't break.
The assistant path never runs in developer mode: the companion refuses to talk to an app that did
not accept its token.

### 2.2 Token

- **What:** 32 random bytes from the operating system's secure random source, as 64 hex characters.
  One token per launch of the app. A new launch means a new token, so a token from a previous launch
  is useless.
- **Who makes it:** the companion, which launches the app. The app never generates or writes it.
- **How it reaches the app:** a file, never the command line or the environment (both are visible to
  other processes through `ps`). The companion creates a private folder (mode 0700) and the file
  inside it (mode 0600), and passes only the file's path with `--control-token-file`. The app reads
  it once at startup and **refuses to start the assistant channel** if the file is group- or
  world-accessible, empty, shorter than 64 characters or unreadable. No unsafe code is needed (mode
  from `std::fs::Metadata`).
- **Lifetime:** valid only for that app process. The companion deletes the file when the app exits;
  a stale file from a crashed run is removed at the next launch. If the companion restarts while the
  app keeps running it re-reads its own file; there is no rotation in the first release.
- **Comparison:** constant time (fold over the bytes). The reference used in validation is 3 lines.
- **Never logged**, never echoed in a reply, never in the journal or history (PRD §11.1).

### 2.3 Handshake and connection rules

1. The client's first message must be `{"id":N,"method":"auth","params":{"token":"<64 hex>"}}`.
   Success replies `{"id":N,"ok":true,"result":{"scope":"assistant"}}`. The token is not echoed.
2. Until then every other method gets `{"ok":false,"error":"unauthenticated"}` and **is not
   forwarded**.
3. Three failed `auth` attempts close the connection.
4. A connection that sends nothing for `auth_timeout` (10 s in production, 300 ms in tests) is
   closed.
5. A line over 4 KiB before authentication, or over 1 MiB after, closes the connection. Reads are
   bounded; the current `lines()` reader is not.
6. At most 16 open connections; further ones are refused.
7. After authentication an idle connection closes after 10 minutes.

### 2.4 Allowlist (assistant scope)

Enforced **in the server thread, before a request reaches the UI thread**:

| Method | Purpose |
|---|---|
| `auth` | Handshake (handled in the server, never forwarded) |
| `session.inspect` | Read session, instance ID and content revision (P1-E01) |
| `session.apply` | Guarded group publish (P1-E02) |
| `session.execution_status` | Outcome query for reconciliation (P1-C07) |
| Render and analysis requests | Added when P1-U02 and P1-A01 define them |

Not reachable with an assistant token, by test: `engine.execute`, `command`, `engine.commands`,
`ui.*` (click, key, text, menu, set, screenshot), `document.inspect` (an alias; the assistant uses
`session.inspect`), `file.*`, `app.quit`, and unknown methods. `session.apply` is validated again at
the engine boundary against the capability manifest (P1-E04) and the scope diff (P1-E06), so the
gate is one layer, not the only one.

### 2.5 Hardening that applies to both modes

If the **first** line on a connection looks like HTTP (starts with a method such as `GET `, `POST `
or `OPTIONS `, or contains ` HTTP/`), close the connection without forwarding anything. A raw
JSON-lines client never sends that, so nothing legitimate breaks, and it closes the browser route in
developer mode too. This is the one behaviour change for developer mode.

### 2.6 Logging

Log method name, connection ID and outcome (accepted, refused, closed and why). Never log params of
`auth`. Params of other methods follow the existing logging rules and exclude audio and credentials
(PRD §11.1).

## 3. Alternatives considered

| Option | Why not (for now) |
|---|---|
| **Unix domain socket**, mode 0600 in a private folder | Strongest fit on macOS: the filesystem is the credential, and browsers can't open it. But `Remote` and `mcp --connect` take a TCP port, the web build and other platforms need TCP anyway, and the app would carry two transports. Revisit after Phase 1; the allowlist and limits carry over unchanged. |
| Per-request signature (HMAC) | Adds replay and key-handling work for no benefit over a per-launch token on a loopback connection that is already authenticated once. |
| TLS with client certificates | Heavy dependencies and certificate handling for a same-machine channel. |
| Rely on the random port | Not a credential: it is printed to stdout, logged and discoverable (§1). |

## 4. What this does not protect against

- **Another process running as the same user** that can read the token file (or ptrace the app) is
  inside the trust boundary. This defence is against other users, other sandboxes, browsers and
  stray local clients.
- **A compromised companion** holds a valid token. Defence there is the allowlist, the manifest and
  the scope diff, and keeping the model's tool set separate from the companion's own authority
  (PRD §10.2).
- **Windows and Linux** token-file permissions: the first release is macOS only. Other platforms
  need their own permission check before this mode is enabled there.

## 5. Tests

`protection_tests` in `apps/soundcraft/src/control_server.rs` drives a real server over TCP, with a
stand-in for the UI thread that records every method it is asked to run (`seen`), so each test
asserts exactly what got past the gate. The harness starts the production `start_with`.

| Test | Design rule it pins | Status |
|---|---|---|
| `developer_mode_without_a_token_still_forwards_json_lines` | §2.1: developer mode keeps working | passes |
| `without_a_token_nothing_reaches_the_ui_thread` | §2.3 rule 2 | passes |
| `a_wrong_token_is_refused_and_three_failures_close_the_connection` | §2.3 rule 3 | passes |
| `a_token_from_a_previous_launch_is_refused` | §2.2 per-launch token | passes |
| `the_right_token_unlocks_the_assistant_allowlist_and_nothing_else` | §2.3 rule 1, §2.4 (13 refused methods, 2 allowed, no token echo) | passes |
| `an_http_request_carrying_a_json_body_is_not_executed` | §2.5 (runs with and without a token) | passes |
| `an_oversized_line_closes_the_connection` | §2.3 rule 5 | passes |
| `a_silent_unauthenticated_connection_is_closed` | §2.3 rule 4 | passes |

All eight tests now run in `cargo test -p soundcraft --bin soundcraft protection_tests`; the earlier `#[ignore]` attributes were removed when the server gained the protection.

**Validation that they can pass, and that they notice regressions.** A throwaway reference
implementation of §2.2–§2.5 in the test module (about 90 lines: bounded reads, first-line HTTP check,
handshake, constant-time compare, allowlist, failure counter, timeout) made all 8 tests pass. Then
each defence was removed in turn; every removal made at least one test fail:

| Defence removed | Tests that failed |
|---|---|
| HTTP first-line check | `an_http_request_carrying_a_json_body_is_not_executed` |
| Close after three failures | `a_wrong_token_is_refused_and_three_failures…` |
| Allowlist | `the_right_token_unlocks_the_assistant_allowlist…` |
| Line-length cap | `an_oversized_line_closes_the_connection` |
| Authentication timeout | `a_silent_unauthenticated_connection_is_closed` |
| Token comparison (always true) | `a_wrong_token…`, `a_token_from_a_previous_launch…` |
| "Must authenticate first" check | `without_a_token_nothing_reaches…`, `a_token_from_a_previous_launch…` |

The reference was discarded; the production version in `control_server.rs` follows the same algorithm.

## 6. P1-E03 work breakdown

1. ~~Move `ControlConfig` and `start_with` out of the test module; implement §2.2–§2.6 in the server; remove the `#[ignore]` attributes.~~ Done.
2. ~~`--control-token-file` in `apps/soundcraft/src/main.rs`; startup checks for the file.~~ Done.
3. ~~`Remote` and `soundcraft-cli app` / `mcp --connect`: optional `--token-file`, sending `auth` first.~~ Done.
4. Companion: token generation, private folder and file modes, cleanup of stale files.
5. ~~Update `docs/control-protocol.md` (modes, handshake, allowlist, the HTTP behaviour change).~~ Done.
6. Add the real methods to the allowlist as P1-E01, E02 and C07 land.

**Estimate:** roughly 8–12 h (a guess; the gate itself is the small part, the plumbing and the
companion side are most of it). Generic, so a candidate for an upstream PR as its own commit.

## 7. Open items

- **Port disclosure:** the app prints `SOUNDCRAFT_CONTROL_PORT=<n>` to stdout. That is fine for a
  companion reading its child's pipe, but in assistant mode consider not logging the port at info
  level.
- **Token file location and cleanup** on macOS (`~/Library/Application Support/…` versus a
  per-user temporary folder) is decided with the companion's packaging (P0-T10).
- **Rotation** (a new token for a long-running session) is deferred; the first release uses one
  token per app launch.
- **Audit trail** of refused requests for the user (not just a log): decide with the task-history
  design (P1-I05).

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | Design and tests (P0-T07). |
| 0.2 | 15 October 2026 | Server protection, token file and client token option implemented; tests no longer ignored. |
