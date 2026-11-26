use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::MergeFrom;

/// Default upper bound for a single dictation, in seconds. `0` disables the cap.
pub const DEFAULT_MAX_DURATION_SECS: u64 = 900;
/// Default length of a transcription chunk, in seconds. `0` disables chunking.
pub const DEFAULT_CHUNK_SECS: u64 = 120;
/// Default number of previous-transcript characters fed back to the next chunk
/// so context carries across chunk boundaries. `0` disables it.
pub const DEFAULT_CONTEXT_CHARS: usize = 200;
/// Recordings shorter than this are discarded instead of transcribed.
pub const DEFAULT_MIN_DURATION_MS: u64 = 300;
/// Stop automatically after this much silence. `0` disables automatic stopping.
pub const DEFAULT_SILENCE_TIMEOUT_SECS: u64 = 0;
/// Peak amplitude below which a frame counts as silence, in the `0.0..=1.0`
/// range of normalized samples.
pub const DEFAULT_SILENCE_THRESHOLD: f32 = 0.02;
/// Per-request timeout for cloud transcriptions.
pub const DEFAULT_CLOUD_TIMEOUT_SECS: u64 = 60;
/// Extra attempts after the first failed cloud transcription request.
pub const DEFAULT_CLOUD_RETRIES: u32 = 1;
/// Default local model identifier (must match a built-in catalog entry).
pub const DEFAULT_LOCAL_MODEL: &str = "whisper-large-v3-turbo";

/// Settings for dictating prompts into the agent panel with the microphone.
///
/// Audio is recorded locally and sent to the configured transcription backend.
/// With the `cloud` engine the audio leaves the machine; with `local` it does
/// not. Nothing is written to disk unless a local model has to be downloaded.
///
/// Every knob is optional; the documented defaults apply when a field is
/// omitted. They are listed here so the settings file is self-describing.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, MergeFrom)]
#[serde(rename_all = "snake_case")]
pub struct VoiceInputSettingsContent {
    /// Whether the microphone button is shown in the message editor and whether
    /// the `agent::ToggleVoiceInput` action does anything.
    ///
    /// Default: true
    pub enabled: Option<bool>,
    /// Where transcription runs.
    ///
    /// Default: cloud
    pub engine: Option<VoiceInputEngine>,
    /// Language hint passed to the backend, as an ISO-639-1 code such as `en`
    /// or `ru`. When unset the backend auto-detects the language.
    ///
    /// Default: auto-detect
    pub language: Option<String>,
    /// Input device to record from, overriding the microphone selected in the
    /// `audio` settings. Match the device name shown in your system mixer.
    ///
    /// Default: the `audio` settings device
    pub device: Option<String>,
    /// Maximum length of a single dictation, in seconds. When reached the
    /// recording stops and is transcribed. `0` means no limit.
    ///
    /// Default: 300
    pub max_duration_secs: Option<u64>,
    /// Recordings shorter than this are discarded instead of being sent to the
    /// backend, which avoids stray clicks producing empty transcriptions.
    ///
    /// Default: 300
    pub min_duration_ms: Option<u64>,
    /// Split long dictations into chunks of this many seconds before sending
    /// them for transcription. Chunking keeps each request small, so a slow or
    /// failing request loses at most one chunk, and lets the transcript appear
    /// incrementally instead of all at once. `0` sends the whole recording in a
    /// single request.
    ///
    /// Default: 120
    pub chunk_secs: Option<u64>,
    /// Keep the recorded audio on disk in `<data dir>/voice-recordings`.
    /// Audio from a failed transcription is always saved, regardless of this
    /// setting, so nothing that was spoken is lost.
    ///
    /// Default: false
    pub retain_audio: Option<bool>,
    /// How many characters of the previous transcript to send as a hint for the
    /// next chunk, so a long dictation keeps its context and spelling across
    /// chunk boundaries. `0` disables cross-chunk context.
    ///
    /// Default: 200
    pub context_chars: Option<usize>,
    /// Stop recording automatically after this many seconds of silence. `0`
    /// disables automatic stopping.
    ///
    /// Default: 0
    pub silence_timeout_secs: Option<u64>,
    /// Peak amplitude below which audio counts as silence. Normalized samples
    /// are in the `0.0..=1.0` range.
    ///
    /// Default: 0.02
    pub silence_threshold: Option<f32>,
    /// How the transcript is inserted into the editor.
    pub insertion: Option<VoiceInputInsertion>,
    /// Settings for the `cloud` engine.
    pub cloud: Option<VoiceInputCloudSettingsContent>,
    /// Settings for the `local` engine.
    pub local: Option<VoiceInputLocalSettingsContent>,
}

impl VoiceInputSettingsContent {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn engine(&self) -> VoiceInputEngine {
        self.engine.unwrap_or_default()
    }

    pub fn language(&self) -> Option<&str> {
        self.language
            .as_deref()
            .map(str::trim)
            .filter(|language| !language.is_empty() && *language != "auto")
    }

    pub fn device(&self) -> Option<&str> {
        self.device
            .as_deref()
            .map(str::trim)
            .filter(|device| !device.is_empty())
    }

    pub fn max_duration_secs(&self) -> u64 {
        self.max_duration_secs.unwrap_or(DEFAULT_MAX_DURATION_SECS)
    }

    pub fn min_duration_ms(&self) -> u64 {
        self.min_duration_ms.unwrap_or(DEFAULT_MIN_DURATION_MS)
    }

    pub fn chunk_secs(&self) -> u64 {
        self.chunk_secs.unwrap_or(DEFAULT_CHUNK_SECS)
    }

    pub fn retain_audio(&self) -> bool {
        self.retain_audio.unwrap_or(false)
    }

    pub fn context_chars(&self) -> usize {
        self.context_chars.unwrap_or(DEFAULT_CONTEXT_CHARS)
    }

    pub fn silence_timeout_secs(&self) -> u64 {
        self.silence_timeout_secs
            .unwrap_or(DEFAULT_SILENCE_TIMEOUT_SECS)
    }

    pub fn silence_threshold(&self) -> f32 {
        self.silence_threshold.unwrap_or(DEFAULT_SILENCE_THRESHOLD)
    }

    pub fn insertion(&self) -> VoiceInputInsertion {
        self.insertion.clone().unwrap_or_default()
    }

    pub fn cloud(&self) -> VoiceInputCloudSettingsContent {
        self.cloud.clone().unwrap_or_default()
    }

    pub fn local(&self) -> VoiceInputLocalSettingsContent {
        self.local.clone().unwrap_or_default()
    }
}

/// Which transcription backend is used.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, MergeFrom,
)]
#[serde(rename_all = "snake_case")]
pub enum VoiceInputEngine {
    /// An OpenAI-compatible transcription API reachable over the network.
    #[default]
    Cloud,
    /// A model running on this machine.
    Local,
}

/// How the transcript is placed in the message editor.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, MergeFrom)]
#[serde(rename_all = "snake_case")]
pub struct VoiceInputInsertion {
    /// Append a space after the inserted text so consecutive dictations do not
    /// run together.
    ///
    /// Default: true
    pub trailing_space: Option<bool>,
    /// Submit the message as soon as the transcript is inserted.
    ///
    /// Default: false
    pub auto_submit: Option<bool>,
    /// Insert each transcribed chunk into the editor as soon as it is ready,
    /// instead of waiting for the whole recording. This keeps a slow or failing
    /// backend from holding back text that has already been transcribed.
    ///
    /// Default: true
    pub streaming: Option<bool>,
}

impl VoiceInputInsertion {
    pub fn trailing_space(&self) -> bool {
        self.trailing_space.unwrap_or(true)
    }

    pub fn auto_submit(&self) -> bool {
        self.auto_submit.unwrap_or(false)
    }

    pub fn streaming(&self) -> bool {
        self.streaming.unwrap_or(true)
    }
}

/// Settings for transcribing through an OpenAI-compatible HTTP API.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, MergeFrom)]
#[serde(rename_all = "snake_case")]
pub struct VoiceInputCloudSettingsContent {
    /// Which built-in endpoint preset to use: `openai`, `groq`, `openrouter`, or
    /// `custom`. `custom` uses `endpoint` and `model` verbatim.
    ///
    /// Default: openrouter
    pub provider: Option<String>,
    /// Overrides the preset's transcription endpoint. The request is sent to
    /// `<endpoint>/audio/transcriptions`.
    pub endpoint: Option<String>,
    /// Overrides the preset's model name, for example `whisper-large-v3-turbo`.
    pub model: Option<String>,
    /// API key for the endpoint. When unset, `api_key_env` (or the preset's own
    /// environment variable) is consulted, and failing that the key stored in
    /// the system keychain under `endpoint`.
    pub api_key: Option<String>,
    /// Environment variable to read the API key from, overriding the preset's
    /// `OPENAI_API_KEY` / `GROQ_API_KEY` / `OPENROUTER_API_KEY`.
    pub api_key_env: Option<String>,
    /// Optional prompt passed to the model to bias spelling and vocabulary,
    /// for example a list of identifier names.
    pub prompt: Option<String>,
    /// Sampling temperature for the model, in the `0.0..=1.0` range.
    pub temperature: Option<f64>,
    /// Per-request timeout in seconds. `0` disables the timeout.
    ///
    /// Default: 60
    pub timeout_secs: Option<u64>,
    /// Extra attempts after the first failed request.
    ///
    /// Default: 1
    pub retries: Option<u32>,
    /// How a provider is chosen when several serve the configured model.
    ///
    /// Default: cheapest
    pub routing: Option<VoiceInputRouting>,
    /// Provider to use when `routing` is `manual`, for example the OpenRouter
    /// provider slug `Groq` or `DeepInfra`.
    pub provider_slug: Option<String>,
    /// Sort order advertised to an aggregator, such as OpenRouter's `price`,
    /// `throughput`, or `latency`.
    ///
    /// Default: price
    pub provider_sort: Option<String>,
    /// Whether the aggregator may fall back to another provider when the chosen
    /// one is unavailable.
    ///
    /// Default: true
    pub allow_fallbacks: Option<bool>,
    /// Additional headers sent with every request, for gateways that need them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<collections::HashMap<String, String>>,
}

impl VoiceInputCloudSettingsContent {
    pub fn provider(&self) -> &str {
        self.provider
            .as_deref()
            .map(str::trim)
            .filter(|provider| !provider.is_empty())
            .unwrap_or("openrouter")
    }

    pub fn routing(&self) -> VoiceInputRouting {
        self.routing.unwrap_or_default()
    }

    pub fn api_key_env(&self) -> Option<&str> {
        self.api_key_env
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
    }

    pub fn prompt(&self) -> Option<&str> {
        self.prompt
            .as_deref()
            .map(str::trim)
            .filter(|prompt| !prompt.is_empty())
    }

    pub fn provider_sort(&self) -> &str {
        self.provider_sort
            .as_deref()
            .map(str::trim)
            .filter(|sort| !sort.is_empty())
            .unwrap_or("price")
    }

    pub fn allow_fallbacks(&self) -> bool {
        self.allow_fallbacks.unwrap_or(true)
    }

    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs.unwrap_or(DEFAULT_CLOUD_TIMEOUT_SECS)
    }

    pub fn retries(&self) -> u32 {
        self.retries.unwrap_or(DEFAULT_CLOUD_RETRIES)
    }

    pub fn extra_headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.extra_headers
            .iter()
            .flatten()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

/// How to pick a serving provider for the configured transcription model.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, MergeFrom,
)]
#[serde(rename_all = "snake_case")]
pub enum VoiceInputRouting {
    /// Ask the aggregator which providers serve the model and use the one with
    /// the lowest price.
    #[default]
    Cheapest,
    /// Always use `provider_slug`.
    Manual,
}

/// Settings for transcribing with a model running on this machine.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, MergeFrom)]
#[serde(rename_all = "snake_case")]
pub struct VoiceInputLocalSettingsContent {
    /// How local models are executed.
    ///
    /// Default: whisper_cpp
    pub engine: Option<VoiceInputLocalEngine>,
    /// Which model to run. Built-in ids are listed by the microphone menu; any
    /// other value is passed to the engine unchanged.
    ///
    /// Default: whisper-large-v3-turbo
    pub model: Option<String>,
    /// Path to an existing model file or directory. When set it is used as-is,
    /// bypassing the built-in catalog and its download step, so any compatible
    /// model can be used.
    pub model_path: Option<String>,
    /// Directory to download built-in models into.
    ///
    /// Default: `<data dir>/voice-models`
    pub models_dir: Option<String>,
    /// Download the selected built-in model automatically the first time it is
    /// needed. When false the download must be started explicitly, which avoids
    /// unexpectedly fetching large files on a metered connection.
    ///
    /// Default: false
    pub auto_download: Option<bool>,
    /// Path to the engine executable. When unset the engine is looked up on
    /// `PATH` (`whisper-cli`, `whisper.cpp`, `main` for whisper.cpp;
    /// `sherpa-onnx-offline` for sherpa-onnx).
    pub binary_path: Option<String>,
    /// Number of threads the engine may use. When unset the engine default is
    /// used.
    pub threads: Option<usize>,
    /// Extra command-line arguments appended verbatim to the engine invocation.
    /// This is the escape hatch for engine options that are not modeled above.
    pub extra_args: Option<Vec<String>>,
    /// For the `server` engine: base URL of a local OpenAI-compatible server,
    /// such as `http://127.0.0.1:8000/v1`.
    pub endpoint: Option<String>,
}

impl VoiceInputLocalSettingsContent {
    pub fn engine(&self) -> VoiceInputLocalEngine {
        self.engine.unwrap_or_default()
    }

    pub fn model(&self) -> &str {
        self.model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .unwrap_or(DEFAULT_LOCAL_MODEL)
    }

    pub fn model_path(&self) -> Option<&str> {
        self.model_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
    }

    pub fn models_dir(&self) -> Option<&str> {
        self.models_dir
            .as_deref()
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
    }

    pub fn binary_path(&self) -> Option<&str> {
        self.binary_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
    }

    pub fn auto_download(&self) -> bool {
        self.auto_download.unwrap_or(false)
    }

    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint
            .as_deref()
            .map(str::trim)
            .filter(|endpoint| !endpoint.is_empty())
    }

    pub fn extra_args(&self) -> &[String] {
        self.extra_args.as_deref().unwrap_or(&[])
    }

    pub fn threads(&self) -> Option<usize> {
        self.threads.filter(|threads| *threads > 0)
    }
}

/// How a local transcription model is executed.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, MergeFrom,
)]
#[serde(rename_all = "snake_case")]
pub enum VoiceInputLocalEngine {
    /// The `whisper.cpp` command line tool with a `ggml-*.bin` model.
    #[default]
    WhisperCpp,
    /// The `sherpa-onnx-offline` command line tool, used for NVIDIA Parakeet
    /// and other NeMo transducer models.
    SherpaOnnx,
    /// A local OpenAI-compatible transcription server, for example
    /// `speaches` or `whisper.cpp`'s own HTTP server. Uses `endpoint` and
    /// `model`.
    Server,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> VoiceInputSettingsContent {
        serde_json::from_str(json).expect("valid voice_input settings")
    }

    #[test]
    fn defaults_are_documented_values() {
        let content = VoiceInputSettingsContent::default();
        assert!(content.enabled());
        assert_eq!(content.engine(), VoiceInputEngine::Cloud);

        assert_eq!(content.max_duration_secs(), DEFAULT_MAX_DURATION_SECS);
        assert_eq!(content.min_duration_ms(), DEFAULT_MIN_DURATION_MS);
        assert_eq!(content.chunk_secs(), DEFAULT_CHUNK_SECS);
        assert!(!content.retain_audio());
        assert_eq!(content.context_chars(), DEFAULT_CONTEXT_CHARS);
        assert_eq!(content.silence_timeout_secs(), DEFAULT_SILENCE_TIMEOUT_SECS);
        assert_eq!(content.silence_threshold(), DEFAULT_SILENCE_THRESHOLD);
        assert_eq!(content.insertion().trailing_space(), true);
        assert_eq!(content.insertion().auto_submit(), false);
        assert_eq!(content.insertion().streaming(), true);
        assert_eq!(content.cloud().provider(), "openrouter");
        assert_eq!(content.cloud().routing(), VoiceInputRouting::Cheapest);
        assert_eq!(content.cloud().timeout_secs(), DEFAULT_CLOUD_TIMEOUT_SECS);
        assert_eq!(content.cloud().retries(), DEFAULT_CLOUD_RETRIES);
        assert_eq!(content.cloud().provider_sort(), "price");
        assert!(content.cloud().allow_fallbacks());
        assert_eq!(content.local().engine(), VoiceInputLocalEngine::WhisperCpp);
        assert_eq!(content.local().model(), DEFAULT_LOCAL_MODEL);
        assert_eq!(content.local().auto_download(), false);
        assert!(content.local().extra_args().is_empty());
        assert!(content.local().threads().is_none());
    }

    #[test]
    fn every_knob_is_driven_by_settings() {
        let content = parse(
            r#"{
                "enabled": false,
                "engine": "local",

                "language": "ru",
                "device": "USB Mic",
                "max_duration_secs": 30,
                "min_duration_ms": 500,
                "chunk_secs": 45,
                "retain_audio": true,
                "context_chars": 50,
                "silence_timeout_secs": 2,
                "silence_threshold": 0.05,
                "insertion": { "trailing_space": false, "auto_submit": true, "streaming": false },
                "cloud": {
                    "provider": "groq",
                    "endpoint": "https://example.test/v1",
                    "model": "whisper-large-v3",
                    "api_key": "secret",
                    "api_key_env": "MY_KEY",
                    "prompt": "Zed, GPUI",
                    "temperature": 0.2,
                    "timeout_secs": 15,
                    "retries": 3,
                    "routing": "manual",
                    "provider_slug": "Groq",
                    "provider_sort": "throughput",
                    "allow_fallbacks": false,
                    "extra_headers": { "X-Gateway": "abc" }
                },
                "local": {
                    "engine": "sherpa_onnx",
                    "model": "parakeet-tdt-0.6b-v2",
                    "model_path": "/models/custom",
                    "models_dir": "/models",
                    "auto_download": true,
                    "binary_path": "/usr/local/bin/sherpa-onnx-offline",
                    "threads": 8,
                    "extra_args": ["--foo", "bar"],
                    "endpoint": "http://127.0.0.1:8000/v1"
                }
            }"#,
        );

        assert!(!content.enabled());
        assert_eq!(content.engine(), VoiceInputEngine::Local);

        assert_eq!(content.language(), Some("ru"));
        assert_eq!(content.device(), Some("USB Mic"));
        assert_eq!(content.max_duration_secs(), 30);
        assert_eq!(content.min_duration_ms(), 500);
        assert_eq!(content.chunk_secs(), 45);
        assert!(content.retain_audio());
        assert_eq!(content.context_chars(), 50);
        assert_eq!(content.silence_timeout_secs(), 2);
        assert_eq!(content.silence_threshold(), 0.05);
        assert!(!content.insertion().trailing_space());
        assert!(content.insertion().auto_submit());
        assert!(!content.insertion().streaming());

        let cloud = content.cloud();
        assert_eq!(cloud.provider(), "groq");
        assert_eq!(cloud.endpoint.as_deref(), Some("https://example.test/v1"));
        assert_eq!(cloud.model.as_deref(), Some("whisper-large-v3"));
        assert_eq!(cloud.api_key.as_deref(), Some("secret"));
        assert_eq!(cloud.api_key_env(), Some("MY_KEY"));
        assert_eq!(cloud.prompt(), Some("Zed, GPUI"));
        assert_eq!(cloud.temperature, Some(0.2));
        assert_eq!(cloud.timeout_secs(), 15);
        assert_eq!(cloud.retries(), 3);
        assert_eq!(cloud.routing(), VoiceInputRouting::Manual);
        assert_eq!(cloud.provider_slug.as_deref(), Some("Groq"));
        assert_eq!(cloud.provider_sort(), "throughput");
        assert!(!cloud.allow_fallbacks());
        assert_eq!(
            cloud.extra_headers().collect::<Vec<_>>(),
            vec![("X-Gateway", "abc")]
        );

        let local = content.local();
        assert_eq!(local.engine(), VoiceInputLocalEngine::SherpaOnnx);
        assert_eq!(local.model(), "parakeet-tdt-0.6b-v2");
        assert_eq!(local.model_path(), Some("/models/custom"));
        assert_eq!(local.models_dir(), Some("/models"));
        assert!(local.auto_download());
        assert_eq!(
            local.binary_path(),
            Some("/usr/local/bin/sherpa-onnx-offline")
        );
        assert_eq!(local.threads(), Some(8));
        assert_eq!(local.extra_args(), ["--foo".to_string(), "bar".to_string()]);
        assert_eq!(local.endpoint(), Some("http://127.0.0.1:8000/v1"));
    }

    #[test]
    fn blank_strings_fall_back_to_defaults() {
        let content = parse(
            r#"{
                "language": "  ",
                "device": "",
                "cloud": { "provider": "  ", "prompt": "   ", "provider_sort": "" },
                "local": { "model": "  ", "model_path": " ", "threads": 0 }
            }"#,
        );

        assert_eq!(content.language(), None);
        assert_eq!(content.device(), None);
        assert_eq!(content.cloud().provider(), "openrouter");
        assert_eq!(content.cloud().prompt(), None);
        assert_eq!(content.cloud().provider_sort(), "price");
        assert_eq!(content.local().model(), DEFAULT_LOCAL_MODEL);
        assert_eq!(content.local().model_path(), None);
        assert_eq!(content.local().threads(), None);
    }
}
