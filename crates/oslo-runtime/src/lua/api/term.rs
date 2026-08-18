//! `oslo.term` — what this terminal can actually do.
//!
//! **Negotiated, not guessed from `$TERM`.** Before the first prompt oslo sends the terminal a
//! handful of queries — `CSI ? u` for the keyboard protocol, `CSI ? 2026 $p` for synchronised
//! output, `OSC 11` for the background — with a Primary DA request behind them as a barrier, and
//! reads what comes back. Everything here is the terminal's own answer, or a documented default
//! where it declined to give one.
//!
//! # Why a config needs this
//!
//! Because the alternative is oslo guessing on your behalf. Ctrl+Enter does not exist on a terminal
//! that does not report modifiers — in the legacy encoding Ctrl-M *is* Enter, the same byte — so a
//! shell that decides "Enter adds a line, Ctrl+Enter sends" without asking produces, on half the
//! terminals in the world, a prompt that can never run anything. oslo used to make that choice from
//! the probe itself, which was still oslo choosing.
//!
//! Now it does not choose. The default is the behaviour that works everywhere, and a config that
//! knows better says so:
//!
//! ```lua
//! if oslo.term.kitty_keyboard() then
//!   oslo.opts.set("lua_enter", "newline")   -- Enter adds a line, Ctrl+Enter sends
//! end
//! ```
//!
//! # What `origin` means
//!
//! Every capability also has an origin, from [`oslo.term.origins`](origins): `verified` is the
//! terminal answering a query, `host` is inferred from what the terminal calls itself, `portable`
//! is a safe default nobody confirmed, `opt-in` was asked for by a config, and `disabled` is off.
//! A `true` that is only `portable` is a guess; a `true` that is `verified` is a fact.

use super::util::{ok, put};
use oslo_base::value::{Table, Value};

/// Build the `oslo.term` table.
pub fn build() -> Table {
    let mut term = Table::new();

    // Each capability is a function rather than a field, because a field is read once when the
    // config runs and these are answered by a terminal that can be replaced under a running
    // session — `oslo exec`, a multiplexer reattaching, a `.env.lua` in another window.
    each_capability(&mut term);

    // The size, which is the one thing here that changes constantly.
    put(&mut term, "size", |_, _| {
        let mut out = Table::new();
        out.set_str(
            "columns",
            Value::int(oslo_ui::dropdown::width::terminal_cols() as i64),
        );
        out.set_str(
            "rows",
            Value::int(oslo_ui::dropdown::width::terminal_rows() as i64),
        );
        ok(Value::table(out))
    });

    // How much colour can be written. `none` when `$NO_COLOR` is set or `$TERM` is `dumb`, which is
    // the question a config drawing its own prompt actually has.
    put(&mut term, "colors", |_, _| {
        ok(Value::str(match oslo_ui::theme::depth() {
            oslo_ui::theme::Depth::None => "none",
            oslo_ui::theme::Depth::Ansi16 => "ansi16",
            oslo_ui::theme::Depth::Ansi256 => "ansi256",
            oslo_ui::theme::Depth::True => "truecolor",
        }))
    });

    // `"dark"`, `"light"`, or nil when the terminal never said and no environment hint named it.
    // Nil rather than a guess: a prompt that picks its colours from this should be able to tell
    // "it said dark" from "nobody knows".
    put(&mut term, "background", |_, _| {
        ok(match oslo_ui::theme::background() {
            Some(oslo_ui::term::query::Background::Dark) => Value::str("dark"),
            Some(oslo_ui::term::query::Background::Light) => Value::str("light"),
            None => Value::Nil,
        })
    });

    // Whether the negotiation happened at all. False for a script, a pipe, or `$TERM=dumb` — and
    // then every capability above is a documented default rather than an answer. "Not asked" and
    // "asked and told no" are different facts and a config should be able to tell them apart.
    put(&mut term, "negotiated", |_, _| {
        ok(Value::Bool(
            oslo_ui::term::capability::snapshot_if_initialized().is_some(),
        ))
    });

    put(&mut term, "origins", |_, _| ok(origins()));
    put(&mut term, "all", |_, _| ok(all()));
    term
}

/// The capabilities, in the order they are worth reading.
///
/// Kept as one table so `all()`, `origins()` and the individual functions cannot drift apart —
/// adding a capability is one row here and it appears in all three.
type Reader = fn(&oslo_ui::term::capability::Capabilities) -> bool;
type OriginReader = fn(&oslo_ui::term::capability::Origins) -> oslo_ui::term::capability::Origin;

const CAPABILITIES: &[(&str, Reader, Option<OriginReader>)] = &[
    // The one that decides whether Ctrl+Enter, Ctrl+Tab and every other modifier chord exist.
    (
        "kitty_keyboard",
        |c| c.kitty_keyboard,
        Some(|o| o.kitty_keyboard),
    ),
    (
        "synchronized_output",
        |c| c.synchronized_output,
        Some(|o| o.synchronized_output),
    ),
    (
        "bracketed_paste",
        |c| c.bracketed_paste,
        Some(|o| o.bracketed_paste),
    ),
    (
        "semantic_clicks",
        |c| c.semantic_clicks,
        Some(|o| o.semantic_clicks),
    ),
    (
        "legacy_clicks",
        |c| c.legacy_clicks,
        Some(|o| o.legacy_clicks),
    ),
    (
        "osc99_notifications",
        |c| c.osc99_notifications,
        Some(|o| o.osc99_notifications),
    ),
    ("osc99_unfocused", |c| c.osc99_unfocused, None),
    ("osc9_progress", |c| c.osc9_progress, None),
    ("osc133_cmdline_url", |c| c.osc133_cmdline_url, None),
    ("osc1337_user_vars", |c| c.osc1337_user_vars, None),
    ("semantic_basic", |c| c.semantic_basic, None),
    ("semantic_secondary", |c| c.semantic_secondary, None),
    ("semantic_right_prompt", |c| c.semantic_right_prompt, None),
    ("vscode_rich", |c| c.vscode_rich, None),
];

/// One predicate per capability: `oslo.term.kitty_keyboard()`.
fn each_capability(term: &mut Table) {
    for &(name, read, _) in CAPABILITIES {
        put(term, name, move |_, _| {
            ok(Value::Bool(match snapshot() {
                Some(caps) => read(caps),
                None => false,
            }))
        });
    }
    // The semantic prompt protocol, which is a choice of three rather than a yes or no.
    put(term, "semantic", |_, _| {
        ok(Value::str(match snapshot().map(|c| c.semantic) {
            Some(oslo_ui::term::capability::SemanticProtocol::Osc133) => "osc133",
            Some(oslo_ui::term::capability::SemanticProtocol::Vscode633) => "vscode633",
            _ => "none",
        }))
    });
}

/// Every capability at once, for printing or for a config that wants to look around.
fn all() -> Value {
    let mut out = Table::new();
    let caps = snapshot();
    for &(name, read, _) in CAPABILITIES {
        out.set_str(name, Value::Bool(caps.is_some_and(read)));
    }
    out.set_str(
        "semantic",
        Value::str(match caps.map(|c| c.semantic) {
            Some(oslo_ui::term::capability::SemanticProtocol::Osc133) => "osc133",
            Some(oslo_ui::term::capability::SemanticProtocol::Vscode633) => "vscode633",
            _ => "none",
        }),
    );
    Value::table(out)
}

/// Where each answer came from: `verified`, `host`, `portable`, `opt-in` or `disabled`.
///
/// The difference between a fact and a guess. `kitty_keyboard = true` with origin `verified` means
/// the terminal answered the query; with origin `portable` it means nobody knows and a safe default
/// was taken.
fn origins() -> Value {
    let mut out = Table::new();
    let Some(caps) = snapshot() else {
        return Value::table(out);
    };
    for &(name, _, origin) in CAPABILITIES {
        if let Some(origin) = origin {
            out.set_str(name, Value::str(origin(&caps.origins).label()));
        }
    }
    Value::table(out)
}

/// The negotiated capabilities, or `None` when nothing was negotiated.
///
/// `snapshot_if_initialized` rather than `snapshot`: the latter *performs* detection, which means
/// terminal I/O, and a config asking a question must not be the thing that triggers a probe.
fn snapshot() -> Option<&'static oslo_ui::term::capability::Capabilities> {
    oslo_ui::term::capability::snapshot_if_initialized()
}
