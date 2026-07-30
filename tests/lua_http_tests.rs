//! `oslo.http`, against a server this test starts itself.
//!
//! No network and no TLS here: a listener on localhost speaking HTTP/1.1 by hand exercises every
//! part of the surface — method, headers, body, status, the result shape — without depending on
//! anything outside the machine. The TLS half is covered by the unit tests in
//! `src/lua/api/http/certs.rs`, which pin curl's certificate precedence.

mod common;

use common::oslo_bin;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;

/// What the server saw, so a test can assert on the request as well as the response.
struct Seen {
    request_line: String,
    headers: Vec<String>,
    body: String,
}

/// Serve exactly one request, answering with `status` and `body`.
///
/// Returns the port and a channel carrying what the client sent.
fn serve_once(status: &'static str, body: &'static str) -> (u16, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let mut reader = BufReader::new(&stream);

        let mut request_line = String::new();
        let _ = reader.read_line(&mut request_line);

        let mut headers = Vec::new();
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap_or(0);
            }
            headers.push(line.trim().to_string());
        }

        let mut request_body = vec![0u8; length];
        if length > 0 {
            use std::io::Read;
            let _ = reader.read_exact(&mut request_body);
        }

        let mut stream = &stream;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\
             X-Mixed-Case: yes\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();

        let _ = tx.send(Seen {
            request_line: request_line.trim().to_string(),
            headers,
            body: String::from_utf8_lossy(&request_body).into_owned(),
        });
    });

    (port, rx)
}

/// Run a Lua chunk with `PORT` set, and return its stdout.
#[track_caller]
fn lua(port: u16, script: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("case.lua");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(oslo_bin())
        .arg(&path)
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("PORT", port.to_string())
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

#[test]
fn get_returns_the_status_body_and_headers() {
    let (port, _seen) = serve_once("200 OK", "hello from the server");
    let out = lua(
        port,
        r#"
        local r = oslo.http.get("http://127.0.0.1:" .. PORT .. "/thing")
        print(r.status, r.ok, r.body)
        -- Header names are lower-cased, because HTTP's are case-insensitive and a script should
        -- not have to guess how the server spelled them.
        print(r.headers["content-type"], r.headers["x-mixed-case"])
    "#,
    );
    assert_eq!(out, "200\ttrue\thello from the server\ntext/plain\tyes");
}

/// A 404 is an answer, not a failure to get one — curl prints the body and exits 0 without
/// `--fail`, and a script needs to see both the status and the page.
#[test]
fn a_404_reports_its_status_and_still_carries_a_body() {
    let (port, _seen) = serve_once("404 Not Found", "no such page");
    let out = lua(
        port,
        r#"
        local r = oslo.http.get("http://127.0.0.1:" .. PORT .. "/missing")
        print(r.status, r.ok, r.body)
    "#,
    );
    assert_eq!(out, "404\tfalse\tno such page");
}

#[test]
fn post_sends_the_body_and_the_headers_it_was_given() {
    let (port, seen) = serve_once("201 Created", "made");
    let out = lua(
        port,
        r#"
        local r = oslo.http.post(
            "http://127.0.0.1:" .. PORT .. "/new",
            '{"a":1}',
            {headers = {["content-type"] = "application/json"}}
        )
        print(r.status, r.ok, r.body)
    "#,
    );
    assert_eq!(out, "201\ttrue\tmade");

    let request = seen
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("served");
    assert!(
        request.request_line.starts_with("POST /new "),
        "{}",
        request.request_line
    );
    assert_eq!(request.body, r#"{"a":1}"#);
    assert!(
        request
            .headers
            .iter()
            .any(|h| h.eq_ignore_ascii_case("content-type: application/json")),
        "{:?}",
        request.headers
    );
}

#[test]
fn request_takes_the_method_from_its_options() {
    let (port, seen) = serve_once("200 OK", "ok");
    lua(
        port,
        r#"
        oslo.http.request{
            url = "http://127.0.0.1:" .. PORT .. "/thing",
            method = "delete",
        }
    "#,
    );
    let request = seen
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("served");
    // Upper-cased on the way out, so `method = "delete"` is a DELETE and not a 501.
    assert!(
        request.request_line.starts_with("DELETE /thing "),
        "{}",
        request.request_line
    );
}

/// Nothing raises. A refused connection is a condition a script handles, the same as a command
/// that could not be run.
#[test]
fn a_connection_that_fails_answers_rather_than_raising() {
    // Bound and immediately dropped, so the port is almost certainly closed.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("addr").port()
    };
    let out = lua(
        port,
        r#"
        local r = oslo.http.get("http://127.0.0.1:" .. PORT .. "/")
        print(r.ok, r.status, r.error ~= nil)
    "#,
    );
    assert_eq!(out, "false\t0\ttrue");
}

/// A certificate file that was named on purpose and cannot be read is an error — never a quiet
/// fall-through to some other trust store.
#[test]
fn a_missing_cacert_is_reported_and_not_fallen_past() {
    let (port, _seen) = serve_once("200 OK", "unreached");
    let out = lua(
        port,
        r#"
        local r = oslo.http.get("https://127.0.0.1:" .. PORT .. "/", {cacert = "/nope-zz/ca.pem"})
        print(r.ok, r.error:find("/nope%-zz/ca%.pem") ~= nil)
    "#,
    );
    assert_eq!(out, "false\ttrue");
}

#[test]
fn request_without_a_url_is_a_mistake_in_the_script() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("case.lua");
    std::fs::write(&path, "oslo.http.request{method = 'GET'}\n").expect("write");
    let output = Command::new(oslo_bin())
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("`url` is required"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
