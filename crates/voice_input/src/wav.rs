//! Minimal WAV encoding and resampling for transcription uploads.
//!
//! Speech-to-text APIs expect mono 16 kHz audio, but microphones hand out
//! whatever the device is configured for (usually 44.1 or 48 kHz, often
//! stereo), so the captured samples are mixed down and resampled before being
//! wrapped in a 16-bit PCM WAV container.

/// Target sample rate for transcription uploads.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Mixes interleaved samples down to mono.
pub fn downmix_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }

    samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// Resamples mono audio with linear interpolation.
pub fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() || from_rate == 0 || to_rate == 0 {
        return samples.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = ((samples.len() as f64) / ratio).floor() as usize;
    let mut output = Vec::with_capacity(output_len);

    for index in 0..output_len {
        let position = index as f64 * ratio;
        let left = position.floor() as usize;
        let right = (left + 1).min(samples.len() - 1);
        let fraction = (position - left as f64) as f32;
        let left_sample = samples[left];
        let right_sample = samples[right];
        output.push(left_sample + (right_sample - left_sample) * fraction);
    }

    output
}

/// Encodes mono f32 samples as a 16-bit PCM WAV file.
pub fn encode_wav(pcm: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() * 2;
    let mut wav = Vec::with_capacity(44 + data_len);

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_len as u32).to_le_bytes());

    for sample in pcm {
        let clamped = sample.clamp(-1.0, 1.0);
        let scaled = (clamped * i16::MAX as f32).round() as i16;
        wav.extend_from_slice(&scaled.to_le_bytes());
    }

    wav
}

/// Converts captured mono samples into a 16 kHz 16-bit PCM WAV file.
pub fn to_transcription_wav(samples: &[f32], channels: usize, sample_rate: u32) -> Vec<u8> {
    let mono = downmix_to_mono(samples, channels);
    let resampled = resample_linear(&mono, sample_rate, TARGET_SAMPLE_RATE);
    encode_wav(&resampled, TARGET_SAMPLE_RATE)
}

/// Converts captured audio into one or more self-contained 16 kHz mono WAV
/// files, splitting long recordings into chunks of at most `chunk_secs`.
///
/// Splits are placed at the quietest 10 ms frame near each nominal boundary so
/// a word is unlikely to be cut in half, falling back to a hard split when no
/// quiet frame is found. `chunk_secs == 0` or a short recording yields a single
/// chunk. Sending chunks one at a time keeps each request small so a slow or
/// failing request cannot discard a whole long dictation.
pub fn split_and_encode(
    samples: &[f32],
    channels: usize,
    sample_rate: u32,
    chunk_secs: u64,
    silence_threshold: f32,
) -> Vec<Vec<u8>> {
    let mono = downmix_to_mono(samples, channels);
    let resampled = resample_linear(&mono, sample_rate, TARGET_SAMPLE_RATE);
    split_mono_and_encode(&resampled, chunk_secs, silence_threshold)
}

/// Encoded chunks are preceded and followed by this much audio so the model
/// still hears a natural onset and release instead of a hard cut.
const SILENCE_PADDING_SAMPLES: usize = 1_600; // 100 ms at 16 kHz

/// Range of `pcm` that contains speech, with a little padding. The end points
/// snap to 10 ms frames so a trimmed chunk does not start mid-word.
fn trim_silence(pcm: &[f32], silence_threshold: f32) -> &[f32] {
    const FRAME: usize = 160; // 10 ms at 16 kHz
    let is_loud = |frame: &[f32]| frame.iter().any(|sample| sample.abs() > silence_threshold);

    let first = pcm.chunks(FRAME).position(is_loud);
    let last = pcm.chunks(FRAME).rposition(is_loud);
    let (Some(first), Some(last)) = (first, last) else {
        return &pcm[..0];
    };

    let start = (first * FRAME).saturating_sub(SILENCE_PADDING_SAMPLES);
    let end = ((last + 1) * FRAME + SILENCE_PADDING_SAMPLES).min(pcm.len());
    &pcm[start..end]
}

/// Splits 16 kHz mono samples into chunks and encodes each as a WAV.
///
/// Leading and trailing silence is trimmed first: the model tends to invent
/// subtitle credits and other text when it is fed dead air, and trimming also
/// keeps the request small. Audio that never rises above the threshold yields
/// no chunks at all.
pub fn split_mono_and_encode(pcm: &[f32], chunk_secs: u64, silence_threshold: f32) -> Vec<Vec<u8>> {
    let pcm = trim_silence(pcm, silence_threshold);
    if pcm.is_empty() {
        return Vec::new();
    }

    let chunk_samples = (chunk_secs as usize) * TARGET_SAMPLE_RATE as usize;
    if chunk_secs == 0 || pcm.len() <= chunk_samples {
        return vec![encode_wav(pcm, TARGET_SAMPLE_RATE)];
    }

    let frame = (TARGET_SAMPLE_RATE as usize) / 100; // 10 ms analysis frame
    let search = chunk_samples / 5; // search the trailing 20% for a quiet spot
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < pcm.len() {
        if pcm.len() - start <= chunk_samples {
            chunks.push(encode_wav(&pcm[start..], TARGET_SAMPLE_RATE));
            break;
        }

        let nominal_end = start + chunk_samples;
        let search_start = nominal_end - search;
        let mut best: Option<(usize, f32)> = None;
        let mut position = search_start;
        while position + frame <= nominal_end {
            let peak = pcm[position..position + frame]
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            if best.is_none_or(|(_, best_peak)| peak < best_peak) {
                best = Some((position + frame / 2, peak));
            }
            position += frame;
        }

        let cut = match best {
            Some((at, peak)) if peak <= silence_threshold && at > start => at,
            _ => nominal_end,
        };
        chunks.push(encode_wav(&pcm[start..cut], TARGET_SAMPLE_RATE));
        start = cut;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        let samples = [1.0, 0.0, 0.5, 0.5];
        assert_eq!(downmix_to_mono(&samples, 2), vec![0.5, 0.5]);
    }

    #[test]
    fn resample_halves_the_length_when_halving_the_rate() {
        let samples: Vec<f32> = (0..48_000).map(|index| index as f32).collect();
        let resampled = resample_linear(&samples, 48_000, 16_000);
        assert!((resampled.len() as i32 - 16_000).abs() <= 1);
    }

    #[test]
    fn wav_header_is_well_formed() {
        let wav = encode_wav(&[0.0, 0.5, -0.5], TARGET_SAMPLE_RATE);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 6);
    }

    fn loud_signal(len: usize) -> Vec<f32> {
        (0..len)
            .map(|index| if index % 2 == 0 { 0.6 } else { -0.6 })
            .collect()
    }

    #[test]
    fn short_audio_is_a_single_chunk() {
        let pcm = loud_signal(TARGET_SAMPLE_RATE as usize);
        let chunks = split_mono_and_encode(&pcm, 120, 0.02);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn zero_chunk_secs_disables_chunking() {
        let pcm = loud_signal(TARGET_SAMPLE_RATE as usize * 300);
        let chunks = split_mono_and_encode(&pcm, 0, 0.02);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn silence_is_trimmed_away_entirely() {
        let pcm = vec![0.0_f32; TARGET_SAMPLE_RATE as usize * 5];
        assert!(split_mono_and_encode(&pcm, 120, 0.02).is_empty());
    }

    #[test]
    fn surrounding_silence_is_trimmed() {
        let rate = TARGET_SAMPLE_RATE as usize;
        let mut pcm = vec![0.0_f32; rate];
        pcm.extend(loud_signal(rate / 2));
        pcm.extend(vec![0.0_f32; rate]);

        let trimmed = trim_silence(&pcm, 0.02);
        assert!(trimmed.len() < pcm.len());
        assert!(trimmed.len() > rate / 2);
        assert!(
            (trimmed.len() as i64 - (rate / 2 + 2 * SILENCE_PADDING_SAMPLES) as i64).abs() <= 320
        );
    }

    #[test]
    fn empty_audio_has_no_chunks() {
        assert!(split_mono_and_encode(&[], 120, 0.02).is_empty());
    }

    #[test]
    fn long_audio_is_split_into_bounded_chunks() {
        // 300 s of loud audio with `chunk_secs = 60` must split into ~5 chunks.
        let seconds = 300;
        let pcm = loud_signal(TARGET_SAMPLE_RATE as usize * seconds);
        let chunks = split_mono_and_encode(&pcm, 60, 0.02);
        assert!(chunks.len() >= 5, "got {} chunks", chunks.len());

        let max_len = TARGET_SAMPLE_RATE as usize * 60 * 2 + 44;
        for chunk in &chunks {
            assert!(
                chunk.len() <= max_len,
                "chunk of {} bytes too large",
                chunk.len()
            );
            assert_eq!(&chunk[0..4], b"RIFF");
        }
    }

    #[test]
    fn splits_land_on_quiet_frames() {
        // Loud audio with a silent gap right before each nominal boundary should
        // be cut inside the silence instead of at the fixed boundary.
        let seconds = 120;
        let mut pcm = loud_signal(TARGET_SAMPLE_RATE as usize * seconds);
        let gap_start = TARGET_SAMPLE_RATE as usize * 55;
        let gap_len = TARGET_SAMPLE_RATE as usize * 2;
        for sample in &mut pcm[gap_start..gap_start + gap_len] {
            *sample = 0.0;
        }

        let chunks = split_mono_and_encode(&pcm, 60, 0.02);
        assert!(chunks.len() >= 2);
        // The first chunk should end within the silent gap rather than at 60 s.
        let first_samples = (chunks[0].len() - 44) / 2;
        assert!(first_samples >= gap_start);
        assert!(first_samples <= gap_start + gap_len);
    }
}
