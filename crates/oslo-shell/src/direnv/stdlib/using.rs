//! `use <thing>` — direnv's one extension point.
//!
//! `use java` calls `use_java`, so a `direnvrc` can add one and every project gets it. Generic by
//! construction: it knows no `thing` by name, which is why it stays here while the two handlers
//! that *are* about Nix live behind the `nix` feature.

use super::fault;
use crate::env::Environment;
use oslo_base::error::Result;

/// `use <thing> [args...]` — dispatch to `use_<thing>`.
///
/// A shell function wins over the builtin, which is how a `direnvrc` overrides or adds one. Looked
/// up by name at call time rather than resolved when the stdlib is installed, so a `direnvrc` that
/// defines `use_java` *after* something else sourced it still works.
pub fn use_dispatch(env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(thing) = args.get(1) else {
        return fault("use", "needs something to use");
    };
    let target = format!("use_{thing}");
    let mut forwarded = vec![target.clone()];
    forwarded.extend_from_slice(&args[2..]);

    if env.get_function(&target).is_some() {
        return super::run(env, &forwarded);
    }
    match env.get_builtin(&target) {
        Some(func) => func(env, &forwarded),
        None => fault(
            "use",
            &format!("{thing} is not something oslo knows how to use"),
        ),
    }
}
