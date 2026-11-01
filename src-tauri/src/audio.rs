//! Plays tiles: one voice per sounding tile, mixed in the output callback. Gains follow the
//! ramps and envelope of `soundboard_core::gain`, advanced by each audio block.

use crate::decode;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use soundboard_core::gain::{self, seek, LinearRamp};
use soundboard_core::model::{PlayerSettings, SoundSet};
use soundboard_core::Uuid;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub enum EngineEvent {
    /// The voice ended on its own: end of the sound, faded to silence, or output lost.
    Finished(Uuid),
    /// Continuous mode: the voice reached the point where the next tile should start, `lead`
    /// seconds before its end (0 = at the end). Sent at most once per voice.
    Advance(Uuid, f64),
    /// A start couldn't happen (unreadable file, empty region…).
    StartFailed(Uuid, String),
}

pub struct StartRequest {
    pub path: PathBuf,
    pub file_id: Uuid,
    pub settings: PlayerSettings,
    pub title: String,
    /// > 0 starts from silence and ramps up over this time.
    pub fade_in: f64,
    /// MIDI velocity as a gain factor, 1 for mouse and keyboard.
    pub velocity: f32,
    /// The set's crossfade when Continuous is active, to report the advance point.
    pub continuous_crossfade: Option<f64>,
    /// False for one-off regions (editor previews), which would only fill the cache.
    pub cached: bool,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct BufferKey {
    file_id: Uuid,
    trim_start: u64,
    trim_end: u64,
    rate: u32,
}

/// Decoded regions kept for instant restarts, least recently used dropped first beyond the
/// budget. Voices keep their own reference, so dropping one never cuts a sound.
#[derive(Default)]
struct BufferCache {
    entries: HashMap<BufferKey, Arc<Vec<f32>>>,
    recency: VecDeque<BufferKey>,
    bytes: usize,
}

impl BufferCache {
    const BUDGET: usize = 256 * 1024 * 1024;

    fn get(&mut self, key: &BufferKey) -> Option<Arc<Vec<f32>>> {
        let buffer = self.entries.get(key)?.clone();
        self.recency.retain(|k| k != key);
        self.recency.push_back(key.clone());
        Some(buffer)
    }

    fn insert(&mut self, key: BufferKey, buffer: Arc<Vec<f32>>) {
        if self.entries.contains_key(&key) {
            return;
        }
        self.bytes += buffer.len() * 4;
        self.entries.insert(key.clone(), buffer);
        self.recency.push_back(key);
        while self.bytes > Self::BUDGET && self.recency.len() > 1 {
            let oldest = self.recency.pop_front().expect("not empty");
            if let Some(dropped) = self.entries.remove(&oldest) {
                self.bytes -= dropped.len() * 4;
            }
        }
    }
}

struct Voice {
    /// Interleaved stereo at the output rate.
    buffer: Arc<Vec<f32>>,
    frame: usize,
    loops: bool,
    settings: PlayerSettings,
    volume: LinearRamp,
    fade: LinearRamp,
    velocity: f32,
    paused: bool,
    stops_when_silent: bool,
    /// Seconds played since the start, across loops and seeks; drives a looping fade-in.
    played: f64,
    advance_lead: Option<f64>,
    has_advanced: bool,
    /// Gain at the end of the last block, where the next block's ramp starts.
    last_gain: f32,
}

impl Voice {
    fn frames(&self) -> usize {
        self.buffer.len() / 2
    }

    fn volume_for(settings: &PlayerSettings) -> f32 {
        if settings.muted {
            0.0
        } else {
            settings.active_volume()
        }
    }

    fn gain(&self, rate: f64) -> f32 {
        let length = self.frames() as f64 / rate;
        let position = if self.loops {
            self.played
        } else {
            self.frame as f64 / rate
        };
        let envelope = gain::envelope(
            position,
            length,
            self.settings.fade_in_time,
            self.settings.fade_out_time,
            self.loops,
        );
        gain::snapped(self.volume.value() * self.velocity * self.fade.value() * envelope)
    }
}

struct Mixer {
    voices: HashMap<Uuid, Voice>,
    master: LinearRamp,
    rate: f64,
    events: Sender<EngineEvent>,
}

impl Mixer {
    /// Mixes every voice into `out` (interleaved, `channels` per frame) and reports voices
    /// that ended or reached their advance point.
    fn render(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        let frames = out.len() / channels;
        if frames == 0 {
            return;
        }
        let rate = self.rate;
        let elapsed = frames as f64 / rate;
        let master_start = self.master.value();
        let master_end = self.master.advance(elapsed);
        let mut ended = vec![];
        for (id, voice) in self.voices.iter_mut().filter(|(_, v)| !v.paused) {
            voice.volume.advance(elapsed);
            voice.fade.advance(elapsed);
            let start_gain = voice.last_gain * master_start;
            let total = voice.frames();
            let mut finished = false;
            for (i, frame) in out.chunks_exact_mut(channels).enumerate() {
                if voice.frame >= total {
                    if voice.loops && total > 0 {
                        voice.frame = 0;
                    } else {
                        finished = true;
                        break;
                    }
                }
                // Gain moves linearly across the block, so ramps never click.
                let t = i as f32 / frames as f32;
                let g = start_gain + (voice.gain(rate) * master_end - start_gain) * t;
                let (l, r) = (
                    voice.buffer[voice.frame * 2],
                    voice.buffer[voice.frame * 2 + 1],
                );
                if channels == 1 {
                    frame[0] += (l + r) * 0.5 * g;
                } else {
                    frame[0] += l * g;
                    frame[1] += r * g;
                }
                voice.frame += 1;
                voice.played += 1.0 / rate;
            }
            voice.last_gain = voice.gain(rate);
            let length = total as f64 / rate;
            let position = voice.frame as f64 / rate;
            if let Some(lead) = voice.advance_lead {
                let reached = finished || position >= length - lead;
                if reached && !voice.has_advanced && !voice.stops_when_silent {
                    voice.has_advanced = true;
                    let _ = self.events.send(EngineEvent::Advance(*id, lead));
                }
            }
            if finished || (voice.stops_when_silent && voice.fade.is_finished()) {
                ended.push(*id);
            }
        }
        for id in ended {
            self.voices.remove(&id);
            let _ = self.events.send(EngineEvent::Finished(id));
        }
    }

    fn set_master(&mut self, gain: f32) {
        // With nothing sounding there is nothing to ramp.
        let over = if self.voices.is_empty() {
            0.0
        } else {
            LinearRamp::VOLUME_CHANGE
        };
        self.master.move_to(gain, over);
    }
}

pub struct Engine {
    mixer: Arc<Mutex<Mixer>>,
    cache: Arc<Mutex<BufferCache>>,
    /// Starts waiting for their sound to be decoded; a stop before then cancels them.
    pending: Arc<Mutex<HashMap<Uuid, u64>>>,
    tokens: AtomicU64,
    rate: Arc<AtomicU32>,
}

impl Engine {
    /// Opens the default output on a thread that owns the stream and reopens it when the
    /// device goes away (headphones unplugged…). Events arrive on the returned receiver.
    pub fn start() -> (Engine, Receiver<EngineEvent>) {
        let (events, receiver) = mpsc::channel();
        let rate = Arc::new(AtomicU32::new(48_000));
        let mixer = Arc::new(Mutex::new(Mixer {
            voices: HashMap::new(),
            master: LinearRamp::new(1.0),
            rate: 48_000.0,
            events,
        }));
        let engine = Engine {
            mixer: mixer.clone(),
            cache: Arc::default(),
            pending: Arc::default(),
            tokens: AtomicU64::new(0),
            rate: rate.clone(),
        };
        let cache = engine.cache.clone();
        thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || output_thread(mixer, cache, rate))
            .expect("audio thread starts");
        (engine, receiver)
    }

    pub fn start_voice(&self, id: Uuid, request: StartRequest) {
        self.mixer.lock().expect("mixer").voices.remove(&id);
        let rate = self.rate.load(Ordering::Relaxed);
        let key = BufferKey {
            file_id: request.file_id,
            trim_start: request.settings.trim_start.to_bits(),
            trim_end: request.settings.trim_end.to_bits(),
            rate,
        };
        if request.cached {
            if let Some(buffer) = self.cache.lock().expect("cache").get(&key) {
                self.pending.lock().expect("pending").remove(&id);
                self.insert(id, &request, buffer);
                return;
            }
        }
        let token = self.tokens.fetch_add(1, Ordering::Relaxed);
        self.pending.lock().expect("pending").insert(id, token);
        let (mixer, cache, pending) =
            (self.mixer.clone(), self.cache.clone(), self.pending.clone());
        thread::spawn(move || {
            let decoded = decode::region(
                &request.path,
                request.settings.trim_start,
                request.settings.trim_end,
                rate,
                &request.title,
            );
            let mut waiting = pending.lock().expect("pending");
            if waiting.get(&id) != Some(&token) {
                return;
            }
            waiting.remove(&id);
            match decoded {
                Ok(samples) => {
                    let buffer = Arc::new(samples);
                    if request.cached {
                        cache.lock().expect("cache").insert(key, buffer.clone());
                    }
                    insert(&mixer, id, &request, buffer);
                }
                Err(e) => {
                    let _ = mixer
                        .lock()
                        .expect("mixer")
                        .events
                        .send(EngineEvent::StartFailed(id, e));
                }
            }
        });
    }

    fn insert(&self, id: Uuid, request: &StartRequest, buffer: Arc<Vec<f32>>) {
        insert(&self.mixer, id, request, buffer);
    }

    /// `fade == None` cuts immediately; otherwise fades to silence, then stops.
    pub fn stop(&self, id: Uuid, fade: Option<f64>) {
        self.pending.lock().expect("pending").remove(&id);
        let mut mixer = self.mixer.lock().expect("mixer");
        match fade.filter(|f| *f > 0.0) {
            None => {
                mixer.voices.remove(&id);
            }
            Some(fade) => {
                if let Some(voice) = mixer.voices.get_mut(&id) {
                    voice.fade.move_to(0.0, fade);
                    voice.stops_when_silent = true;
                }
            }
        }
    }

    pub fn pause(&self, id: Uuid, paused: bool) {
        if let Some(voice) = self.mixer.lock().expect("mixer").voices.get_mut(&id) {
            voice.paused = paused;
        }
    }

    /// Applies edited settings to a sounding tile; volume changes ramp over 0.2 s. Trim and
    /// loop are fixed by the decoded region until the next start.
    pub fn update(&self, id: Uuid, settings: &PlayerSettings) {
        if let Some(voice) = self.mixer.lock().expect("mixer").voices.get_mut(&id) {
            voice
                .volume
                .move_to(Voice::volume_for(settings), LinearRamp::VOLUME_CHANGE);
            voice.settings = settings.clone();
        }
    }

    /// Seconds into the trimmed region, and the region's length, while the voice sounds.
    pub fn playback(&self, id: Uuid) -> Option<(f64, f64)> {
        let mixer = self.mixer.lock().expect("mixer");
        let voice = mixer.voices.get(&id)?;
        Some((
            voice.frame as f64 / mixer.rate,
            voice.frames() as f64 / mixer.rate,
        ))
    }

    /// Moves a sounding voice to `position` seconds into its trimmed region.
    pub fn seek(&self, id: Uuid, position: f64) {
        let mut mixer = self.mixer.lock().expect("mixer");
        let rate = mixer.rate;
        let Some(voice) = mixer.voices.get_mut(&id) else {
            return;
        };
        let length = voice.frames() as f64 / rate;
        let target = seek::target(position, length, voice.loops);
        voice.frame = (target * rate) as usize;
        if target < length - voice.advance_lead.unwrap_or(0.0) {
            voice.has_advanced = false;
        }
    }

    pub fn set_master_gain(&self, gain: f32) {
        self.mixer.lock().expect("mixer").set_master(gain);
    }
}

fn insert(mixer: &Mutex<Mixer>, id: Uuid, request: &StartRequest, buffer: Arc<Vec<f32>>) {
    let mut mixer = mixer.lock().expect("mixer");
    let rate = mixer.rate;
    let mut fade = LinearRamp::new(1.0);
    if request.fade_in > 0.0 {
        fade = LinearRamp::new(0.0);
        fade.move_to(1.0, request.fade_in);
    }
    let length = buffer.len() as f64 / 2.0 / rate;
    let mut voice = Voice {
        buffer,
        frame: 0,
        loops: request.settings.loops,
        settings: request.settings.clone(),
        volume: LinearRamp::new(Voice::volume_for(&request.settings)),
        fade,
        velocity: request.velocity,
        paused: false,
        stops_when_silent: false,
        played: 0.0,
        advance_lead: request
            .continuous_crossfade
            .map(|c| SoundSet::crossfade_lead(c, length)),
        has_advanced: false,
        last_gain: 0.0,
    };
    voice.last_gain = voice.gain(rate);
    mixer.voices.insert(id, voice);
}

fn output_thread(mixer: Arc<Mutex<Mixer>>, cache: Arc<Mutex<BufferCache>>, rate: Arc<AtomicU32>) {
    loop {
        let (lost, lost_signal) = mpsc::channel::<()>();
        match open_output(&mixer, lost) {
            Ok((stream, device_rate)) => {
                if rate.swap(device_rate, Ordering::Relaxed) != device_rate {
                    // Cached regions were resampled for the old rate.
                    *cache.lock().expect("cache") = BufferCache::default();
                }
                let _ = lost_signal.recv();
                drop(stream);
                // The device is gone with what it was playing: report those sounds as ended.
                let mut mixer = mixer.lock().expect("mixer");
                for id in mixer.voices.drain().map(|(id, _)| id).collect::<Vec<_>>() {
                    let _ = mixer.events.send(EngineEvent::Finished(id));
                }
            }
            Err(e) => eprintln!("audio output unavailable: {e}"),
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn open_output(mixer: &Arc<Mutex<Mixer>>, lost: Sender<()>) -> Result<(cpal::Stream, u32), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("aucune sortie audio")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let config: cpal::StreamConfig = supported.config();
    mixer.lock().expect("mixer").rate = f64::from(config.sample_rate.0);
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, mixer.clone(), lost),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, mixer.clone(), lost),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, mixer.clone(), lost),
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, mixer.clone(), lost),
        other => return Err(format!("format de sortie {other} non pris en charge")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, config.sample_rate.0))
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mixer: Arc<Mutex<Mixer>>,
    lost: Sender<()>,
) -> Result<cpal::Stream, String> {
    let channels = usize::from(config.channels);
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                scratch.resize(out.len(), 0.0);
                mixer.lock().expect("mixer").render(&mut scratch, channels);
                for (o, s) in out.iter_mut().zip(&scratch) {
                    *o = T::from_sample(s.clamp(-1.0, 1.0));
                }
            },
            move |e| {
                eprintln!("audio output error: {e}");
                let _ = lost.send(());
            },
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixer() -> (Mixer, Receiver<EngineEvent>) {
        let (events, receiver) = mpsc::channel();
        let mixer = Mixer {
            voices: HashMap::new(),
            master: LinearRamp::new(1.0),
            rate: 100.0,
            events,
        };
        (mixer, receiver)
    }

    fn request(settings: PlayerSettings, crossfade: Option<f64>) -> StartRequest {
        StartRequest {
            path: PathBuf::new(),
            file_id: Uuid::new_v4(),
            settings,
            title: String::new(),
            fade_in: 0.0,
            velocity: 1.0,
            continuous_crossfade: crossfade,
            cached: false,
        }
    }

    /// One second of constant 0.5 at 100 Hz, stereo.
    fn second() -> Arc<Vec<f32>> {
        Arc::new(vec![0.5; 200])
    }

    #[test]
    fn plays_to_the_end_then_reports_it() {
        let (mixer, events) = mixer();
        let mixer = Mutex::new(mixer);
        let id = Uuid::new_v4();
        insert(
            &mixer,
            id,
            &request(PlayerSettings::default(), None),
            second(),
        );
        let mut mixer = mixer.into_inner().unwrap();
        let mut out = vec![0.0; 120];
        mixer.render(&mut out, 2);
        assert_eq!(out[0], 0.5);
        mixer.render(&mut out, 2);
        mixer.render(&mut out, 2);
        assert!(mixer.voices.is_empty());
        assert!(matches!(events.try_recv(), Ok(EngineEvent::Finished(f)) if f == id));
    }

    #[test]
    fn continuous_reports_the_advance_point_once() {
        let (mixer, events) = mixer();
        let mixer = Mutex::new(mixer);
        let id = Uuid::new_v4();
        insert(
            &mixer,
            id,
            &request(PlayerSettings::default(), Some(0.3)),
            second(),
        );
        let mut mixer = mixer.into_inner().unwrap();
        // Blocks of 0.4 s.
        let mut out = vec![0.0; 80];
        mixer.render(&mut out, 2);
        assert!(events.try_recv().is_err(), "0.6 s left");
        mixer.render(&mut out, 2);
        assert!(matches!(events.try_recv(), Ok(EngineEvent::Advance(_, lead)) if lead == 0.3));
        mixer.render(&mut out, 2);
        mixer.render(&mut out, 2);
        assert!(matches!(events.try_recv(), Ok(EngineEvent::Finished(_))));
        assert!(events.try_recv().is_err(), "advance is reported once");
    }

    #[test]
    fn faded_stops_end_when_silent() {
        let (mixer, events) = mixer();
        let mixer = Mutex::new(mixer);
        let id = Uuid::new_v4();
        let looping = PlayerSettings {
            loops: true,
            ..Default::default()
        };
        insert(&mixer, id, &request(looping, None), second());
        let mut mixer = mixer.into_inner().unwrap();
        let voice = mixer.voices.get_mut(&id).unwrap();
        voice.fade.move_to(0.0, 0.5);
        voice.stops_when_silent = true;
        let mut out = vec![0.0; 120];
        for _ in 0..5 {
            mixer.render(&mut out, 2);
        }
        assert!(matches!(events.try_recv(), Ok(EngineEvent::Finished(_))));
    }

    #[test]
    fn mono_outputs_get_both_channels() {
        let (mixer, _events) = mixer();
        let mixer = Mutex::new(mixer);
        insert(
            &mixer,
            Uuid::new_v4(),
            &request(PlayerSettings::default(), None),
            second(),
        );
        let mut mixer = mixer.into_inner().unwrap();
        let mut out = vec![0.0; 10];
        mixer.render(&mut out, 1);
        assert_eq!(out[0], 0.5);
    }
}
