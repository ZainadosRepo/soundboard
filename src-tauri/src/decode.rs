//! Reading sound files: lengths, trimmed regions ready for the output device, and mono
//! samples for waveforms.

use rubato::{FftFixedIn, Resampler};
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Extensions offered when adding sounds; the decoders cover them all.
pub const AUDIO_EXTENSIONS: [&str; 12] = [
    "wav", "wave", "aif", "aiff", "aifc", "mp3", "m4a", "aac", "mp4", "flac", "ogg", "oga",
];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Decoded samples, one vector per channel.
pub struct Decoded {
    pub rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Decoded {
    fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

fn open(path: &Path) -> Result<Box<dyn FormatReader>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map(|p| p.format)
        .map_err(|_| "format audio non reconnu".to_owned())
}

/// Length of the whole file in seconds, from its header when it says, otherwise by decoding.
pub fn duration(path: &Path) -> Result<f64, String> {
    let format = open(path)?;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("aucune piste audio")?;
    if let (Some(frames), Some(rate)) =
        (track.codec_params.n_frames, track.codec_params.sample_rate)
    {
        return Ok(frames as f64 / f64::from(rate));
    }
    let decoded = decode(path)?;
    Ok(decoded.frames() as f64 / f64::from(decoded.rate))
}

pub fn decode(path: &Path) -> Result<Decoded, String> {
    let mut format = open(path)?;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("aucune piste audio")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| "codec audio non pris en charge".to_owned())?;
    let mut out = Decoded {
        rate: track.codec_params.sample_rate.unwrap_or(0),
        channels: vec![],
    };
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            // A damaged packet is skipped, as players do, rather than failing the whole sound.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let spec = *buffer.spec();
        let count = spec.channels.count();
        if out.channels.is_empty() {
            out.channels = vec![Vec::new(); count];
            out.rate = spec.rate;
        }
        let mut samples = SampleBuffer::<f32>::new(buffer.capacity() as u64, spec);
        samples.copy_interleaved_ref(buffer);
        for frame in samples.samples().chunks_exact(count) {
            for (channel, sample) in out.channels.iter_mut().zip(frame) {
                channel.push(*sample);
            }
        }
    }
    if out.channels.is_empty() || out.rate == 0 {
        return Err("le fichier ne contient aucun son".into());
    }
    Ok(out)
}

/// The trimmed region of a file as interleaved stereo at `rate`, for the mixer.
pub fn region(
    path: &Path,
    trim_start: f64,
    trim_end: f64,
    rate: u32,
    title: &str,
) -> Result<Vec<f32>, String> {
    let decoded = decode(path)?;
    let file_rate = f64::from(decoded.rate);
    let frames = decoded.frames();
    let start = ((trim_start.max(0.0) * file_rate) as usize).min(frames);
    let end = frames.saturating_sub((trim_end.max(0.0) * file_rate) as usize);
    if end <= start {
        return Err(format!("« {title} » est vide une fois découpé."));
    }
    let left = &decoded.channels[0][start..end];
    let right = &decoded.channels.get(1).unwrap_or(&decoded.channels[0])[start..end];
    let (left, right) = if decoded.rate == rate {
        (left.to_vec(), right.to_vec())
    } else {
        let mut resampled = resample(&[left, right], decoded.rate, rate)?;
        let right = resampled.pop().expect("two channels");
        (resampled.pop().expect("two channels"), right)
    };
    Ok(left
        .iter()
        .zip(&right)
        .flat_map(|(l, r)| [*l, *r])
        .collect())
}

/// Every channel mixed down to mono, for waveforms.
pub fn mono(path: &Path) -> Result<Vec<f32>, String> {
    let decoded = decode(path)?;
    let scale = 1.0 / decoded.channels.len() as f32;
    Ok((0..decoded.frames())
        .map(|i| decoded.channels.iter().map(|c| c[i]).sum::<f32>() * scale)
        .collect())
}

fn resample(input: &[&[f32]], from: u32, to: u32) -> Result<Vec<Vec<f32>>, String> {
    let error = |e: &dyn std::fmt::Display| format!("conversion de fréquence impossible ({e})");
    let frames = input[0].len();
    let mut resampler = FftFixedIn::<f32>::new(from as usize, to as usize, 1024, 2, input.len())
        .map_err(|e| error(&e))?;
    let delay = resampler.output_delay();
    let expected = (frames as f64 * f64::from(to) / f64::from(from)).round() as usize;
    let mut out: Vec<Vec<f32>> = vec![Vec::with_capacity(expected + delay); input.len()];
    let append = |out: &mut Vec<Vec<f32>>, chunk: Vec<Vec<f32>>| {
        for (o, c) in out.iter_mut().zip(chunk) {
            o.extend(c);
        }
    };
    let mut position = 0;
    while frames - position >= resampler.input_frames_next() {
        let next = resampler.input_frames_next();
        let chunk: Vec<&[f32]> = input
            .iter()
            .map(|c| &c[position..position + next])
            .collect();
        append(
            &mut out,
            resampler.process(&chunk, None).map_err(|e| error(&e))?,
        );
        position += next;
    }
    if position < frames {
        let chunk: Vec<&[f32]> = input.iter().map(|c| &c[position..]).collect();
        append(
            &mut out,
            resampler
                .process_partial(Some(&chunk), None)
                .map_err(|e| error(&e))?,
        );
    }
    // The resampler holds back `delay` frames: flush them out with silence.
    while out[0].len() < expected + delay {
        let flushed = resampler
            .process_partial::<&[f32]>(None, None)
            .map_err(|e| error(&e))?;
        if flushed[0].is_empty() {
            break;
        }
        append(&mut out, flushed);
    }
    for channel in &mut out {
        channel.drain(..delay.min(channel.len()));
        channel.truncate(expected);
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_length_and_level() {
        let tone: Vec<f32> = (0..44_100)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 0.5)
            .collect();
        let out = resample(&[&tone, &tone], 44_100, 48_000).unwrap();
        assert_eq!(out[0].len(), 48_000);
        let peak = out[0][1000..47_000]
            .iter()
            .fold(0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.02, "peak {peak}");
    }

    /// A 16-bit PCM WAV of `seconds` of a 440 Hz tone, the way any editor writes one.
    pub(crate) fn write_wav(path: &Path, seconds: f64, rate: u32, channels: u16) {
        let frames = (seconds * f64::from(rate)) as u32;
        let data_len = frames * u32::from(channels) * 2;
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + data_len).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(channels.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend((rate * u32::from(channels) * 2).to_le_bytes());
        bytes.extend((channels * 2).to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(data_len.to_le_bytes());
        for i in 0..frames {
            let v =
                ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 16_000.0) as i16;
            for _ in 0..channels {
                bytes.extend(v.to_le_bytes());
            }
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn reads_lengths_regions_and_mono() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        write_wav(&path, 2.0, 44_100, 1);
        assert!((duration(&path).unwrap() - 2.0).abs() < 1e-3);
        let stereo = region(&path, 0.5, 0.5, 48_000, "tone").unwrap();
        assert_eq!(
            stereo.len(),
            48_000 * 2,
            "one second at the output rate, interleaved"
        );
        assert_eq!(stereo[100], stereo[101], "mono is copied to both channels");
        assert_eq!(mono(&path).unwrap().len(), 88_200);
        let empty = region(&path, 1.5, 0.6, 48_000, "tone").unwrap_err();
        assert!(empty.contains("vide"), "{empty}");
        let missing = dir.path().join("absent.wav");
        assert!(duration(&missing).is_err());
    }

    #[test]
    fn audio_extensions_ignore_case() {
        assert!(is_audio(Path::new("a/Bip.MP3")));
        assert!(!is_audio(Path::new("a/notes.txt")));
    }
}
