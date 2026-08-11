//! What a scratch may be called.
//!
//! There is no suggested name and nothing is ever auto-named: the finder offers to create only what
//! has been typed into it, so every scratch is called what somebody meant it to be called.

/// Whether a name can be a scratch's, which is also whether it can be a filename.
///
/// **A name is part of a path**, so this is the same class of decision as a profile name: refuse
/// rather than sanitise, because a name that is quietly rewritten is a name you cannot find again.
/// No separators, no leading dot, nothing that means something to a shell reading the directory.
pub fn valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests;
