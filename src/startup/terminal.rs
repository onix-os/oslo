//! Interactive terminal initialization before the first prompt.

/// Whether a multiplexer sits between this shell and the terminal that would answer a query.
///
/// **`$TMUX` and `$STY` are set by the multiplexer itself**, which is what makes them the reliable
/// signal; `$TERM` is a fallback for the case where a configuration has renamed it. `screen` covers
/// GNU screen and the many terminals that still describe themselves that way inside one.
fn multiplexed() -> bool {
    std::env::var_os("TMUX").is_some()
        || std::env::var_os("STY").is_some()
        || std::env::var("TERM")
            .map(|term| term.starts_with("tmux") || term.starts_with("screen"))
            .unwrap_or(false)
}

pub fn initialize() {
    oslo::ui::term::watch_for_resize();
    let mut background = oslo::ui::term::query::background_from_environment();
    let mut verified = oslo::ui::term::capability::Verified::default();

    // **Nothing is asked through a multiplexer.**
    //
    // The startup exchange writes five queries and waits for the reply to `CSI c` to say the rest
    // have arrived. Inside tmux that fence is worthless: tmux answers `CSI c` *itself*, from its
    // own idea of what a terminal is, so the exchange finishes while the real terminal's replies
    // are still in flight. Those bytes then arrive during the first prompt and are read as
    // keystrokes.
    //
    // The background query is worse than useless there and was measured: inside tmux, `OSC 11`
    // returns nothing at all and `$COLORFGBG` is unset. Every reply that *does* come back is
    // tmux's, describing tmux — a terminal whose capabilities are not the ones oslo is drawing on.
    // Recording them is how a session ends up with the wrong syntax palette and a keyboard
    // protocol the terminal never agreed to.
    //
    // So inside a multiplexer the environment is trusted and nothing is asked. `$TERM` and
    // `$COLORTERM` are propagated by tmux and are what every program used before this negotiation
    // existed. tmux's `allow-passthrough` can carry a query to the outer terminal, but it is off by
    // default, it is the user's setting rather than ours, and a reply that arrives after the fence
    // is indistinguishable from something they typed.
    let query_terminal = std::env::var("TERM").as_deref() != Ok("dumb") && !multiplexed();
    if query_terminal && let Some(tty) = oslo::ui::term::Tty::open() {
        let negotiated = oslo::ui::term::negotiate::on(tty.fd(), background.is_none());
        background = background.or(negotiated.background);
        verified = negotiated.verified;
        oslo::ui::term::query::preserve_startup_input(negotiated.pending);
    }
    oslo::ui::term::keyboard::remember_support(verified.kitty_keyboard);
    oslo::ui::term::capability::initialize_with_verified(verified);
    oslo::ui::marks::enable(true);
    if let Some(background) = background {
        oslo::ui::theme::set_background(background);
    }
}

#[cfg(test)]
mod tests {
    /// The three signals, spelled the way the programs that set them spell it.
    ///
    /// Asserted on a pure predicate rather than by setting the variables: `std::env::set_var` is
    /// process-global and this test binary runs in parallel, which is the hazard `direnv/tests.rs`
    /// documents at length.
    fn multiplexed_from(tmux: bool, sty: bool, term: &str) -> bool {
        tmux || sty || term.starts_with("tmux") || term.starts_with("screen")
    }

    #[test]
    fn a_multiplexer_is_recognised_however_it_announces_itself() {
        assert!(multiplexed_from(true, false, "xterm-256color"), "$TMUX");
        assert!(multiplexed_from(false, true, "xterm-256color"), "$STY");
        assert!(multiplexed_from(false, false, "tmux-256color"));
        assert!(multiplexed_from(false, false, "screen-256color"));
    }

    /// A plain terminal is still queried — this guard must not switch the feature off everywhere.
    #[test]
    fn a_terminal_of_its_own_is_still_asked() {
        for term in ["xterm-256color", "foot", "alacritty", "kitty", "wezterm"] {
            assert!(!multiplexed_from(false, false, term), "{term}");
        }
    }
}
