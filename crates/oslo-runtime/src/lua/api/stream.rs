//! `oslo.stream` — a unix socket, as a Lua handle.
//!
//! ```lua
//! local h = oslo.stream.connect("/run/user/1000/onix/hexe/main.sock")
//! h:send(bytes)
//! local reply = h:recv()
//! h:close()
//! ```
//!
//! # The one native the client library needs
//!
//! Everything else about talking to another tool — framing, encoding, the verbs — is plain Lua in
//! [`client.lua`](../client.lua), so it is copied between siblings rather than reimplemented. This
//! is the piece that cannot be: a socket has to come from the host.
//!
//! It is deliberately *four functions and no protocol*. Anything that knows what a frame is belongs
//! in the Lua half, where a fix reaches every tool that copied it.
//!
//! # Bytes, not text
//!
//! `send` takes and `recv` answers [`Value::bytes`]. A frame header is four bytes of length and a
//! payload may hold anything; running either through UTF-8 validation would corrupt exactly the
//! values the binary encoding exists to carry.
//!
//! # Deadlines are not optional
//!
//! A blocking read from a peer that has stopped answering hangs the shell with no way out but
//! SIGINT, and a blocking `connect` to a socket whose backlog is full parks in the kernel with no
//! timeout of its own. Both take one here, defaulting to something a person will wait for.

use super::handle::Handle;
use super::util::{failed, int, ok, opt_text, put, raw, text};
use oslo_base::value::{Table, Value};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::time::Duration;

/// How long a connect, a send or a receive may take before it gives up.
///
/// Long enough that a peer doing real work is waited for, short enough that a wedged one does not
/// take the prompt with it. `oslo.stream.connect(path, ms)` overrides it per call.
const DEFAULT_TIMEOUT_MS: i64 = 5_000;

/// The largest single `recv`. A frame header names its own length, so a caller that wants more asks
/// again; this is the ceiling on one syscall's buffer, not on a conversation.
const MAX_RECV: usize = 1 << 20;

pub fn build() -> Value {
    let mut it = Table::new();

    // oslo.stream.connect(path, timeout_ms) -> handle, or nil + message
    put(&mut it, "connect", |_, args| {
        let path = text(&args, 1, "oslo.stream.connect")?;
        let timeout = match args.get(1) {
            Some(Value::Nil) | None => DEFAULT_TIMEOUT_MS,
            _ => int(&args, 2, "oslo.stream.connect")?,
        };
        match dial(&path, timeout) {
            Ok(sock) => ok(stream(sock, &path)),
            Err(e) => failed(&format!("connect {path}"), e),
        }
    });

    // oslo.stream.path(tool, session) -> where that tool's socket would be
    //
    // The convention in one place, so a client library does not spell it out and a server does not
    // spell it differently. Answers a path whether or not anything is listening on it — asking is
    // `connect`'s job.
    put(&mut it, "path", |_, args| {
        let tool = text(&args, 1, "oslo.stream.path")?;
        let session = opt_text(&args, 2, "oslo.stream.path")?;
        ok(Value::str(
            oslo_base::wire::socket_path(&tool, session.as_deref()).to_string_lossy(),
        ))
    });

    Value::table(it)
}

/// Connect with a deadline, and apply the same deadline to reads and writes.
fn dial(path: &str, timeout_ms: i64) -> std::io::Result<UnixStream> {
    let limit = Duration::from_millis(timeout_ms.max(1) as u64);
    let sock = UnixStream::connect(path)?;
    sock.set_read_timeout(Some(limit))?;
    sock.set_write_timeout(Some(limit))?;
    Ok(sock)
}

/// The handle: `send`, `recv`, `close`, and `<close>` for a scoped `local`.
fn stream(sock: UnixStream, path: &str) -> Value {
    let held = Rc::new(RefCell::new(Some(sock)));
    let mut handle = Handle::new("oslo.stream");
    handle.field("path", Value::str(path)).shows(path);

    let it = Rc::clone(&held);
    handle.verb("send", move |_, args| {
        let bytes = raw(&args, 2, "oslo.stream:send")?;
        let mut slot = it.borrow_mut();
        let Some(sock) = slot.as_mut() else {
            return failed("send", "the stream is closed");
        };
        match sock.write_all(&bytes).and_then(|()| sock.flush()) {
            Ok(()) => ok(Value::int(bytes.len() as i64)),
            Err(e) => failed("send", e),
        }
    });

    // h:recv(n) — up to `n` bytes, or whatever one read answers.
    //
    // **Short reads are the caller's to handle**, because a framed protocol already knows how many
    // bytes it is owed and a stream is free to deliver them in pieces. Answering `""` at end of
    // stream and `nil, message` on a real failure keeps those two apart, which a caller in a loop
    // has to be able to tell.
    let it = Rc::clone(&held);
    handle.verb("recv", move |_, args| {
        let want = match args.get(1) {
            Some(Value::Nil) | None => MAX_RECV,
            _ => (int(&args, 2, "oslo.stream:recv")?.max(0) as usize).min(MAX_RECV),
        };
        let mut slot = it.borrow_mut();
        let Some(sock) = slot.as_mut() else {
            return failed("recv", "the stream is closed");
        };
        let mut buffer = vec![0u8; want];
        match sock.read(&mut buffer) {
            Ok(read) => {
                buffer.truncate(read);
                ok(Value::bytes(&buffer))
            }
            Err(e) => failed("recv", e),
        }
    });

    let it = Rc::clone(&held);
    handle.verb("close", move |_, _| {
        ok(Value::Bool(it.borrow_mut().take().is_some()))
    });

    handle.on_close("oslo.stream.close", move || {
        let _ = held.borrow_mut().take();
    });

    handle.build()
}
