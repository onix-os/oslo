//! A bash CLI framework: a script declares its options as comments and this parses them.
//!
//! Vendored code is not restyled — see `vendor/README.md`. The allowances below are for the lints
//! oslo runs at `-D warnings` and upstream does not: what is unused here is unused only because
//! oslo builds a subset of the features, and restyling somebody else's crate to silence a lint is
//! how a fork stops being rebasable.
#![allow(unused_imports, unused_variables, dead_code)]
#![allow(clippy::all, clippy::pedantic)]

mod argc_value;
#[cfg(feature = "build")]
mod build;
mod command;
#[cfg(feature = "compgen")]
mod compgen;
#[cfg(feature = "completions")]
mod completions;
#[cfg(feature = "mangen")]
mod mangen;
#[cfg(any(feature = "eval", feature = "compgen"))]
mod matcher;
mod param;
mod parser;
mod runtime;
#[cfg(any(feature = "compgen", feature = "completions"))]
mod shell;
pub mod utils;

use anyhow::Result;

/// **Re-exported for oslo.** A caller implementing [`Runtime`] has to name `anyhow::Result` in its
/// signatures, and making every such caller depend on `anyhow` directly would be a dependency taken
/// on for one type name.
pub use anyhow;
pub use argc_value::ArgcValue;
#[cfg(feature = "build")]
pub use build::build;
#[cfg(feature = "export")]
pub use command::CommandValue;
#[cfg(feature = "compgen")]
pub use compgen::{compgen, compgen_kind, CompKind, COMPGEN_KIND_SYMBOL};
#[cfg(feature = "completions")]
pub use completions::generate_completions;
#[cfg(feature = "mangen")]
pub use mangen::mangen;
pub use param::{ChoiceValue, DefaultValue};
#[cfg(feature = "export")]
pub use param::{EnvValue, FlagOptionValue, PositionalValue};
#[cfg(feature = "native-runtime")]
pub use runtime::native::NativeRuntime;
#[cfg(any(feature = "eval", feature = "compgen"))]
pub use runtime::Runtime;
#[cfg(any(feature = "compgen", feature = "completions"))]
pub use shell::Shell;

#[cfg(feature = "eval")]
pub fn eval<T: Runtime>(
    runtime: T,
    script_content: &str,
    args: &[String],
    script_path: Option<&str>,
    wrap_width: Option<usize>,
) -> Result<Vec<ArgcValue>> {
    let mut cmd = command::Command::new(script_content, &args[0])?;
    if let Some(p) = script_path {
        if cmd.has_metadata(crate::utils::META_EXTERNAL_SUBCOMMANDS) {
            cmd.external_subcommands = command::collect_external_subcommands(runtime, p);
        }
    }
    cmd.eval(runtime, args, script_path, wrap_width)
}

#[cfg(feature = "export")]
pub fn export(source: &str, root_name: &str) -> Result<CommandValue> {
    let cmd = command::Command::new(source, root_name)?;
    Ok(cmd.export())
}
