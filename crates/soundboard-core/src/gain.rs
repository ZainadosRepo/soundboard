//! Gains, fades and the master volume. Every live fade and volume change is linear in gain.

use crate::Seconds;
use serde::{Deserialize, Serialize};

/// A gain that moves linearly from its current value to a target over a duration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearRamp {
    value: f32,
    target: f32,
    origin: f32,
    duration: Seconds,
    progress: f64,
}

impl LinearRamp {
    /// Duration used for every volume change (A/B switch, slider, mute, master).
    pub const VOLUME_CHANGE: Seconds = 0.2;

    pub fn new(value: f32) -> Self {
        LinearRamp {
            value,
            target: value,
            origin: value,
            duration: 0.0,
            progress: 1.0,
        }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn is_finished(&self) -> bool {
        self.progress >= 1.0
    }

    /// Starts moving from the current value, so retargeting mid-ramp never jumps.
    pub fn move_to(&mut self, target: f32, over: Seconds) {
        self.origin = self.value;
        self.target = target;
        self.duration = over;
        if over <= 0.0 {
            self.value = target;
            self.progress = 1.0;
        } else {
            self.progress = 0.0;
        }
    }

    pub fn advance(&mut self, elapsed: Seconds) -> f32 {
        if self.progress >= 1.0 {
            return self.value;
        }
        self.progress = (self.progress + elapsed / self.duration).clamp(0.0, 1.0);
        self.value = if self.progress >= 1.0 {
            self.target
        } else {
            self.origin + (self.target - self.origin) * self.progress as f32
        };
        self.value
    }
}

/// Values this close to 0 or 1 are treated as exactly 0 or 1.
pub const SNAP_TOLERANCE: f32 = 1e-5;

pub fn snapped(gain: f32) -> f32 {
    if gain.abs() < SNAP_TOLERANCE {
        0.0
    } else if (gain - 1.0).abs() < SNAP_TOLERANCE {
        1.0
    } else {
        gain
    }
}

/// Per-tile fade envelope `elapsed` seconds after the sound started, in a trimmed region of
/// `length` seconds. Each fade is limited to half the region so they never overlap.
/// A looping sound fades in on its first pass only and never fades out on its own: it
/// would otherwise dip to silence at every loop.
pub fn envelope(
    elapsed: Seconds,
    length: Seconds,
    fade_in: Seconds,
    fade_out: Seconds,
    loops: bool,
) -> f32 {
    if length <= 0.0 {
        return 1.0;
    }
    if loops {
        return if elapsed < length {
            envelope(elapsed, length, fade_in, 0.0, false)
        } else {
            1.0
        };
    }
    let half = length / 2.0;
    let fade_in = fade_in.clamp(0.0, half);
    let fade_out = fade_out.clamp(0.0, half);
    let mut gain: f64 = 1.0;
    if fade_in > 0.0 && elapsed < fade_in {
        gain = gain.min(elapsed.max(0.0) / fade_in);
    }
    let remaining = length - elapsed;
    if fade_out > 0.0 && remaining < fade_out {
        gain = gain.min(remaining.max(0.0) / fade_out);
    }
    gain as f32
}

/// App-wide output level with two presets, A and B, like each tile's own volumes.
/// Switching between them dims or restores the whole board at once.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MasterVolume {
    pub volume_a: f32,
    pub volume_b: f32,
    /// When true the board plays at `volume_b`.
    pub uses_volume_b: bool,
    pub muted: bool,
}

impl Default for MasterVolume {
    fn default() -> Self {
        MasterVolume {
            volume_a: 1.0,
            volume_b: 0.4,
            uses_volume_b: false,
            muted: false,
        }
    }
}

impl MasterVolume {
    /// Change applied by Master Volume Up / Down.
    pub const STEP: f32 = 0.05;

    /// The preset currently in use; editing it edits A or B.
    pub fn active_volume(&self) -> f32 {
        if self.uses_volume_b {
            self.volume_b
        } else {
            self.volume_a
        }
    }

    pub fn set_active_volume(&mut self, value: f32) {
        let clamped = value.clamp(0.0, 1.0);
        if self.uses_volume_b {
            self.volume_b = clamped;
        } else {
            self.volume_a = clamped;
        }
    }

    /// Gain applied to every sound.
    pub fn gain(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.active_volume()
        }
    }

    pub fn toggle_ab(&mut self) {
        self.uses_volume_b = !self.uses_volume_b;
    }

    pub fn step_up(&mut self) {
        self.set_active_volume(self.active_volume() + Self::STEP);
    }

    pub fn step_down(&mut self) {
        self.set_active_volume(self.active_volume() - Self::STEP);
    }
}

/// Moving the playback of a sounding tile (Scan Backward / Forward in the editor).
pub mod seek {
    use crate::Seconds;

    /// Step of ← / → in the editor; ⇧ uses the fine step.
    pub const SCAN_STEP: Seconds = 5.0;
    pub const FINE_SCAN_STEP: Seconds = 1.0;
    /// A sound that isn't looping stops this close to its end rather than at it, so a scan
    /// forward still lets the end be heard.
    pub const END_MARGIN: Seconds = 0.25;

    /// Where playback goes, in seconds into the trimmed region of `length` seconds.
    /// Looping sounds wrap around; other sounds stay between the start and just before the end.
    pub fn target(requested: Seconds, length: Seconds, loops: bool) -> Seconds {
        if length <= 0.0 {
            return 0.0;
        }
        if loops {
            return requested.rem_euclid(length);
        }
        requested.clamp(0.0, (length - END_MARGIN).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_moves_linearly_and_retargets_without_jumping() {
        let mut r = LinearRamp::new(0.0);
        r.move_to(1.0, 1.0);
        assert_eq!(r.advance(0.25), 0.25);
        r.move_to(0.0, 0.5);
        assert_eq!(r.value(), 0.25);
        assert_eq!(r.advance(0.25), 0.125);
        assert_eq!(r.advance(10.0), 0.0);
        assert!(r.is_finished());
    }

    #[test]
    fn zero_duration_ramps_jump() {
        let mut r = LinearRamp::new(1.0);
        r.move_to(0.3, 0.0);
        assert_eq!(r.value(), 0.3);
        assert!(r.is_finished());
    }

    #[test]
    fn snapping() {
        assert_eq!(snapped(0.000001), 0.0);
        assert_eq!(snapped(0.999999), 1.0);
        assert_eq!(snapped(0.5), 0.5);
    }

    #[test]
    fn envelope_fades_are_limited_to_half_the_region() {
        assert_eq!(envelope(0.5, 10.0, 1.0, 0.0, false), 0.5);
        assert_eq!(envelope(9.5, 10.0, 0.0, 1.0, false), 0.5);
        assert_eq!(envelope(2.0, 4.0, 10.0, 0.0, false), 1.0);
        assert_eq!(envelope(1.0, 0.0, 1.0, 1.0, false), 1.0);
    }

    #[test]
    fn looping_sounds_fade_in_once_and_never_out() {
        assert_eq!(envelope(0.5, 10.0, 1.0, 1.0, true), 0.5);
        assert_eq!(envelope(9.9, 10.0, 1.0, 1.0, true), 1.0);
        assert_eq!(envelope(10.5, 10.0, 1.0, 1.0, true), 1.0);
    }

    #[test]
    fn master_volume_edits_the_active_preset() {
        let mut m = MasterVolume::default();
        m.toggle_ab();
        m.step_up();
        assert!((m.volume_b - 0.45).abs() < 1e-6);
        assert_eq!(m.volume_a, 1.0);
        m.toggle_ab();
        m.step_up();
        assert_eq!(m.volume_a, 1.0, "clamped at 1");
        m.muted = true;
        assert_eq!(m.gain(), 0.0);
    }

    #[test]
    fn seek_wraps_loops_and_keeps_the_end_audible() {
        assert_eq!(seek::target(-1.0, 10.0, true), 9.0);
        assert_eq!(seek::target(12.0, 10.0, true), 2.0);
        assert_eq!(seek::target(12.0, 10.0, false), 9.75);
        assert_eq!(seek::target(-3.0, 10.0, false), 0.0);
        assert_eq!(seek::target(3.0, 0.0, false), 0.0);
    }
}
