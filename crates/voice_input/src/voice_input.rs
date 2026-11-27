//! Voice input (dictation) for the agent panel.
//!
//! The crate records microphone audio, converts it to the mono 16 kHz WAV that
//! transcription backends expect, and transcribes it either through an
//! OpenAI-compatible HTTP API or with a locally downloaded model. Settings,
//! recording and transcription are exposed here so the UI layer only has to
//! drive the microphone button and insert the resulting text.

mod catalog;
mod downloads;
mod local;
mod recorder;
mod settings;
mod text;
mod transcribe;
mod wav;

use std::sync::Arc;

use ::settings::Settings as _;
use anyhow::{Context as _, Result};
use gpui::{App, BackgroundExecutor, Task};
use http_client::HttpClient;

pub use catalog::{
    CLOUD_PRESETS, CloudPreset, DEFAULT_CLOUD_PRESET, DEFAULT_LOCAL_MODEL, Download, LOCAL_MODELS,
    LocalEngine, LocalModelDef, cloud_preset, is_downloaded, is_downloaded_by_id, local_model,
    model_dir, models_dir, resolve_binary, whisper_binary,
};
pub use downloads::{ModelDownload, ModelDownloadStatus};
pub use downloads::{
    cancel as cancel_model_download, cancel_whisper_install, delete as delete_model_download,
    is_any_downloading, start as start_model_download, start_whisper_install,
    status as model_download_status, whisper_binary_ready, whisper_install_status,
};
pub use local::{DownloadProgress, ensure_model, models_dir as local_models_dir, transcribe_local};
pub use recorder::{
    AutoStopReason, InputDeviceInfo, Recorder, auto_stop_reason, input_devices, is_too_short,
};
pub use settings::VoiceInputSettings;
pub use settings_content::{
    DEFAULT_CHUNK_SECS, DEFAULT_CLOUD_RETRIES, DEFAULT_CLOUD_TIMEOUT_SECS, DEFAULT_CONTEXT_CHARS,
    DEFAULT_MAX_DURATION_SECS, DEFAULT_MIN_DURATION_MS, DEFAULT_SILENCE_THRESHOLD,
    DEFAULT_SILENCE_TIMEOUT_SECS, VoiceInputCloudSettingsContent, VoiceInputEngine,
    VoiceInputInsertion, VoiceInputLocalEngine, VoiceInputLocalSettingsContent, VoiceInputRouting,
    VoiceInputSettingsContent,
};
pub use text::{append_chunk, clean_chunk, prompt_tail};
pub use transcribe::{CloudParams, keychain_endpoint, parse_credential_api_key, transcribe_cloud};
/// Whether `model`'s files are present in the directory configured in settings.
pub fn model_is_downloaded(settings: &VoiceInputSettings, model: &LocalModelDef) -> bool {
    let base = local::models_dir(settings);
    catalog::is_downloaded_in(&catalog::model_dir_in(&base, model), model)
}

pub use wav::{
    TARGET_SAMPLE_RATE, encode_wav, split_and_encode, split_mono_and_encode, to_transcription_wav,
};

/// Registers voice input settings with the settings store.
pub fn init(cx: &mut App) {
    VoiceInputSettings::register(cx);
    crate::downloads::init(cx);
}

/// Directory where captured audio is kept so a dictation survives a failed or
/// interrupted transcription.
pub fn recordings_dir() -> std::path::PathBuf {
    paths::data_dir().join("voice-recordings")
}

/// Writes each chunk to its own WAV file under [`recordings_dir`], returning the
/// paths that were written. Used to keep audio when a transcription fails or
/// when `voice_input.retain_audio` is enabled.
pub fn save_recordings(chunks: &[Vec<u8>]) -> Result<Vec<std::path::PathBuf>> {
    let directory = recordings_dir();
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("could not create {}", directory.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();

    let mut paths = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.iter().enumerate() {
        let path = directory.join(format!("dictation-{stamp}-part{}.wav", index + 1));
        std::fs::write(&path, chunk)
            .with_context(|| format!("could not write {}", path.display()))?;
        paths.push(path);
    }
    Ok(paths)
}

/// Transcribes 16 kHz mono WAV audio using the engine configured in `settings`.
///
/// `keychain_key` is an API key read from the system keychain when neither the
/// settings nor the environment variable provided one; it is only used by the
/// cloud and local-server engines. Local model execution runs on the background
/// executor because it shells out to a command-line binary.
///
/// `prompt_context` is the tail of the transcript produced so far. It is fed to
/// the backend as a prompt so spelling, terminology and sentence flow carry
/// across chunk boundaries, which is what keeps a long dictation coherent when
/// it is transcribed piece by piece.
pub fn transcribe(
    executor: &BackgroundExecutor,
    http_client: Arc<dyn HttpClient>,
    settings: VoiceInputSettings,
    keychain_key: Option<String>,
    prompt_context: Option<String>,
    wav: Vec<u8>,
) -> Task<Result<String>> {
    let background = executor.clone();
    executor.spawn(async move {
        let content = settings.content().clone();
        let prompt_context = prompt_context
            .map(|context| context.trim().to_string())
            .filter(|context| !context.is_empty());
        match content.engine() {
            VoiceInputEngine::Cloud => {
                let params = CloudParams::from_settings(&settings)?
                    .with_api_key(keychain_key)
                    .with_prompt_context(prompt_context);
                transcribe_cloud(&background, &*http_client, &params, &wav).await
            }
            VoiceInputEngine::Local => {
                let local = content.local();
                if local.engine() == VoiceInputLocalEngine::Server {
                    let params = CloudParams::from_local(&settings)?
                        .with_api_key(keychain_key)
                        .with_prompt_context(prompt_context);
                    transcribe_cloud(&background, &*http_client, &params, &wav).await
                } else {
                    transcribe_local(&settings, &wav, prompt_context.as_deref()).await
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_unset() {
        let content = VoiceInputSettingsContent::default();
        assert!(content.enabled());
        assert_eq!(content.engine(), VoiceInputEngine::Local);

        assert_eq!(content.max_duration_secs(), DEFAULT_MAX_DURATION_SECS);
        assert_eq!(content.min_duration_ms(), DEFAULT_MIN_DURATION_MS);
        assert_eq!(content.silence_timeout_secs(), DEFAULT_SILENCE_TIMEOUT_SECS);
        assert!((content.silence_threshold() - DEFAULT_SILENCE_THRESHOLD).abs() < f32::EPSILON);
        assert!(content.device().is_none());
    }

    #[test]
    fn cloud_defaults_select_groq_cheapest() {
        let cloud = VoiceInputSettingsContent::default().cloud();
        assert_eq!(cloud.provider(), DEFAULT_CLOUD_PRESET);
        assert_eq!(cloud.routing(), VoiceInputRouting::Cheapest);
        assert_eq!(cloud.timeout_secs(), DEFAULT_CLOUD_TIMEOUT_SECS);
        assert_eq!(cloud.retries(), DEFAULT_CLOUD_RETRIES);
        assert_eq!(cloud.provider_sort(), "price");
        assert!(cloud.allow_fallbacks());
    }

    #[test]
    fn local_defaults_are_whisper_cpp() {
        let local = VoiceInputSettingsContent::default().local();
        assert_eq!(local.engine(), VoiceInputLocalEngine::WhisperCpp);
        assert_eq!(local.model(), DEFAULT_LOCAL_MODEL);
        assert!(!local.auto_download());
        assert!(local.extra_args().is_empty());
        assert!(local.models_dir().is_none());
        assert!(local.model_path().is_none());
    }

    #[test]
    fn insertion_defaults_add_trailing_space_without_submitting() {
        let insertion = VoiceInputSettingsContent::default().insertion();
        assert!(insertion.trailing_space());
        assert!(!insertion.auto_submit());
    }

    #[test]
    fn explicit_overrides_win() {
        let content = VoiceInputSettingsContent {
            enabled: Some(false),
            engine: Some(VoiceInputEngine::Local),
            max_duration_secs: Some(0),
            device: Some("  ".to_string()),
            ..Default::default()
        };
        assert!(!content.enabled());
        assert_eq!(content.engine(), VoiceInputEngine::Local);
        // `0` means no cap, and blank device names fall back to the audio setting.
        assert_eq!(content.max_duration_secs(), 0);
        assert!(content.device().is_none());
    }
}
