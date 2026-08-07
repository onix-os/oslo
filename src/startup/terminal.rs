//! Interactive terminal initialization before the first prompt.

pub fn initialize() {
    oslo::ui::term::watch_for_resize();
    let mut background = oslo::ui::term::query::background_from_environment();
    let mut verified = oslo::ui::term::capability::Verified::default();
    let query_terminal = std::env::var("TERM").as_deref() != Ok("dumb");
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
