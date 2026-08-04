//! The sweep, frame by frame.
//!
//! Checked against the same cycle hexe's `knight_rider.zig` produces, because the point of a port
//! is that it looks the same — an animation that is subtly different is a rewrite wearing a
//! port's name.

use super::*;

fn scanner() -> Scanner {
    Scanner {
        width: 8,
        step_ms: 75,
        hold: 9,
    }
}

/// Frame `n` of the default cycle, as plain text.
fn at(frame: u64) -> String {
    scanner().plain(frame * 75)
}

/// Four phases: out, hold, back, hold.
#[test]
fn the_cycle_is_out_hold_back_hold() {
    // width + hold + (width - 1) + hold = 8 + 9 + 7 + 9
    assert_eq!(scanner().cycle(), 33);
}

/// The head walks out from the left, growing its tail behind it.
#[test]
fn the_head_sweeps_out_from_the_left() {
    assert_eq!(at(0), "■⬝⬝⬝⬝⬝⬝⬝");
    assert_eq!(at(1), "■■⬝⬝⬝⬝⬝⬝");
    assert_eq!(at(3), "■■■■⬝⬝⬝⬝");
    // Six is the trail length, so from here the far end starts going dark.
    assert_eq!(at(6), "⬝■■■■■■⬝");
    assert_eq!(at(7), "⬝⬝■■■■■■");
}

/// **The tail drains during the hold** rather than freezing — which is what stops the turn at each
/// end from looking like the animation has stalled.
#[test]
fn the_tail_drains_while_the_head_holds() {
    assert_eq!(at(8), "⬝⬝■■■■■■", "the hold begins");
    assert_eq!(at(11), "⬝⬝⬝⬝⬝■■■", "still draining");
    assert_eq!(at(13), "⬝⬝⬝⬝⬝⬝⬝■");
    assert_eq!(at(14), "⬝⬝⬝⬝⬝⬝⬝⬝", "dark before it comes back");
}

/// Coming back, the tail is on the other side of the head — the reason distance is measured in the
/// direction of travel rather than as an absolute.
#[test]
fn the_tail_follows_on_the_way_back() {
    assert_eq!(at(17), "⬝⬝⬝⬝⬝⬝■■");
    assert_eq!(at(20), "⬝⬝⬝■■■■■");
    assert_eq!(at(23), "■■■■■■⬝⬝");
}

/// And drains again at the left end, leaving the cycle where it started.
#[test]
fn the_cycle_returns_to_the_start() {
    assert_eq!(at(26), "■■■■⬝⬝⬝⬝");
    assert_eq!(at(29), "■⬝⬝⬝⬝⬝⬝⬝");
    assert_eq!(at(32), "⬝⬝⬝⬝⬝⬝⬝⬝");
    assert_eq!(at(33), at(0), "the cycle wraps");
}

/// Time drives it, so the frame is a function of elapsed milliseconds and nothing else. Nothing to
/// tick, nothing to drift.
#[test]
fn the_frame_comes_from_the_clock() {
    let s = scanner();
    assert_eq!(s.frame_at(0), 0);
    assert_eq!(s.frame_at(74), 0, "still within the first step");
    assert_eq!(s.frame_at(75), 1);
    assert_eq!(
        s.frame_at(75 * 33),
        0,
        "a whole cycle later, back to the start"
    );
    // A long-running command is no different from a short one: the cycle simply repeats.
    assert_eq!(s.frame_at(75 * 33 * 1000 + 150), 2);
}

/// Every frame is exactly `width` cells, or it would jitter the row it sits in.
#[test]
fn every_frame_is_the_same_width() {
    for frame in 0..scanner().cycle() {
        assert_eq!(
            at(frame).chars().count(),
            8,
            "frame {frame} is the wrong width"
        );
    }
}

/// The width is clamped rather than trusted: a zero would divide the cycle by nothing, and a huge
/// one would draw off the side of the terminal.
#[test]
fn the_width_is_clamped() {
    let tiny = Scanner {
        width: 0,
        ..scanner()
    };
    assert_eq!(tiny.plain(0).chars().count(), 2);
    let huge = Scanner {
        width: 200,
        ..scanner()
    };
    assert_eq!(huge.plain(0).chars().count(), 32);
}

/// A zero step would divide by zero. It is treated as one millisecond.
#[test]
fn a_zero_step_does_not_divide_by_zero() {
    let s = Scanner {
        step_ms: 0,
        ..scanner()
    };
    assert_eq!(s.frame_at(0), 0);
    assert_eq!(s.frame_at(5), 5);
}

/// The head is the brightest cell and the tail fades behind it.
#[test]
fn the_head_is_brighter_than_its_tail() {
    let painted = scanner().render(75 * 5, Depth::Ansi256);
    // Frame 5 is `■■■■■■⬝⬝`: the head is at cell 5, so 243 is present and so are dimmer steps.
    assert!(painted.contains("38;5;243"), "no head colour: {painted:?}");
    assert!(painted.contains("38;5;238"), "no faded tail: {painted:?}");
    assert!(painted.contains("38;5;240"), "no unlit track: {painted:?}");
}
