//! What a tab is called, and what to suggest when you have not decided.
//!
//! Greek letters, in order, first one free. A tab is something you name in a hurry — the moment you
//! reach for it is the moment a build you did not want to lose is already running — so the
//! suggestion has to be typed over, not thought about.

/// The suggestions, in order. Twenty-four is more tabs than anybody will have open.
const GREEK: [&str; 24] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "sigma", "tau", "upsilon", "phi", "chi",
    "psi", "omega",
];

/// The first Greek letter nothing is called yet.
///
/// Falls back to `tab-N` once all twenty-four are taken, because refusing to suggest anything at
/// that point would be a worse answer than an ugly one.
pub fn suggest<S: AsRef<str>>(taken: &[S]) -> String {
    let used = |name: &str| taken.iter().any(|t| t.as_ref() == name);
    if let Some(free) = GREEK.iter().find(|name| !used(name)) {
        return (*free).to_string();
    }
    (1..)
        .map(|n| format!("tab-{n}"))
        .find(|name| !used(name))
        .unwrap_or_default()
}

/// Whether a name can be a tab's, which is also whether it can be a filename.
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
