//! Terminal features selected once before the first prompt.

use std::sync::atomic::{AtomicPtr, Ordering};

/// The semantic protocol used for command lifecycle events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticProtocol {
    None,
    Osc133,
    Vscode633,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Disabled,
    Portable,
    Host,
    Verified,
    OptIn,
}

impl Origin {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Portable => "portable",
            Self::Host => "host",
            Self::Verified => "verified",
            Self::OptIn => "opt-in",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origins {
    pub semantic: Origin,
    pub bracketed_paste: Origin,
    pub kitty_keyboard: Origin,
    pub synchronized_output: Origin,
    pub semantic_clicks: Origin,
    pub legacy_clicks: Origin,
    pub osc99_notifications: Origin,
    pub fallback_notifications: Origin,
}

impl Origins {
    const fn disabled() -> Self {
        Self {
            semantic: Origin::Disabled,
            bracketed_paste: Origin::Disabled,
            kitty_keyboard: Origin::Disabled,
            synchronized_output: Origin::Disabled,
            semantic_clicks: Origin::Disabled,
            legacy_clicks: Origin::Disabled,
            osc99_notifications: Origin::Disabled,
            fallback_notifications: Origin::Disabled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncOverride {
    Auto,
    On,
    Off,
}

/// Features available for one terminal session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub semantic: SemanticProtocol,
    pub semantic_basic: bool,
    pub semantic_secondary: bool,
    pub semantic_right_prompt: bool,
    pub osc133_cmdline_url: bool,
    pub synchronized_output: bool,
    pub bracketed_paste: bool,
    pub kitty_keyboard: bool,
    pub semantic_clicks: bool,
    pub legacy_clicks: bool,
    pub vscode_rich: bool,
    pub osc1337_user_vars: bool,
    pub osc9_progress: bool,
    pub osc99_notifications: bool,
    pub osc99_unfocused: bool,
    pub origins: Origins,
    sync_override: SyncOverride,
}

impl Capabilities {
    pub const fn disabled() -> Self {
        Self {
            semantic: SemanticProtocol::None,
            semantic_basic: false,
            semantic_secondary: false,
            semantic_right_prompt: false,
            osc133_cmdline_url: false,
            synchronized_output: false,
            bracketed_paste: false,
            kitty_keyboard: false,
            semantic_clicks: false,
            legacy_clicks: false,
            vscode_rich: false,
            osc1337_user_vars: false,
            osc9_progress: false,
            osc99_notifications: false,
            osc99_unfocused: false,
            origins: Origins::disabled(),
            sync_override: SyncOverride::Off,
        }
    }

    /// Conservative features that are safe on an otherwise unknown terminal.
    pub const fn portable() -> Self {
        Self {
            semantic: SemanticProtocol::Osc133,
            semantic_basic: true,
            semantic_secondary: false,
            semantic_right_prompt: false,
            osc133_cmdline_url: false,
            synchronized_output: false,
            bracketed_paste: true,
            kitty_keyboard: false,
            semantic_clicks: false,
            legacy_clicks: false,
            vscode_rich: false,
            osc1337_user_vars: false,
            osc9_progress: false,
            osc99_notifications: false,
            osc99_unfocused: false,
            origins: Origins {
                semantic: Origin::Portable,
                bracketed_paste: Origin::Portable,
                fallback_notifications: Origin::Portable,
                ..Origins::disabled()
            },
            sync_override: SyncOverride::Auto,
        }
    }

    /// Select features guaranteed by an exact host contract.
    pub fn from_environment(term_program: Option<&str>) -> Self {
        let mut capabilities = Self::portable();
        match term_program {
            Some("vscode") => {
                capabilities.semantic = SemanticProtocol::Vscode633;
                capabilities.vscode_rich = true;
                capabilities.origins.semantic = Origin::Host;
            }
            Some("iTerm.app") => {
                capabilities.osc1337_user_vars = true;
                capabilities.osc9_progress = true;
            }
            Some("WezTerm") => {
                capabilities.osc1337_user_vars = true;
            }
            _ => {}
        }
        capabilities
    }

    /// Apply editor features requested explicitly by the user.
    pub fn with_editor_opt_ins(mut self, click_events: Option<&str>) -> Self {
        self.semantic_clicks = click_events == Some("1");
        self.legacy_clicks = click_events == Some("legacy");
        if self.semantic_clicks {
            self.origins.semantic_clicks = Origin::OptIn;
        }
        if self.legacy_clicks {
            self.origins.legacy_clicks = Origin::OptIn;
        }
        self
    }

    /// Apply output and input modes requested explicitly by the user.
    pub fn with_explicit_opt_ins(
        mut self,
        synchronized_output: Option<&str>,
        click_events: Option<&str>,
        semantic_extensions: Option<&str>,
    ) -> Self {
        if let Some(value) = synchronized_output {
            self.synchronized_output = value == "1";
            self.sync_override = if self.synchronized_output {
                self.origins.synchronized_output = Origin::OptIn;
                SyncOverride::On
            } else {
                SyncOverride::Off
            };
        }
        if semantic_extensions == Some("kitty") {
            self.semantic_secondary = true;
            self.osc133_cmdline_url = true;
        }
        self.with_editor_opt_ins(click_events)
    }

    /// Apply features established by bounded negotiation.
    pub fn with_verified(mut self, verified: Verified) -> Self {
        self.semantic_secondary |= verified.semantic_secondary;
        self.semantic_right_prompt |= verified.semantic_right_prompt;
        self.osc133_cmdline_url |= verified.semantic_cmdline_url;
        if verified.synchronized_output && self.sync_override != SyncOverride::Off {
            if !self.synchronized_output {
                self.origins.synchronized_output = Origin::Verified;
            }
            self.synchronized_output = true;
        }
        if verified.kitty_keyboard {
            self.kitty_keyboard = true;
            self.origins.kitty_keyboard = Origin::Verified;
        }
        if verified.semantic_clicks && !self.semantic_clicks {
            self.semantic_clicks = true;
            self.origins.semantic_clicks = Origin::Verified;
        }
        self.osc9_progress |= verified.osc9_progress;
        if verified.osc99_notifications {
            self.osc99_notifications = true;
            self.origins.osc99_notifications = Origin::Verified;
        }
        self.osc99_unfocused |= verified.osc99_unfocused;
        self
    }

    /// Stable diagnostic data with no environment reads or terminal queries.
    pub const fn summary(self) -> Summary {
        Summary {
            semantic: self.semantic,
            semantic_basic: self.semantic_basic,
            semantic_secondary: self.semantic_secondary,
            semantic_right_prompt: self.semantic_right_prompt,
            semantic_cmdline_url: self.osc133_cmdline_url,
            synchronized_output: self.synchronized_output,
            bracketed_paste: self.bracketed_paste,
            kitty_keyboard_disambiguate: self.kitty_keyboard,
            semantic_clicks: self.semantic_clicks,
            legacy_clicks: self.legacy_clicks,
            vscode_rich: self.vscode_rich,
            osc1337_user_vars: self.osc1337_user_vars,
            osc9_progress: self.osc9_progress,
            osc99_notifications: self.osc99_notifications,
            osc99_unfocused: self.osc99_unfocused,
            origins: self.origins,
        }
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::portable()
    }
}

/// Results that may only be set by a successful query or verified transport.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Verified {
    pub semantic_secondary: bool,
    pub semantic_right_prompt: bool,
    pub semantic_cmdline_url: bool,
    pub synchronized_output: bool,
    pub kitty_keyboard: bool,
    pub semantic_clicks: bool,
    pub osc9_progress: bool,
    pub osc99_notifications: bool,
    pub osc99_unfocused: bool,
}

/// Read-only view used by tests and future diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub semantic: SemanticProtocol,
    pub semantic_basic: bool,
    pub semantic_secondary: bool,
    pub semantic_right_prompt: bool,
    pub semantic_cmdline_url: bool,
    pub synchronized_output: bool,
    pub bracketed_paste: bool,
    pub kitty_keyboard_disambiguate: bool,
    pub semantic_clicks: bool,
    pub legacy_clicks: bool,
    pub vscode_rich: bool,
    pub osc1337_user_vars: bool,
    pub osc9_progress: bool,
    pub osc99_notifications: bool,
    pub osc99_unfocused: bool,
    pub origins: Origins,
}

/// The session's capabilities, replaceable.
///
/// **It was a `OnceLock`, and once is exactly the problem.** Detection is one bounded exchange with
/// a 100 ms budget, run before the first prompt — and a terminal that misses that window, on a
/// machine busy enough to lose it, left the shell with the degraded answer for the rest of its life.
/// Nothing could ask again: `reset` cannot reach this, because it is the shell's own memory rather
/// than the terminal's state.
///
/// A pointer to a leaked value rather than a lock, so that [`snapshot`] still hands out
/// `&'static Capabilities` and none of its thirty-odd readers change. Re-probing is rare and asked
/// for, so the handful of bytes each one leaks is not a cost worth a lock on the read path.
static SESSION: AtomicPtr<Capabilities> = AtomicPtr::new(std::ptr::null_mut());

/// Install `capabilities` as the session's, replacing whatever was there.
fn install(capabilities: Capabilities) -> &'static Capabilities {
    let leaked: &'static mut Capabilities = Box::leak(Box::new(capabilities));
    SESSION.store(leaked as *mut Capabilities, Ordering::Release);
    leaked
}

/// Forget the session snapshot, so the next read detects again.
///
/// The shell calls this when the terminal has been reset under it: what was true of the old terminal
/// state is no longer known to be true, and a stale "no, this terminal cannot do that" is the answer
/// that never comes right on its own.
pub fn forget() {
    SESSION.store(std::ptr::null_mut(), Ordering::Release);
}

/// Detect host-contract features without installing the session snapshot.
pub fn detect_host() -> Capabilities {
    Capabilities::from_environment(std::env::var("TERM_PROGRAM").ok().as_deref())
        .with_explicit_opt_ins(
            std::env::var("OSLO_SYNC_OUTPUT").ok().as_deref(),
            std::env::var("OSLO_CLICK_EVENTS").ok().as_deref(),
            std::env::var("OSLO_TERMINAL_EXTENSIONS").ok().as_deref(),
        )
}

/// Install query results before any prompt or editor reads the snapshot.
///
/// **Replaces what is there**, which is what makes a second negotiation mean anything: this is how
/// a re-probe hands its answer back, and the answer is worth having only if it is allowed to differ
/// from the one that was wrong.
pub fn initialize_with_verified(verified: Verified) -> &'static Capabilities {
    install(detect_host().with_verified(verified))
}

/// Return the session snapshot, detecting from the environment if nothing is installed.
pub fn initialize() -> &'static Capabilities {
    let installed = SESSION.load(Ordering::Acquire);
    if !installed.is_null() {
        // SAFETY: only `install` ever stores here, and what it stores is leaked for the process's
        // lifetime, so a non-null pointer is always a live `Capabilities`.
        return unsafe { &*installed };
    }
    install(detect_host())
}

/// Return the immutable session snapshot.
pub fn snapshot() -> &'static Capabilities {
    initialize()
}

/// Return the installed session without performing detection or terminal I/O.
pub fn snapshot_if_initialized() -> Option<&'static Capabilities> {
    let installed = SESSION.load(Ordering::Acquire);
    // SAFETY: as in `initialize` — non-null means a leaked, live `Capabilities`.
    (!installed.is_null()).then(|| unsafe { &*installed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_hosts_get_only_safe_portable_features() {
        let capabilities = Capabilities::from_environment(Some("xterm-kitty"));
        assert_eq!(capabilities.semantic, SemanticProtocol::Osc133);
        assert!(capabilities.semantic_basic);
        assert!(capabilities.bracketed_paste);
        assert!(!capabilities.osc133_cmdline_url);
        assert!(!capabilities.synchronized_output);
        assert!(!capabilities.kitty_keyboard);
        assert!(!capabilities.semantic_clicks);
        assert!(!capabilities.legacy_clicks);
        assert!(!capabilities.osc9_progress);
        assert!(!capabilities.osc99_notifications);
    }

    #[test]
    fn disabled_sessions_select_a_null_backend() {
        let capabilities = Capabilities::disabled();
        assert_eq!(capabilities.semantic, SemanticProtocol::None);
        assert!(!capabilities.semantic_basic);
        assert!(!capabilities.bracketed_paste);
        assert!(!capabilities.kitty_keyboard);
    }

    #[test]
    fn exact_host_contracts_enable_only_their_own_features() {
        let vscode = Capabilities::from_environment(Some("vscode"));
        assert_eq!(vscode.semantic, SemanticProtocol::Vscode633);
        assert!(vscode.vscode_rich);
        assert!(!vscode.osc1337_user_vars);

        for host in ["iTerm.app", "WezTerm"] {
            let capabilities = Capabilities::from_environment(Some(host));
            assert!(capabilities.osc1337_user_vars, "{host}");
            assert!(!capabilities.vscode_rich, "{host}");
        }
        assert!(Capabilities::from_environment(Some("iTerm.app")).osc9_progress);
        assert!(!Capabilities::from_environment(Some("WezTerm")).osc9_progress);
    }

    #[test]
    fn click_events_require_an_explicit_exact_opt_in() {
        for value in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
            assert!(
                !Capabilities::portable()
                    .with_editor_opt_ins(value)
                    .semantic_clicks
            );
        }
        assert!(
            Capabilities::portable()
                .with_editor_opt_ins(Some("1"))
                .semantic_clicks
        );
        assert!(
            Capabilities::portable()
                .with_editor_opt_ins(Some("legacy"))
                .legacy_clicks
        );
        assert!(
            !Capabilities::portable()
                .with_editor_opt_ins(Some("legacy"))
                .semantic_clicks
        );
    }

    #[test]
    fn synchronized_output_requires_an_explicit_exact_opt_in() {
        let off = Capabilities::portable().with_explicit_opt_ins(Some("true"), None, None);
        assert!(!off.synchronized_output);
        let on = Capabilities::portable().with_explicit_opt_ins(Some("1"), None, None);
        assert!(on.synchronized_output);
        let forced_off = Capabilities::portable()
            .with_explicit_opt_ins(Some("0"), None, None)
            .with_verified(Verified {
                synchronized_output: true,
                ..Verified::default()
            });
        assert!(!forced_off.synchronized_output);
    }

    #[test]
    fn kitty_semantic_extensions_require_an_exact_opt_in() {
        for value in [None, Some(""), Some("1"), Some("Kitty")] {
            let capabilities = Capabilities::portable().with_explicit_opt_ins(None, None, value);
            assert!(!capabilities.semantic_secondary, "{value:?}");
            assert!(!capabilities.osc133_cmdline_url, "{value:?}");
        }
        let capabilities =
            Capabilities::portable().with_explicit_opt_ins(None, None, Some("kitty"));
        assert!(capabilities.semantic_secondary);
        assert!(capabilities.osc133_cmdline_url);
    }

    #[test]
    fn negotiated_features_are_explicit_and_visible_in_the_summary() {
        let capabilities = Capabilities::portable().with_verified(Verified {
            synchronized_output: true,
            kitty_keyboard: true,
            osc99_notifications: true,
            ..Verified::default()
        });
        let summary = capabilities.summary();
        assert!(summary.synchronized_output);
        assert!(summary.kitty_keyboard_disambiguate);
        assert!(summary.osc99_notifications);
        assert!(!summary.semantic_clicks);
        assert!(!summary.legacy_clicks);
    }
}
