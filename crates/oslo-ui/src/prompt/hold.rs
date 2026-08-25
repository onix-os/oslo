//! Keeping the prompt alive while something else runs.
//!
//! A command normally takes the screen with it: the editor has handed the line back, so the loop
//! that ticks an animated segment, services the background and repaints is gone until the command
//! ends. That is right almost always — a file browser drawn inline owns the terminal, and anything
//! oslo wrote over it would be damage.
//!
//! **Except when the command draws somewhere else.** A browser opened in a terminal mux's float
//! leaves oslo's own screen sitting there with a prompt on it, and that prompt then spends the whole
//! visit frozen: a spinner stops turning, and a directory the browser moves the shell to — over the
//! control socket, while you navigate — is not shown until you come back.
//!
//! So a command may be marked as not owning the terminal, and while it runs [`pump`] is called in
//! its wait loop. What it does is the editor's loop with the keyboard taken out: service the
//! background, re-render if that changed anything, redraw where the prompt is.
//!
//! # Why the renderer is a `fn` and not a closure
//!
//! Building a prompt means running Lua, which is `oslo-runtime`'s, and by the time a command is
//! waiting the closure the editor was given has been dropped. A plain function pointer can be
//! registered once for the session and reach the interpreter through the thread-local the runtime
//! already keeps for exactly this — hooks fire from places with no engine in hand the same way.

use std::sync::atomic::{AtomicBool, Ordering};

/// Renders the prompt as it stands now: the left and the right. `None` when it cannot be built —
/// no interpreter on this thread, or the shell state held by something else.
type Render = fn() -> Option<(String, String)>;

thread_local! {
    static RENDER: std::cell::Cell<Option<Render>> = const { std::cell::Cell::new(None) };
}

/// Whether a prompt is on screen to keep alive.
///
/// **Not the same question as "is a command running".** A command run from a script, from `-c`, or
/// before the first prompt has nothing to repaint, and a pump that drew one there would put a
/// prompt in the middle of somebody's output.
static SHOWING: AtomicBool = AtomicBool::new(false);

/// Register how to build a prompt. Called once, by startup.
pub fn renders_with(render: Render) {
    RENDER.with(|slot| slot.set(Some(render)));
}

/// Say whether a prompt is on screen and worth keeping alive.
pub fn showing(yes: bool) {
    SHOWING.store(yes, Ordering::Relaxed);
}

/// One turn of the editor's loop, with the keyboard taken out.
///
/// Answers whether it drew, so a caller can tell a wait that did something from one that did not.
///
/// **Drawn from the cursor's own row.** Where a prompt is depends on how the line that started this
/// ended: a key bound with `erase` parks the cursor at the top of the prompt block it left, which is
/// exactly where the block goes again. Nothing here counts rows or remembers a position, because
/// the one thing that reliably knows is the terminal.
pub fn pump(cols: usize) -> bool {
    if !SHOWING.load(Ordering::Relaxed) {
        return false;
    }
    let Some(render) = RENDER.with(|slot| slot.get()) else {
        return false;
    };
    // The same servicing the input wait does: timers, a finished job, and a directory a peer asked
    // for over the control socket. It is what makes this worth doing at all.
    let serviced = oslo_base::background::service();
    let ticked = crate::prompt::tick_due();
    if !serviced && !ticked {
        return false;
    }
    let Some((left, right)) = render() else {
        return false;
    };
    let placed = crate::edit::layout::place(&crate::edit::layout::Row {
        prompt: &left,
        text: "",
        plain: "",
        cursor: 0,
        hint: "",
        right: &right,
        cols,
        lead: crate::transcript::lead(),
    });
    let frame = crate::edit::screen::redraw(
        0,
        &placed.text,
        crate::edit::screen::At {
            rows: placed.rows,
            cursor_row: placed.cursor_row,
            cursor_col: placed.cursor_col,
        },
    );
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(frame.as_bytes());
    let _ = out.flush();
    true
}

#[cfg(test)]
#[path = "hold/tests.rs"]
mod tests;
