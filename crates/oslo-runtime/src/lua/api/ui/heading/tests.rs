//! The rule's width, which is the only part with arithmetic in it.

use super::*;

#[test]
fn the_rule_matches_the_title_in_cells_not_bytes() {
    // Four bytes, two cells: a rule cut to `#text` would be twice too long.
    assert_eq!(rule_width("日本", None), 4);
    assert_eq!(rule_width("abcd", None), 4);
}

#[test]
fn an_escape_costs_no_cells() {
    assert_eq!(rule_width("\x1b[31mred\x1b[0m", None), 3);
}

#[test]
fn a_rule_is_never_narrower_than_one_cell() {
    assert_eq!(rule_width("", None), 1);
}
