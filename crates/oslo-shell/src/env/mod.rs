pub mod announce;
pub mod builtins;
pub mod dynamic;
pub mod lists;
pub mod nesting;
pub mod options;
pub mod scope;
/// The shell as one Lua record, for a caller that already holds the state.
pub mod view;

pub use scope::Environment;
/// Where the builtin now running should say its diagnostics came from — see [`scope::origin`].
pub use scope::origin::origin_now;
