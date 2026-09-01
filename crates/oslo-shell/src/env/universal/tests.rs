use super::*;
use crate::env::scope::Environment;

/// One store per test, named after it, in a directory nothing else touches.
///
/// `OSLO_UNIVERSAL` is process-wide and the snapshot is thread-local, so the tests are serialised
/// rather than allowed to take turns badly: two of them pointing at two files through one variable
/// is a race with no correct outcome.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Store {
    path: PathBuf,
    _held: std::sync::MutexGuard<'static, ()>,
}

impl Store {
    fn new(name: &str) -> Self {
        let held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let path = std::env::temp_dir().join(format!(
            "oslo-universal-{}-{name}/universal",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
        // SAFETY: the mutex above is what makes this the only test setting it.
        unsafe { std::env::set_var("OSLO_UNIVERSAL", &path) };
        forget();
        Self { path, _held: held }
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap_or_default()
    }

    /// What another session would see: a fresh read of the same file.
    fn as_another_session(&self) {
        forget();
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().expect("a parent"));
        unsafe { std::env::remove_var("OSLO_UNIVERSAL") };
        forget();
    }
}

#[test]
fn a_value_survives_being_written_and_read_back() {
    let store = Store::new("round-trip");
    set("theme", "dark").expect("writable");
    assert_eq!(get("theme").as_deref(), Some("dark"));
    store.as_another_session();
    assert_eq!(get("theme").as_deref(), Some("dark"));
    assert!(store.text().starts_with(HEADING));
}

/// A newline in a value would be a second entry, and a tab a second field.
#[test]
fn a_value_with_anything_in_it_comes_back_the_same() {
    let _store = Store::new("escapes");
    let awkward = "one\ttwo\nthree\\four\rfive";
    set("x", awkward).expect("writable");
    forget();
    assert_eq!(get("x").as_deref(), Some(awkward));
}

/// The writer reads the file rather than its own snapshot, so a value another session added a
/// moment ago is carried forward instead of dropped.
#[test]
fn a_write_keeps_what_another_session_had_added() {
    let store = Store::new("carry-forward");
    set("a", "1").expect("writable");

    // Another session, writing straight into the file behind this one's back.
    let mut theirs = BTreeMap::new();
    theirs.insert("a".to_string(), "1".to_string());
    theirs.insert("b".to_string(), "2".to_string());
    write(&store.path, &theirs).expect("writable");

    set("c", "3").expect("writable");
    let now = all();
    assert_eq!(now.get("a").map(String::as_str), Some("1"));
    assert_eq!(now.get("b").map(String::as_str), Some("2"), "b was dropped");
    assert_eq!(now.get("c").map(String::as_str), Some("3"));
}

#[test]
fn erasing_says_whether_there_was_anything_to_erase() {
    let _store = Store::new("erase");
    set("gone", "soon").expect("writable");
    assert_eq!(erase("gone"), Ok(true));
    assert_eq!(get("gone"), None);
    assert_eq!(erase("gone"), Ok(false));
}

/// **A store that will not parse must never look like a store that was emptied.** The one failure
/// in this feature that would be silent and expensive.
#[test]
fn a_corrupt_file_leaves_the_session_alone() {
    let store = Store::new("corrupt");
    set("kept", "yes").expect("writable");
    assert_eq!(
        get("kept").as_deref(),
        Some("yes"),
        "read once, before the harm"
    );

    std::fs::write(&store.path, "this is not the format\n").expect("writable");
    assert_eq!(
        get("kept").as_deref(),
        Some("yes"),
        "an unreadable store emptied the snapshot"
    );

    // Half of it parsing is not half of it being applied.
    std::fs::write(&store.path, "a\t1\nnot a line at all\n").expect("writable");
    let now = all();
    assert_eq!(now.get("kept").map(String::as_str), Some("yes"));
    assert!(!now.contains_key("a"), "half a file was applied");
}

/// A file that is genuinely gone is genuinely empty — the other side of the rule above.
#[test]
fn a_file_that_is_removed_empties_the_store() {
    let store = Store::new("removed");
    set("x", "1").expect("writable");
    std::fs::remove_file(&store.path).expect("removable");
    assert_eq!(get("x"), None);
}

#[test]
fn a_sync_hands_a_session_everything_the_first_time() {
    let _store = Store::new("sync-first");
    set("a", "1").expect("writable");
    set("b", "2").expect("writable");
    forget();

    let mut env = Environment::new();
    let changes = sync_into(&mut env);
    assert_eq!(changes.len(), 2);
    assert_eq!(env.get_var("a"), Some("1"));
    assert_eq!(env.get_var("b"), Some("2"));

    // Nothing moved, so nothing is reported and nothing is written.
    assert!(sync_into(&mut env).is_empty());
}

/// **Only what changed.** A universal variable becomes an ordinary shell variable, and a sync that
/// rewrote all of them would undo an assignment typed a second ago.
#[test]
fn a_sync_leaves_a_local_assignment_alone() {
    let store = Store::new("sync-local");
    set("shared", "one").expect("writable");
    forget();

    let mut env = Environment::new();
    sync_into(&mut env);
    env.set_var("shared", "mine", false);

    assert!(sync_into(&mut env).is_empty());
    assert_eq!(env.get_var("shared"), Some("mine"));

    // Until the store itself moves, which is the store winning on purpose.
    let mut theirs = BTreeMap::new();
    theirs.insert("shared".to_string(), "two".to_string());
    write(&store.path, &theirs).expect("writable");
    let changes = sync_into(&mut env);
    assert_eq!(changes.len(), 1);
    assert_eq!(env.get_var("shared"), Some("two"));
}

/// Erased in one session, gone from the next one's variables.
#[test]
fn a_sync_takes_away_what_was_erased() {
    let store = Store::new("sync-erase");
    set("temporary", "1").expect("writable");
    forget();

    let mut env = Environment::new();
    sync_into(&mut env);
    assert_eq!(env.get_var("temporary"), Some("1"));

    // Erased by another session: this one finds out by re-reading, which is the whole point.
    write(&store.path, &BTreeMap::new()).expect("writable");
    let changes = sync_into(&mut env);
    assert_eq!(changes, [Change::Erased("temporary".to_string())]);
    assert_eq!(env.get_var("temporary"), None);
}

/// A name no shell could expand would be a value nobody could reach.
#[test]
fn only_a_name_a_shell_can_read_is_stored() {
    let store = Store::new("names");
    // Written straight in, because `set -U` refuses it before it could get here.
    std::fs::create_dir_all(store.path.parent().expect("a parent")).expect("writable");
    std::fs::write(&store.path, "not a name\tx\n").expect("writable");
    assert!(all().is_empty(), "an unusable name was read back");
}
