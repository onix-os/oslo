//! `oslo.http` — HTTP, in the binary rather than as a library.
//!
//! ```lua
//! local r = oslo.http.get("https://example.com/")
//! if r.ok then print(r.body) end
//!
//! oslo.http.request{ url = u, method = "POST", body = payload,
//!                    headers = {["content-type"] = "application/json"} }
//! ```
//!
//! **The result is `oslo.run`'s.** `{status, ok, body, headers}`, and it never raises: `ok` is
//! `status < 400`, and a connection that failed comes back with `ok = false` and an `error`
//! field. One rule covers a command and a request, which is one rule fewer to remember.
//!
//! **Certificates come from the system, exactly as curl's do** — see [`certs`]. Nothing is
//! bundled, so a distrusted authority stops being trusted when the distribution says so rather
//! than when oslo is next rebuilt.
//!
//! `insecure = true` is curl's `-k`, and is per-request and never a default. There is no way to
//! turn verification off globally, because the global form is the one that gets set in a config
//! file during an afternoon of debugging and stays there.

mod certs;

use super::util::{ok, put, record, text};
use crate::lua::eval::value::{Table, Value};
use crate::lua::eval::{LuaError, LuaResult};
use std::sync::OnceLock;

pub fn build() -> Value {
    let mut http = Table::new();

    // oslo.http.get(url, opts)
    put(&mut http, "get", |_, args| {
        let url = text(&args, 1, "oslo.http.get")?;
        let request = Request::from_lua(args.get(1), &url, "GET")?;
        ok(perform(request))
    });

    // oslo.http.post(url, body, opts)
    put(&mut http, "post", |_, args| {
        let url = text(&args, 1, "oslo.http.post")?;
        let body = text(&args, 2, "oslo.http.post")?;
        let mut request = Request::from_lua(args.get(2), &url, "POST")?;
        request.body = Some(body);
        ok(perform(request))
    });

    // oslo.http.request{url = …, method = …, …} — everything, in one table.
    put(&mut http, "request", |_, args| {
        let Some(Value::Table(t)) = args.first() else {
            return Err(LuaError::new(
                "oslo.http.request: expected a table of options",
            ));
        };
        let url = match t.borrow().get(&Value::str("url")) {
            Value::Str(u) => u.to_string(),
            _ => return Err(LuaError::new("oslo.http.request: `url` is required")),
        };
        let request = Request::from_lua(args.first(), &url, "GET")?;
        ok(perform(request))
    });

    Value::table(http)
}

/// One request, read off the options table.
struct Request {
    url: String,
    method: String,
    body: Option<String>,
    headers: Vec<(String, String)>,
    timeout: Option<u64>,
    insecure: bool,
    certs: certs::Requested,
}

impl Request {
    fn from_lua(options: Option<&Value>, url: &str, default_method: &str) -> LuaResult<Self> {
        let mut request = Request {
            url: url.to_string(),
            method: default_method.to_string(),
            body: None,
            headers: Vec::new(),
            timeout: None,
            insecure: false,
            certs: certs::Requested::default(),
        };
        let Some(Value::Table(t)) = options else {
            return Ok(request);
        };
        let table = t.borrow();

        if let Value::Str(m) = table.get(&Value::str("method")) {
            request.method = m.to_uppercase();
        }
        if let Value::Str(b) = table.get(&Value::str("body")) {
            request.body = Some(b.to_string());
        }
        if let Value::Table(h) = table.get(&Value::str("headers")) {
            for (name, value) in h.borrow().pairs() {
                if let (Value::Str(name), Value::Str(value)) = (&name, &value) {
                    request.headers.push((name.to_string(), value.to_string()));
                }
            }
            // Sorted, so a request built from a Lua table sends its headers in a stable order —
            // table iteration has none, and an unstable order makes a failure unreproducible.
            request.headers.sort();
        }
        if let Some(seconds) = table.get(&Value::str("timeout")).as_number() {
            request.timeout = Some(seconds.as_float().max(0.0) as u64);
        }
        request.insecure = table.get(&Value::str("insecure")).truthy();
        if let Value::Str(path) = table.get(&Value::str("cacert")) {
            request.certs.cacert = Some(path.to_string());
        }
        if let Value::Str(path) = table.get(&Value::str("capath")) {
            request.certs.capath = Some(path.to_string());
        }
        Ok(request)
    }
}

/// Install the pure-Rust crypto provider, once.
///
/// rustls asks for one process-wide, and `install_default` fails if something already did it —
/// which is fine and not an error here, since either way there is a provider in place.
fn provider() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let _ =
            rustls::crypto::CryptoProvider::install_default(rustls_graviola::default_provider());
    });
}

/// Make the request, and turn everything that can go wrong into a result table.
fn perform(request: Request) -> Value {
    provider();
    match send(&request) {
        Ok(value) => value,
        // A refused connection, a name that does not resolve, an expired certificate: all of
        // these are conditions a script handles, not bugs in it. `ok` is false and `error` says
        // what happened — the same shape a command that could not be run comes back with.
        Err(message) => record(vec![
            ("ok", Value::Bool(false)),
            ("status", Value::int(0)),
            ("error", Value::str(message)),
        ]),
    }
}

fn send(request: &Request) -> Result<Value, String> {
    let agent = agent(request)?;
    let mut builder = ureq::http::Request::builder()
        .method(request.method.as_str())
        .uri(&request.url);
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }

    // The two arms differ only in the body type, which is what decides whether a `content-length`
    // is sent at all — a GET with an empty body is not the same request as a GET with none.
    let sent = match &request.body {
        Some(body) => builder
            .body(body.as_str())
            .map_err(|e| format!("{}: {e}", request.url))
            .and_then(|r| agent.run(r).map_err(|e| format!("{}: {e}", request.url))),
        None => builder
            .body(())
            .map_err(|e| format!("{}: {e}", request.url))
            .and_then(|r| agent.run(r).map_err(|e| format!("{}: {e}", request.url))),
    };
    let mut response = sent?;

    let status = response.status().as_u16() as i64;
    let mut headers = Table::new();
    for (name, value) in response.headers() {
        // Lower-cased, because HTTP header names are case-insensitive and a script indexing
        // `r.headers["content-type"]` should not have to guess how the server spelled it.
        if let Ok(text) = value.to_str() {
            headers.set(
                Value::str(name.as_str().to_ascii_lowercase()),
                Value::str(text),
            );
        }
    }
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("{}: reading the body: {e}", request.url))?;

    Ok(record(vec![
        ("status", Value::int(status)),
        // `ok` is `status < 400`, matching curl's `--fail`: a 404 is a request that worked and
        // an answer that says no, but treating it as success is how a script downloads an error
        // page and writes it to disk as if it were the file.
        ("ok", Value::Bool((200..400).contains(&status))),
        ("body", Value::str(body)),
        ("headers", Value::table(headers)),
    ]))
}

/// Build the agent, with the trust store this request asked for.
fn agent(request: &Request) -> Result<ureq::Agent, String> {
    // A 404 is an answer, not a failure to get one — curl prints the body and exits 0 unless
    // `--fail` is given. Left at ureq's default, a missing page would come back indistinguishable
    // from a refused connection: `status = 0` and no body to look at.
    let mut config = ureq::Agent::config_builder().http_status_as_error(false);
    if let Some(seconds) = request.timeout {
        config = config.timeout_global(Some(std::time::Duration::from_secs(seconds)));
    }

    let tls = if request.insecure {
        // curl's `-k`. Verification is off for *this request only*; there is deliberately no
        // global switch.
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::Rustls)
            .disable_verification(true)
            .build()
    } else {
        let roots = certs::load(&request.certs)?;
        let certificates: Vec<ureq::tls::Certificate<'static>> = roots
            .iter()
            .map(|der| ureq::tls::Certificate::from_der(der.as_ref()).to_owned())
            .collect();
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::Rustls)
            .root_certs(ureq::tls::RootCerts::Specific(std::sync::Arc::new(
                certificates,
            )))
            .build()
    };

    Ok(config.tls_config(tls).build().into())
}
