//! Controller notes that trigger tiles, and pad lights sent back to the controller.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A note on one MIDI channel. Ordered by channel then note, so messages come out sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MidiNote {
    /// 1…16, as shown on controllers.
    pub channel: u8,
    /// 0…127; 60 is middle C (C3 in the Yamaha naming most controllers use).
    pub note: u8,
}

impl MidiNote {
    pub fn new(channel: u8, note: u8) -> Self {
        MidiNote { channel, note }
    }

    /// Note name with octave, e.g. "C3" for note 60.
    pub fn name(&self) -> String {
        const NAMES: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        format!(
            "{}{}",
            NAMES[usize::from(self.note % 12)],
            i32::from(self.note / 12) - 2
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMessage {
    /// Velocity 1…127; a note-on with velocity 0 is reported as a note-off.
    NoteOn(MidiNote, u8),
    NoteOff(MidiNote),
}

impl MidiMessage {
    /// Decodes a MIDI 1.0 channel voice message. Everything else (controllers, clock, system
    /// messages) gives `None`.
    pub fn parse(bytes: &[u8]) -> Option<MidiMessage> {
        let [status, note, velocity, ..] = *bytes else {
            return None;
        };
        let note = MidiNote::new((status & 0x0F) + 1, note & 0x7F);
        let velocity = velocity & 0x7F;
        match status >> 4 {
            0x9 if velocity > 0 => Some(MidiMessage::NoteOn(note, velocity)),
            0x9 | 0x8 => Some(MidiMessage::NoteOff(note)),
            _ => None,
        }
    }

    pub fn bytes(&self) -> [u8; 3] {
        let (status, note, velocity) = match *self {
            MidiMessage::NoteOn(note, velocity) => (0x90, note, velocity),
            MidiMessage::NoteOff(note) => (0x80, note, 0),
        };
        [
            status | (note.channel.saturating_sub(1) & 0x0F),
            note.note & 0x7F,
            velocity & 0x7F,
        ]
    }

    /// Velocity as a gain factor: full velocity plays at the tile's own volume.
    pub fn gain_for_velocity(velocity: u8) -> f32 {
        f32::from(velocity.min(127)) / 127.0
    }
}

/// Which controller pads are lit, so only changes are sent: a pad lights while its tile plays.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MidiLights {
    lit: BTreeSet<MidiNote>,
}

impl MidiLights {
    /// Messages turning off pads no longer wanted and lighting new ones, with `velocity`
    /// (which picks the colour on many pad controllers). Updates what is recorded as lit.
    pub fn update(&mut self, wanted: &BTreeSet<MidiNote>, velocity: u8) -> Vec<MidiMessage> {
        let velocity = velocity.clamp(1, 127);
        let off = self
            .lit
            .difference(wanted)
            .map(|n| MidiMessage::NoteOff(*n));
        let on = wanted
            .difference(&self.lit)
            .map(|n| MidiMessage::NoteOn(*n, velocity));
        let messages = off.chain(on).collect();
        self.lit = wanted.clone();
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_follow_the_c3_convention() {
        assert_eq!(MidiNote::new(1, 60).name(), "C3");
        assert_eq!(MidiNote::new(1, 0).name(), "C-2");
        assert_eq!(MidiNote::new(1, 61).name(), "C#3");
    }

    #[test]
    fn parses_note_on_and_off() {
        assert_eq!(
            MidiMessage::parse(&[0x92, 60, 100]),
            Some(MidiMessage::NoteOn(MidiNote::new(3, 60), 100))
        );
        assert_eq!(
            MidiMessage::parse(&[0x92, 60, 0]),
            Some(MidiMessage::NoteOff(MidiNote::new(3, 60)))
        );
        assert_eq!(
            MidiMessage::parse(&[0x80, 1, 64]),
            Some(MidiMessage::NoteOff(MidiNote::new(1, 1)))
        );
        assert_eq!(
            MidiMessage::parse(&[0xB0, 7, 100]),
            None,
            "controllers are ignored"
        );
        assert_eq!(MidiMessage::parse(&[0xF8]), None);
    }

    #[test]
    fn round_trips_to_bytes() {
        let m = MidiMessage::NoteOn(MidiNote::new(16, 127), 1);
        assert_eq!(MidiMessage::parse(&m.bytes()), Some(m));
    }

    #[test]
    fn velocity_gain() {
        assert_eq!(MidiMessage::gain_for_velocity(127), 1.0);
        assert_eq!(MidiMessage::gain_for_velocity(0), 0.0);
    }

    #[test]
    fn lights_only_send_changes() {
        let mut lights = MidiLights::default();
        let (a, b) = (MidiNote::new(1, 36), MidiNote::new(1, 37));
        assert_eq!(
            lights.update(&BTreeSet::from([a]), 200),
            [MidiMessage::NoteOn(a, 127)]
        );
        assert!(lights.update(&BTreeSet::from([a]), 127).is_empty());
        assert_eq!(
            lights.update(&BTreeSet::from([b]), 5),
            [MidiMessage::NoteOff(a), MidiMessage::NoteOn(b, 5)]
        );
    }
}
