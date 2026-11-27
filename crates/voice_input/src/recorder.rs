//! Microphone capture for voice input.
//!
//! The stream is opened on the device chosen in `voice_input.device`, falling
//! back to the device the user configured for audio calls (`audio` settings), so
//! a headset selected for calls is used for dictation too.
//!
//! Recording length and silence limits are configurable; [`auto_stop_reason`]
//! turns the current elapsed and silence times into an optional stop reason so
//! the UI can stop and transcribe without duplicating the policy.

use std::{
    str::FromStr as _,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use cpal::{
    DeviceId, SampleFormat, SupportedStreamConfig,
    traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _},
};
use settings::Settings as _;

use crate::VoiceInputSettingsContent;
use crate::settings::VoiceInputSettings;

/// Why a recording stopped on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoStopReason {
    /// The `voice_input.max_duration_secs` limit was reached.
    MaxDuration,
    /// No audio above the silence threshold arrived for
    /// `voice_input.silence_timeout_secs`.
    Silence,
}

/// Decides whether an in-progress recording should stop automatically.
///
/// Both limits are opt-in: `max_duration_secs == 0` and
/// `silence_timeout_secs == 0` disable the corresponding check.
pub fn auto_stop_reason(
    elapsed: Duration,
    silence: Duration,
    content: &VoiceInputSettingsContent,
) -> Option<AutoStopReason> {
    let max_duration = content.max_duration_secs();
    if max_duration > 0 && elapsed >= Duration::from_secs(max_duration) {
        return Some(AutoStopReason::MaxDuration);
    }

    let silence_timeout = content.silence_timeout_secs();
    if silence_timeout > 0 && silence >= Duration::from_secs(silence_timeout) {
        return Some(AutoStopReason::Silence);
    }

    None
}

/// Whether a finished recording is too short to bother transcribing.
pub fn is_too_short(elapsed: Duration, content: &VoiceInputSettingsContent) -> bool {
    elapsed < Duration::from_millis(content.min_duration_ms())
}

/// An in-progress microphone recording.
pub struct Recorder {
    config: SupportedStreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
    last_loud: Arc<Mutex<Instant>>,
    threshold: f32,
    stream: cpal::Stream,
    started_at: Instant,
}

impl Recorder {
    /// Starts capturing from the configured microphone.
    ///
    /// The device chosen in `voice_input.device` wins, then the one selected for
    /// audio calls, and if neither can be opened the remaining input devices are
    /// tried in turn. A stale or busy device therefore degrades to any working
    /// microphone instead of failing the whole dictation.
    pub fn start_from_settings(cx: &gpui::App) -> Result<Self> {
        let content = VoiceInputSettings::get_global(cx).content();
        let from_voice_settings = content
            .device()
            .and_then(|device| DeviceId::from_str(device).ok());
        let from_audio_settings = audio::AudioSettings::get_global(cx)
            .input_audio_device
            .clone();

        let preferred: Vec<DeviceId> = [from_voice_settings, from_audio_settings]
            .into_iter()
            .flatten()
            .collect();
        Self::start(&preferred, content.silence_threshold())
    }

    /// Opens the input stream and starts capturing mono samples.
    ///
    /// `silence_threshold` is the peak amplitude below which a frame counts as
    /// silence; it is only used to report [`Self::silence_duration`].
    pub fn start(preferred: &[DeviceId], silence_threshold: f32) -> Result<Self> {
        let mut failures = Vec::new();
        let mut attempted = 0usize;
        for (name, device) in candidate_devices(preferred) {
            attempted += 1;
            match Self::start_on(&device, silence_threshold) {
                Ok(recorder) => {
                    log::info!("voice input: recording from microphone {name}");
                    return Ok(recorder);
                }
                Err(error) => failures.push(format!("{name}: {error:#}")),
            }
        }

        match failures.len() {
            0 => anyhow::bail!(
                "no audio input device is available; connect a microphone or check your system sound settings"
            ),
            1 => anyhow::bail!("could not open the microphone ({})", failures[0]),
            _ => anyhow::bail!(
                "none of the {attempted} microphones could be opened; first error: {}",
                failures[0]
            ),
        }
    }

    fn start_on(device: &cpal::Device, silence_threshold: f32) -> Result<Self> {
        let config = device
            .default_input_config()
            .context("the microphone has no default input configuration")?;

        let samples = Arc::new(Mutex::new(Vec::new()));
        let last_loud = Arc::new(Mutex::new(Instant::now()));
        let channels = config.channels() as usize;
        let sample_format = config.sample_format();

        let stream = device
            .build_input_stream_raw(
                config.config(),
                sample_format,
                {
                    let samples = samples.clone();
                    let last_loud = last_loud.clone();
                    move |data, _: &_| {
                        let Some(mono) = to_mono_f32(sample_format, data, channels) else {
                            return;
                        };
                        if peak(&mono) > silence_threshold {
                            *last_loud
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Instant::now();
                        }
                        let mut samples = samples
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        samples.extend_from_slice(&mono);
                    }
                },
                |error| log::error!("voice input: audio capture error: {error}"),
                Some(Duration::from_millis(100)),
            )
            .context("failed to build the microphone input stream")?;

        stream.play().context("failed to start the microphone")?;

        Ok(Self {
            config,
            samples,
            last_loud,
            threshold: silence_threshold,
            stream,
            started_at: Instant::now(),
        })
    }

    /// How long the microphone has been recording.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// How long it has been since audio above the silence threshold was heard.
    pub fn silence_duration(&self) -> Duration {
        self.last_loud
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .elapsed()
    }

    pub fn silence_threshold(&self) -> f32 {
        self.threshold
    }

    /// Whether the recording should stop automatically, per the current
    /// settings.
    pub fn auto_stop_reason(&self, content: &VoiceInputSettingsContent) -> Option<AutoStopReason> {
        auto_stop_reason(self.elapsed(), self.silence_duration(), content)
    }

    /// Stops the stream and returns the captured mono samples and the device
    /// sample rate, so callers can split a long recording into chunks.
    pub fn finish_samples(self) -> Result<(Vec<f32>, u32)> {
        let Self {
            config,
            samples,
            stream,
            ..
        } = self;
        drop(stream);

        let samples = std::mem::take(
            &mut *samples
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        if samples.is_empty() {
            anyhow::bail!("no audio was captured from the microphone");
        }

        Ok((samples, config.sample_rate()))
    }

    /// Stops the stream and returns a 16 kHz mono WAV of the recording.
    pub fn finish(self) -> Result<Vec<u8>> {
        let (samples, sample_rate) = self.finish_samples()?;
        Ok(crate::wav::to_transcription_wav(&samples, 1, sample_rate))
    }
}

/// Input devices to try, in the order they should be attempted: the preferred
/// ids first, then the system default, then every other input device the host
/// enumerates. Duplicates are dropped by name so a device that is both the
/// default and explicitly selected is only tried once.
fn candidate_devices(preferred: &[DeviceId]) -> Vec<(String, cpal::Device)> {
    let host = cpal::default_host();
    let mut devices: Vec<(String, cpal::Device)> = Vec::new();

    if let Some(device) = host.default_input_device() {
        push_device(&mut devices, device);
    }
    for id in preferred {
        if let Some(device) = host.device_by_id(id) {
            push_device(&mut devices, device);
        }
    }
    match host.input_devices() {
        Ok(input_devices) => {
            for device in input_devices {
                push_device(&mut devices, device);
            }
        }
        Err(error) => log::warn!("voice input: could not enumerate microphones: {error}"),
    }

    devices
}

fn push_device(devices: &mut Vec<(String, cpal::Device)>, device: cpal::Device) {
    let name = device
        .description()
        .map(|description| description.name().to_string())
        .unwrap_or_else(|_| "unknown microphone".to_string());
    if !devices.iter().any(|(existing, _)| *existing == name) {
        devices.push((name, device));
    }
}

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
}

fn to_mono_f32(format: SampleFormat, data: &cpal::Data, channels: usize) -> Option<Vec<f32>> {
    let mono: Vec<f32> = match format {
        SampleFormat::F32 => downmix(data.as_slice::<f32>()?, channels),
        SampleFormat::F64 => downmix(
            &data
                .as_slice::<f64>()?
                .iter()
                .map(|sample| *sample as f32)
                .collect::<Vec<_>>(),
            channels,
        ),
        SampleFormat::I16 => downmix(
            &data
                .as_slice::<i16>()?
                .iter()
                .map(|sample| *sample as f32 / i16::MAX as f32)
                .collect::<Vec<_>>(),
            channels,
        ),
        SampleFormat::U16 => downmix(
            &data
                .as_slice::<u16>()?
                .iter()
                .map(|sample| (*sample as f32 - 32768.0) / 32768.0)
                .collect::<Vec<_>>(),
            channels,
        ),
        SampleFormat::I32 => downmix(
            &data
                .as_slice::<i32>()?
                .iter()
                .map(|sample| *sample as f32 / i32::MAX as f32)
                .collect::<Vec<_>>(),
            channels,
        ),
        other => {
            log::warn!("voice input: unsupported microphone sample format {other:?}");
            return None;
        }
    };

    Some(mono)
}

fn downmix(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }

    samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(max: u64, silence_timeout: u64) -> VoiceInputSettingsContent {
        VoiceInputSettingsContent {
            max_duration_secs: Some(max),
            silence_timeout_secs: Some(silence_timeout),
            ..Default::default()
        }
    }

    #[test]
    fn peak_tracks_loudest_sample() {
        assert!((peak(&[0.1, -0.5, 0.2]) - 0.5).abs() < f32::EPSILON);
        assert_eq!(peak(&[]), 0.0);
    }

    #[test]
    fn no_auto_stop_before_bounds() {
        let content = content(10, 5);
        assert_eq!(
            auto_stop_reason(Duration::from_secs(1), Duration::from_secs(1), &content),
            None
        );
    }

    #[test]
    fn max_duration_triggers_auto_stop() {
        let content = content(10, 0);
        assert_eq!(
            auto_stop_reason(Duration::from_secs(10), Duration::ZERO, &content),
            Some(AutoStopReason::MaxDuration)
        );
    }

    #[test]
    fn silence_timeout_triggers_auto_stop() {
        let content = content(0, 3);
        assert_eq!(
            auto_stop_reason(Duration::from_secs(1), Duration::from_secs(3), &content),
            Some(AutoStopReason::Silence)
        );
    }

    #[test]
    fn zero_disables_both_bounds() {
        let content = content(0, 0);
        assert_eq!(
            auto_stop_reason(Duration::from_secs(600), Duration::from_secs(600), &content),
            None
        );
    }

    #[test]
    fn too_short_detects_quick_taps() {
        let content = VoiceInputSettingsContent {
            min_duration_ms: Some(500),
            ..Default::default()
        };
        assert!(is_too_short(Duration::from_millis(100), &content));
        assert!(!is_too_short(Duration::from_millis(600), &content));
    }
}
