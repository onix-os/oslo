//! Applying what the config set.
//!
//! The theme and the settings are *read out of the interpreter* once the config file has run,
//! rather than pushed in as it runs. A config is a script — it can set a colour, compute another,
//! and change its mind — and reading the final state once is the only way to see what it decided
//! rather than what it passed through on the way.

use oslo::LuaEngine;

/// Read `oslo.theme` and the rest of the settings out of the interpreter and install them.
///
/// Complaints are printed rather than swallowed: an element that quietly keeps its default looks
/// exactly like oslo ignoring the config, which is the hardest kind of mistake to find.
pub fn apply(lua: &LuaEngine) {
    let (theme, problems) = lua.read_theme();
    for problem in problems {
        eprintln!("oslo: {problem}");
    }
    oslo::interactive::theme::install(theme);

    let (settings, problems) = lua.read_settings();
    for problem in problems {
        eprintln!("oslo: {problem}");
    }
    oslo::interactive::settings::install(settings);
}
