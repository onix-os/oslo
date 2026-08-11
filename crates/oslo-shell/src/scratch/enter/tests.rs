use super::*;

/// The finder needs a terminal, so what is testable without one is the rule around it: which names
/// are allowed to become a scratch, and what `$SCRATCH` says about where we are.
#[test]
fn a_typed_name_that_is_not_a_name_never_becomes_a_tab() {
    for typed in ["", "../etc/passwd", "a/b", ".hidden", "a b"] {
        assert!(!naming::valid(typed), "{typed:?} must be refused");
    }
    for typed in ["work", "api-2", "A_b9"] {
        assert!(naming::valid(typed), "{typed:?} must be allowed");
    }
}

/// `$SCRATCH` is what a prompt reads, and the empty string has to mean "not in one" — a variable that
/// is set but empty is the shape an `export SCRATCH=` leaves behind.
#[test]
fn an_empty_tab_variable_is_not_a_tab() {
    let (_scratch, _serial) = crate::scratch::scratch();
    // SAFETY: the scratch guard serialises every test in this subtree that touches the process
    // environment, which is the only thing that makes this sound.
    unsafe { std::env::set_var(INSIDE, "") };
    assert_eq!(current(), None);
    unsafe { std::env::set_var(INSIDE, "work") };
    assert_eq!(current().as_deref(), Some("work"));
    unsafe { std::env::remove_var(INSIDE) };
    assert_eq!(current(), None);
}

/// Without a terminal the finder cannot ask, and a question nobody can answer is not an error —
/// it is a key pressed where there is nothing to press it on.
#[test]
fn no_terminal_goes_nowhere() {
    let (_scratch, _serial) = crate::scratch::scratch();
    assert_eq!(open("ctrl-\\", 0).ok(), Some(Went::Nowhere));
}
