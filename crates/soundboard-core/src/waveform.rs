//! Waveform peaks for drawing a sound, and their cache file.

use serde::Serialize;

/// Lowest and highest sample of one slice of a sound.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Peak {
    pub low: f32,
    pub high: f32,
}

/// Slices drawn across the inspector, whatever the sound's length.
pub const BIN_COUNT: usize = 600;

/// Bins for the editor: about 5 ms each so zooming stays sharp, capped for memory.
pub fn editor_bin_count(duration: f64) -> usize {
    ((duration * 200.0) as usize).clamp(BIN_COUNT, 200_000)
}

/// Splits mono samples into `bin_count` equal slices and keeps each slice's extremes.
/// Shorter sounds give fewer bins, one per sample.
pub fn peaks(samples: &[f32], bin_count: usize) -> Vec<Peak> {
    if samples.is_empty() || bin_count == 0 {
        return vec![];
    }
    let bins = bin_count.min(samples.len());
    (0..bins)
        .map(|bin| {
            let slice = &samples[bin * samples.len() / bins..(bin + 1) * samples.len() / bins];
            Peak {
                low: slice.iter().copied().fold(f32::INFINITY, f32::min),
                high: slice.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            }
        })
        .collect()
}

/// On-disk form of peaks: "SBWF", a format version, the bin count, then each bin's low and
/// high as little-endian f32.
pub mod file {
    use super::Peak;

    const MAGIC: &[u8; 4] = b"SBWF";
    const VERSION: u32 = 1;

    pub fn encode(peaks: &[Peak]) -> Vec<u8> {
        let mut data = MAGIC.to_vec();
        data.extend(VERSION.to_le_bytes());
        data.extend((peaks.len() as u32).to_le_bytes());
        for p in peaks {
            data.extend(p.low.to_le_bytes());
            data.extend(p.high.to_le_bytes());
        }
        data
    }

    /// `None` for anything not written by `encode` in this version, so a damaged or outdated
    /// file is just computed again.
    pub fn decode(data: &[u8]) -> Option<Vec<Peak>> {
        let word =
            |offset: usize| -> Option<[u8; 4]> { data.get(offset..offset + 4)?.try_into().ok() };
        if data.get(..4)? != MAGIC || u32::from_le_bytes(word(4)?) != VERSION {
            return None;
        }
        let count = u32::from_le_bytes(word(8)?) as usize;
        if data.len() != 12 + count * 8 {
            return None;
        }
        (0..count)
            .map(|i| {
                let at = 12 + i * 8;
                Some(Peak {
                    low: f32::from_le_bytes(word(at)?),
                    high: f32::from_le_bytes(word(at + 4)?),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_keep_each_slice_extremes() {
        let samples = [0.1, -0.5, 0.9, 0.2, -0.1, 0.0];
        assert_eq!(
            peaks(&samples, 2),
            [
                Peak {
                    low: -0.5,
                    high: 0.9
                },
                Peak {
                    low: -0.1,
                    high: 0.2
                }
            ]
        );
        assert_eq!(peaks(&samples[..1], 600).len(), 1);
        assert!(peaks(&[], 10).is_empty());
    }

    #[test]
    fn editor_bins_grow_with_the_sound() {
        assert_eq!(editor_bin_count(1.0), BIN_COUNT);
        assert_eq!(editor_bin_count(60.0), 12_000);
        assert_eq!(editor_bin_count(10_000.0), 200_000);
    }

    #[test]
    fn file_round_trips_and_rejects_damage() {
        let p = vec![
            Peak {
                low: -1.0,
                high: 0.5,
            },
            Peak {
                low: 0.0,
                high: 0.25,
            },
        ];
        let data = file::encode(&p);
        assert_eq!(file::decode(&data), Some(p));
        assert_eq!(file::decode(&data[..data.len() - 1]), None);
        assert_eq!(file::decode(b"XXXX"), None);
    }
}
