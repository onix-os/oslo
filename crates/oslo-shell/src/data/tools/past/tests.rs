use super::*;

/// **The declaration and the rows have to agree**, which is what the column contract rests on: the
/// planner refuses a column against `COLUMNS` before anything runs, so a row carrying a name the
/// declaration omits is a column nobody can reach.
#[test]
fn the_declared_columns_are_the_ones_a_row_has() {
    let built = row(oslo_base::track::history::Command {
        line: "cargo test".into(),
        mode: "sh".into(),
        runs: 12,
        last_at: 1_551_744_000,
        dir: "/src".into(),
        places: 3,
        worked: true,
        session: "1-2".into(),
        host: "tron".into(),
        root: None,
    });
    assert_eq!(built.columns(), COLUMNS);
}

/// **`last` is a time, not a number and not a rendering.** That is what makes `sort-by last`
/// chronological and `where 'last > 2days'` arithmetic — the whole argument for the typed kinds,
/// applied to the one table the shell writes for itself.
#[test]
fn the_moment_is_a_time_and_the_counts_are_numbers() {
    let built = row(oslo_base::track::history::Command {
        line: "ls".into(),
        mode: "sh".into(),
        runs: 400,
        last_at: 1_551_744_000,
        dir: "/".into(),
        places: 9,
        worked: false,
        session: "1-2".into(),
        host: "tron".into(),
        root: None,
    });
    // Seconds in the store, nanoseconds in the kind.
    assert_eq!(
        built.get("last"),
        Some(&Val::Time(1_551_744_000_000_000_000))
    );
    assert_eq!(built.get("runs"), Some(&Val::Int(400)));
    assert_eq!(built.get("places"), Some(&Val::Int(9)));
    assert_eq!(built.get("worked"), Some(&Val::Bool(false)));
}

/// **A store that cannot be reached is an empty past, not a failure.** There is nothing to report
/// and nothing went wrong, and a producer that raised here would make `history | …` fail on a
/// machine with no `$HOME` rather than answer nothing.
///
/// `$HOME` and `$XDG_DATA_HOME` are pointed at an empty directory, because `rows` opens the store
/// itself when none is installed — that is what makes it work in a script, and it means this test
/// would otherwise read the real one.
#[test]
fn an_unreachable_store_is_no_rows_rather_than_an_error() {
    // **The environment is process-wide, so this takes a turn.** Tests run in parallel, and a test
    // that repointed `$HOME` for every other one at the same time is the shared-state flake this
    // codebase has already paid for more than once.
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());

    let empty = tempfile::tempdir().expect("tempdir");
    let (home, data) = (std::env::var("HOME"), std::env::var("XDG_DATA_HOME"));
    // SAFETY: the turn above makes this the only thread touching the environment, and both values
    // are put back before the test returns.
    unsafe {
        std::env::set_var("HOME", empty.path());
        std::env::set_var("XDG_DATA_HOME", empty.path().join("nothing-here"));
    }
    let answered = rows();
    unsafe {
        match home {
            Ok(value) => std::env::set_var("HOME", value),
            Err(_) => std::env::remove_var("HOME"),
        }
        match data {
            Ok(value) => std::env::set_var("XDG_DATA_HOME", value),
            Err(_) => std::env::remove_var("XDG_DATA_HOME"),
        }
    }
    assert!(answered.is_empty(), "an empty store answered rows");
}
