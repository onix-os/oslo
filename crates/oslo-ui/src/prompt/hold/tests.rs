//! What has to be true before a prompt is redrawn over somebody's output.

use super::*;

/// **Nothing is drawn without both halves.** A pump that fired on a script, on `-c`, or before the
/// first prompt would put a prompt in the middle of output that nobody asked to interrupt — and a
/// pump with no way to build one would draw an empty block over whatever is there.
///
/// One test, because both switches are process-wide and two would race.
#[test]
fn a_prompt_is_only_redrawn_when_there_is_one_to_redraw() {
    showing(false);
    RENDER.with(|slot| slot.set(None));

    assert!(!pump(80), "nothing showing and nothing registered");

    showing(true);
    assert!(!pump(80), "showing, but no way to build a prompt");

    fn nothing() -> Option<(String, String)> {
        None
    }
    renders_with(nothing);
    assert!(!pump(80), "registered, but it declined to build one");

    showing(false);
    assert!(!pump(80), "and turning it off is enough on its own");
}

/// **The handoff is the whole of the drift.** A repaint leaves the cursor inside its own block;
/// whoever draws next starts from where the cursor is. Stopping without giving the block back
/// therefore had every following prompt drawn a row lower, however carefully each repaint was
/// placed — measured as a prompt walking eight rows down one visit to a file browser.
#[test]
fn settling_gives_the_block_back_from_its_first_row() {
    showing(true);
    assert_eq!(
        AT_ROW.with(|at| at.get()),
        0,
        "a new prompt starts at its top"
    );

    AT_ROW.with(|at| at.set(3));
    settle();
    assert_eq!(
        AT_ROW.with(|at| at.get()),
        0,
        "settling is also what forgets where it was"
    );

    // Nothing to give back, and nothing on screen to give it back to.
    showing(false);
    AT_ROW.with(|at| at.set(3));
    settle();
    assert_eq!(AT_ROW.with(|at| at.get()), 3, "not ours to move");
    showing(false);
}
