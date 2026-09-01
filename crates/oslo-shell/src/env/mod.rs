pub mod announce;
pub mod builtins;
/// Saying a diagnostic in whichever of its two faces the reader can use.
pub mod diagnose;
pub mod dynamic;
pub mod lists;
pub mod nesting;
pub mod options;
pub mod scope;
/// The shell as one Lua record, for a caller that already holds the state.
pub mod view;

/// Where the builtin now running should say its diagnostics came from — see [`scope::origin`].
pub use diagnose::{complain, complain_option, complain_with_usage, complain_within};
pub use scope::Environment;
pub use scope::origin::origin_now;
