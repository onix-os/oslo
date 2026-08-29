//! Frames a prompt tool hands over all at once, and the clock that plays them.
//!
//! A tool asked for one frame per draw is a process per frame: at ten frames a second that is ten
//! spawns a second, for as long as the shell is open, to turn one glyph. Asked for a *filmstrip* it
//! is one spawn, and the playing is arithmetic — which is what this file is.
//!
//! # The horizon is what keeps it honest
//!
//! Frames are drawn ahead of time, so everything they show is a guess about the future: a branch
//! may change, a command may fail. The tool is told how far ahead to draw and the strip is thrown
//! away when that long has passed, so a wrong guess lasts one horizon rather than the session.
//! Inside it the strip repeats, because a cycle shorter than the horizon is an animation and not a
//! one-shot.
//!
//! # A tool that does not speak it is not broken by it
//!
//! Anything that is not a filmstrip is the prompt itself, exactly as before — see [`absorb`].
//! starship and hexe print a prompt and always will.

use super::remember;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Frames in hand for one prompt, and when playback started.
pub(super) struct Strip {
    /// Each frame and how long it holds. A frame with no cadence of its own ends the strip.
    frames: Vec<(String, Duration)>,
    began: Instant,
    /// How far ahead the tool was asked to draw. Past this the data underneath — a branch, a
    /// status — could have moved, so the strip is re-asked for rather than looped forever.
    horizon: Duration,
}

impl Strip {
    /// The frame due now, or `None` once the horizon has passed.
    fn due(&self) -> Option<&str> {
        let elapsed = self.began.elapsed();
        if elapsed >= self.horizon || self.frames.is_empty() {
            return None;
        }
        let cycle: Duration = self.frames.iter().map(|(_, hold)| *hold).sum();
        if cycle.is_zero() {
            return self.frames.first().map(|(text, _)| text.as_str());
        }
        // Wrapped, so a cycle shorter than the horizon repeats rather than freezing on its last
        // picture -- the animation is what the caller asked for.
        let mut at = Duration::from_nanos((elapsed.as_nanos() % cycle.as_nanos()) as u64);
        for (text, hold) in &self.frames {
            if at < *hold {
                return Some(text);
            }
            at -= *hold;
        }
        self.frames.last().map(|(text, _)| text.as_str())
    }
}

/// A filmstrip reply: `{"frames":[{"text":…,"next_frame_ms":N}, …]}`.
///
/// Anything else — a tool that ignored the horizon and printed a prompt, or printed nothing
/// parseable — is `None`, and the caller treats what it printed as the prompt itself. That is what
/// keeps `frames` from breaking a tool that turns out not to speak it.
pub(super) fn parse_strip(out: &str, horizon: Duration) -> Option<Strip> {
    let parsed: serde_json::Value = serde_json::from_str(out.trim()).ok()?;
    let list = parsed.get("frames")?.as_array()?;
    let mut frames = Vec::with_capacity(list.len());
    for frame in list {
        let text = frame.get("text")?.as_str()?.to_string();
        let hold = frame
            .get("next_frame_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        frames.push((text, Duration::from_millis(hold)));
    }
    (!frames.is_empty()).then_some(Strip {
        frames,
        began: Instant::now(),
        horizon,
    })
}

/// Take what the tool printed: a filmstrip becomes frames to play and the first of them is drawn;
/// anything else is the prompt itself, exactly as before.
pub(super) fn absorb(key: &str, out: String, horizon: Option<Duration>) -> String {
    if let Some(horizon) = horizon
        && let Some(strip) = parse_strip(&out, horizon)
    {
        let text = strip.due().unwrap_or_default().to_string();
        if let Ok(mut held) = strips().lock() {
            held.insert(key.to_string(), strip);
        }
        // Remembered too, so the fallbacks that reach for the last good answer -- a run that
        // overran, a tool that died -- find a prompt rather than a page of JSON.
        remember(key, text.clone());
        return text;
    }
    remember(key, out.clone());
    out
}

/// Strips being played, one per prompt.
pub(super) fn strips() -> &'static Mutex<HashMap<String, Strip>> {
    static STRIPS: OnceLock<Mutex<HashMap<String, Strip>>> = OnceLock::new();
    STRIPS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The frame due now from this prompt's strip, if one is still playing.
pub(super) fn playing(key: &str) -> Option<String> {
    let held = strips().lock().ok()?;
    held.get(key)?.due().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A filmstrip is a list of pictures with holds; anything else is a prompt.
    #[test]
    fn a_filmstrip_is_recognised_and_anything_else_is_not() {
        let strip = parse_strip(
            r#"{"frames":[{"text":"a","next_frame_ms":100},{"text":"b","next_frame_ms":100}]}"#,
            Duration::from_millis(1000),
        )
        .expect("two frames");
        assert_eq!(strip.frames.len(), 2);
        assert_eq!(strip.frames[0].1, Duration::from_millis(100));

        // A tool that ignored the horizon printed its prompt: that is not a strip, and must be drawn
        // as-is rather than swallowed.
        assert!(parse_strip("\u{1b}[32m~/src\u{1b}[0m", Duration::from_millis(1000)).is_none());
        assert!(parse_strip("{\"frames\":[]}", Duration::from_millis(1000)).is_none());
    }

    /// Playback walks the holds, wraps at the end of the cycle, and stops at the horizon.
    #[test]
    fn playback_wraps_within_the_cycle_and_stops_at_the_horizon() {
        let frames = vec![
            ("a".to_string(), Duration::from_millis(100)),
            ("b".to_string(), Duration::from_millis(100)),
        ];
        // Begun far enough back to land in the second frame of the second lap: 250ms into a 200ms
        // cycle is 50ms in, which is "a" -- the wrap is the animation continuing.
        let strip = Strip {
            frames: frames.clone(),
            began: Instant::now() - Duration::from_millis(250),
            horizon: Duration::from_millis(1000),
        };
        assert_eq!(
            strip.due(),
            Some("a"),
            "250ms into a 200ms cycle is 50ms in"
        );

        let mid = Strip {
            frames: frames.clone(),
            began: Instant::now() - Duration::from_millis(150),
            horizon: Duration::from_millis(1000),
        };
        assert_eq!(mid.due(), Some("b"), "150ms in is the second frame");

        // Past the horizon the data underneath could have moved, so the strip is spent and the tool is
        // asked again rather than looping on pictures that may no longer be true.
        let spent = Strip {
            frames,
            began: Instant::now() - Duration::from_millis(1200),
            horizon: Duration::from_millis(1000),
        };
        assert_eq!(spent.due(), None, "past its horizon a strip is spent");
    }
}
