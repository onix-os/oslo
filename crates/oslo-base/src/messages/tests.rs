use super::*;

/// The buffer is process-wide, so two tests running at once would read each other's messages. This
/// is the same reason `track` and `feature` serialise their tests.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn alone() -> std::sync::MutexGuard<'static, ()> {
    let guard = match ONE_AT_A_TIME.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    clear();
    guard
}

#[test]
fn what_was_said_comes_back_in_the_order_it_was_said() {
    let _guard = alone();
    say(Level::Note, "first", "one");
    say(Level::Error, "second", "two");

    let said = all();
    assert_eq!(said.len(), 2);
    assert_eq!(said[0].text, "one");
    assert_eq!(said[1].source, "second");
    assert_eq!(said[1].level, Level::Error);
}

/// **The failure this exists to prevent.** A shell that runs for a week must not grow a buffer for a
/// week; the oldest goes, and the newest is always there.
#[test]
fn the_buffer_is_a_ring_and_keeps_the_newest() {
    let _guard = alone();
    for n in 0..KEEP + 50 {
        // Distinct text, or the repeat-collapsing below would keep one line and never fill it.
        say(Level::Note, "loop", n.to_string());
    }

    let said = all();
    assert_eq!(said.len(), KEEP);
    assert_eq!(said[0].text, "50", "the first fifty were dropped");
    assert_eq!(said[KEEP - 1].text, (KEEP + 49).to_string());
}

/// **The eviction this prevents.** A prompt segment that raises says the same thing on every draw;
/// counted, five hundred Returns cost one line, and the startup failure underneath survives.
#[test]
fn the_same_line_twice_running_is_counted_rather_than_kept_twice() {
    let _guard = alone();
    say(Level::Note, "prompt", "boom");
    for _ in 0..999 {
        say(Level::Note, "prompt", "boom");
    }
    let said = all();
    assert_eq!(said.len(), 1);
    assert_eq!(said[0].times, 1000);

    // Only *consecutive* repeats collapse: something else in between means it happened again, which
    // is a different fact from it never having stopped.
    say(Level::Note, "other", "hello");
    say(Level::Note, "prompt", "boom");
    let said = all();
    assert_eq!(said.len(), 3);
    assert_eq!(said[2].times, 1);
}

#[test]
fn a_cleared_buffer_says_nothing() {
    let _guard = alone();
    say(Level::Warn, "a", "b");
    clear();
    assert!(all().is_empty());
}

/// Elapsed time only has to be monotonic and start near zero: it answers "at startup, or just now?".
#[test]
fn the_first_message_is_near_the_start_and_time_only_goes_forward() {
    let _guard = alone();
    say(Level::Note, "a", "one");
    say(Level::Note, "a", "two");

    let said = all();
    assert!(said[0].at <= said[1].at);
    assert!(said[1].at < 60.0, "a test does not take a minute");
}
