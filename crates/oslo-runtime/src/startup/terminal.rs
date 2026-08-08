//! Interactive terminal initialization before the first prompt.

pub fn initialize() {
    oslo_ui::term::watch_for_resize();
    let mut background = oslo_ui::term::query::background_from_environment();
    let mut verified = oslo_ui::term::capability::Verified::default();
    let query_terminal = std::env::var("TERM").as_deref() != Ok("dumb");
    if query_terminal && let Some(tty) = oslo_ui::term::Tty::open() {
        let negotiated = oslo_ui::term::negotiate::on(tty.fd(), background.is_none());
        background = background.or(negotiated.background);
        verified = negotiated.verified;
        oslo_ui::term::query::preserve_startup_input(negotiated.pending);
    }
    oslo_ui::term::keyboard::remember_support(verified.kitty_keyboard);
    oslo_ui::term::capability::initialize_with_verified(verified);
    oslo_ui::marks::enable(true);
    if let Some(background) = background {
        oslo_ui::theme::set_background(background);
    }
}
