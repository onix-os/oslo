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
