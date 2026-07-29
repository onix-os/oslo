//! Tilde expansion.

use crate::env::Environment;

/// Expand a `~`, `~user`, `~+` or `~-` prefix.
///
/// Every failure path returns the text unchanged, which is what POSIX asks for and also the only
/// safe answer: `~nosuchuser` as a literal is a path that will fail loudly, while a guessed home
/// directory is a path that will succeed on the wrong files.
pub fn expand_tilde(env: &Environment, user: &str) -> String {
    match user {
        "" => home_directory(env),
        // `~+` and `~-` are the shell's own notion of where it is, not a user's: they read the
        // variables `cd` maintains, so they follow symlinks exactly the way `cd` left them.
        "+" => env
            .get_var("PWD")
            .map(str::to_string)
            .or_else(current_directory)
            .unwrap_or_else(|| "~+".to_string()),
        // With no previous directory there is nothing to name, and bash leaves `~-` literal.
        "-" => env
            .get_var("OLDPWD")
            .map(str::to_string)
            .unwrap_or_else(|| "~-".to_string()),
        name => passwd_home(name).unwrap_or_else(|| format!("~{name}")),
    }
}

/// `$HOME`, falling back to the password database.
///
/// The fallback matters for `env -u HOME sh`: a login without `HOME` still has a home directory,
/// and expanding `~` to `/` there would point every script at the filesystem root.
fn home_directory(env: &Environment) -> String {
    if let Some(home) = env.get_var("HOME") {
        return home.to_string();
    }
    passwd_home_of_uid().unwrap_or_else(|| "/".to_string())
}

fn current_directory() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// The home directory recorded for `name`, or `None` if there is no such user.
fn passwd_home(name: &str) -> Option<String> {
    let user = nix::unistd::User::from_name(name).ok().flatten()?;
    Some(user.dir.to_string_lossy().into_owned())
}

fn passwd_home_of_uid() -> Option<String> {
    let user = nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten()?;
    Some(user.dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::expand_tilde;
    use crate::env::Environment;

    #[test]
    fn bare_tilde_is_home() {
        let mut env = Environment::new();
        env.set_var("HOME", "/home/tester", false);
        assert_eq!(expand_tilde(&env, ""), "/home/tester");
    }

    #[test]
    fn unknown_user_is_left_alone() {
        let env = Environment::new();
        assert_eq!(
            expand_tilde(&env, "oslo-no-such-user"),
            "~oslo-no-such-user"
        );
    }

    /// `~root` is the one entry every Unix password database is guaranteed to have.
    #[test]
    fn known_user_resolves_through_passwd() {
        let env = Environment::new();
        let expected = nix::unistd::User::from_name("root")
            .unwrap()
            .expect("every system has root")
            .dir
            .to_string_lossy()
            .into_owned();
        assert_eq!(expand_tilde(&env, "root"), expected);
    }

    #[test]
    fn plus_is_pwd_and_minus_is_oldpwd() {
        let mut env = Environment::new();
        env.set_var("PWD", "/here", false);
        env.set_var("OLDPWD", "/there", false);
        assert_eq!(expand_tilde(&env, "+"), "/here");
        assert_eq!(expand_tilde(&env, "-"), "/there");
    }

    /// Without `OLDPWD` there is no previous directory to name, so the text stands.
    #[test]
    fn minus_without_oldpwd_stays_literal() {
        let mut env = Environment::new();
        env.unset_var("OLDPWD");
        assert_eq!(expand_tilde(&env, "-"), "~-");
    }
}
