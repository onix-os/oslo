//! A light sweeping left and right, the way the car in Knight Rider did.
//!
//! Ported from hexe's `knight_rider.zig`, which oslo's author also wrote — same four-phase cycle,
//! same draining tail, same fade by distance.
//!
//! # Time, not frames
//!
//! There is no frame table and no state. The frame is computed from how long the thing has been
//! going, so nothing has to be ticked and nothing can drift:
//!
//! ```text
//! frame = elapsed_ms / step_ms  %  cycle
//! cycle = width + hold + (width - 1) + hold
//! ```
//!
//! That is what makes it free to draw from a redraw loop that already exists: ask what the frame
//! is now, draw it, forget it.
//!
//! # The tail is the trick
//!
//! Distance is measured **in the direction of travel**, so the tail is behind the head on both
//! passes rather than flipping to the wrong side halfway. And during a hold the hold's progress is
//! *added* to that distance, so the tail keeps sliding off the end and drains away instead of
//! freezing while the head waits — which is what stops the turn looking like a stall.

use super::theme::{Color, Depth, Style};

/// The lit head and the unlit track.
const HEAD: char = '■';
const TRACK: char = '⬝';

/// How many cells behind the head still glow.
const TRAIL: i32 = 6;

/// The palette runs from this index at the head, downward as the tail fades.
const BRIGHTEST: u8 = 243;
/// What an unlit cell is drawn in, so the track stays visible.
const UNLIT: u8 = 240;

/// How the scanner is drawn.
#[derive(Debug, Clone, Copy)]
pub struct Scanner {
    /// Cells wide. Clamped to 2..=32, as the original does.
    pub width: u8,
    /// Milliseconds per frame.
    pub step_ms: u64,
    /// Frames the head pauses at each end while its tail drains.
    pub hold: u8,
}

impl Default for Scanner {
    fn default() -> Self {
        // hexe's defaults, which are tuned to read as motion without being distracting.
        Scanner {
            width: 8,
            step_ms: 75,
            hold: 9,
        }
    }
}

/// Where the head is, and what it is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sweep {
    at: i32,
    forward: bool,
    holding: bool,
    held_for: i32,
}

impl Scanner {
    fn cells(self) -> i32 {
        i32::from(self.width.clamp(2, 32))
    }

    fn hold_frames(self) -> i32 {
        i32::from(self.hold.min(60))
    }

    /// Frames in one full there-and-back cycle.
    pub fn cycle(self) -> u64 {
        let (w, hold) = (self.cells(), self.hold_frames());
        (w + hold + (w - 1) + hold) as u64
    }

    /// Which frame of the cycle `elapsed_ms` lands on.
    pub fn frame_at(self, elapsed_ms: u64) -> u64 {
        let step = self.step_ms.max(1);
        (elapsed_ms / step) % self.cycle()
    }

    fn sweep(self, frame: u64) -> Sweep {
        let (w, hold) = (self.cells(), self.hold_frames());
        let frame = frame as i32;
        let end_hold = w + hold;
        let back_end = end_hold + w - 1;
        if frame < w {
            Sweep {
                at: frame,
                forward: true,
                holding: false,
                held_for: 0,
            }
        } else if frame < end_hold {
            Sweep {
                at: w - 1,
                forward: true,
                holding: true,
                held_for: frame - w,
            }
        } else if frame < back_end {
            Sweep {
                at: w - 2 - (frame - end_hold),
                forward: false,
                holding: false,
                held_for: 0,
            }
        } else {
            Sweep {
                at: 0,
                forward: false,
                holding: true,
                held_for: frame - back_end,
            }
        }
    }

    /// How far cell `cell` is behind the head, or `None` when it is dark.
    fn trail_at(self, cell: i32, sweep: Sweep) -> Option<i32> {
        // **In the direction of travel.** Measuring absolute distance would put the tail ahead of
        // the head on the way back.
        let behind = if sweep.forward {
            sweep.at - cell
        } else {
            cell - sweep.at
        };
        if sweep.holding {
            // The hold pushes the whole tail further behind, so it drains off the end rather than
            // standing still while the head waits.
            let drained = behind + sweep.held_for;
            return (0..TRAIL).contains(&drained).then_some(drained);
        }
        (0..TRAIL).contains(&behind).then_some(behind)
    }

    /// The frame at `elapsed_ms`, painted.
    pub fn render(self, elapsed_ms: u64, depth: Depth) -> String {
        let sweep = self.sweep(self.frame_at(elapsed_ms));
        let mut out = String::new();
        for cell in 0..self.cells() {
            let (glyph, colour) = match self.trail_at(cell, sweep) {
                // Brightest at the head, fading with distance, floored so the far end of the tail
                // stays distinguishable from the track.
                Some(behind) => (HEAD, BRIGHTEST - behind.min(7) as u8),
                None => (TRACK, UNLIT),
            };
            out.push_str(&Style::fg(Color::Indexed(colour)).paint(&glyph.to_string(), depth));
        }
        out
    }

    /// The frame as plain text, for tests and for anything that measures it.
    pub fn plain(self, elapsed_ms: u64) -> String {
        let sweep = self.sweep(self.frame_at(elapsed_ms));
        (0..self.cells())
            .map(|cell| {
                if self.trail_at(cell, sweep).is_some() {
                    HEAD
                } else {
                    TRACK
                }
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "scanner/tests.rs"]
mod tests;
