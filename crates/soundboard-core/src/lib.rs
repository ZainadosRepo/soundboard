//! Everything Soundboard decides without touching audio hardware or the UI: sets and their
//! files, playback rules, fades, MIDI, OSC and `soundboard://` links.

pub mod gain;
pub mod hotkey;
pub mod library;
pub mod midi;
pub mod model;
pub mod osc;
pub mod playback;
pub mod smart;
pub mod store;
pub mod text;
pub mod url_command;
pub mod waveform;

pub use uuid::Uuid;

/// Seconds, as every duration in the model.
pub type Seconds = f64;
