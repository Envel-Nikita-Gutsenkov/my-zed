//! Transcription through OpenAI-compatible HTTP endpoints.
//!
//! The same multipart request shape is accepted by OpenAI, Groq and most
//! self-hosted servers, so one implementation covers all of them. For
//! aggregators that host the same model on several providers, the request can
//! be pinned to the cheapest (or a user-chosen) provider.

use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use futures::AsyncReadExt as _;
use http_client::{AsyncBody, HttpClient, Method, Request};

use crate::settings::VoiceInputSettings;
use crate::{VoiceInputEngine, VoiceInputRouting};

/// Resolved parameters for a cloud transcription request.
#[derive(Clone, Debug)]
pub struct CloudParams {
    /// Base URL, without the `/audio/transcriptions` suffix.
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    pub language: Option<String>,
    /// Optional prompt used to bias spelling and vocabulary.
    pub prompt: Option<String>,
    pub temperature: Option<f64>,
    /// Whether the endpoint aggregates multiple providers.
    pub supports_provider_routing: bool,
    pub forced_provider: Option<String>,
    pub pick_cheapest_provider: bool,
    pub provider_sort: String,
    pub allow_fallbacks: bool,
    pub timeout: Option<Duration>,
    pub retries: u32,
    pub extra_headers: Vec<(String, String)>,
}

impl CloudParams {
    pub fn from_settings(settings: &VoiceInputSettings) -> Result<Self> {
        let content = settings.content();
        let cloud = content.cloud();
        let preset = crate::catalog::cloud_preset(cloud.provider()).ok_or_else(|| {
            anyhow!(
                "unknown `voice_input.cloud.provider` {:?}; expected one of: {}",
                cloud.provider(),
                crate::catalog::CLOUD_PRESETS
                    .iter()
                    .map(|preset| preset.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

        let endpoint = cloud
            .endpoint
            .clone()
            .filter(|endpoint| !endpoint.trim().is_empty())
            .unwrap_or_else(|| preset.endpoint.to_string())
            .trim_end_matches('/')
            .to_string();
        if endpoint.is_empty() {
            anyhow::bail!(
                "`voice_input.cloud.endpoint` is required for the custom provider preset"
            );
        }

        let model = cloud
            .model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| preset.model.to_string());
        if model.is_empty() {
            anyhow::bail!("`voice_input.cloud.model` is required for the custom provider preset");
        }

        let from_env = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty())
        };
        let api_key = cloud
            .api_key
            .clone()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .or_else(|| {
                cloud
                    .api_key_env()
                    .and_then(from_env)
                    .or_else(|| from_env(preset.api_key_env))
            });

        let forced_provider = cloud
            .provider_slug
            .as_deref()
            .map(str::trim)
            .filter(|slug| !slug.is_empty())
            .map(str::to_string);
        let routing = cloud.routing();
        let timeout_secs = cloud.timeout_secs();

        Ok(Self {
            endpoint,
            model,
            api_key,
            language: content.language().map(str::to_string),
            prompt: cloud.prompt().map(str::to_string),
            temperature: cloud.temperature,
            supports_provider_routing: preset.supports_provider_routing,
            pick_cheapest_provider: preset.supports_provider_routing
                && routing == VoiceInputRouting::Cheapest
                && forced_provider.is_none(),
            forced_provider,
            provider_sort: cloud.provider_sort().to_string(),
            allow_fallbacks: cloud.allow_fallbacks(),
            timeout: (timeout_secs > 0).then(|| Duration::from_secs(timeout_secs)),
            retries: cloud.retries(),
            extra_headers: cloud
                .extra_headers
                .clone()
                .unwrap_or_default()
                .into_iter()
                .collect(),
        })
    }

    /// Parameters for the local `server` engine, which speaks the same protocol
    /// but runs on this machine.
    pub fn from_local(settings: &VoiceInputSettings) -> Result<Self> {
        let content = settings.content();
        let local = content.local();
        let endpoint = local
            .endpoint()
            .ok_or_else(|| {
                anyhow!(
                    "`voice_input.local.endpoint` is required when `voice_input.local.engine` is `server`"
                )
            })?
            .trim_end_matches('/')
            .to_string();

        Ok(Self {
            endpoint,
            model: local.model().to_string(),
            api_key: None,
            language: content.language().map(str::to_string),
            prompt: None,
            temperature: None,
            supports_provider_routing: false,
            forced_provider: None,
            pick_cheapest_provider: false,
            provider_sort: "price".to_string(),
            allow_fallbacks: true,
            timeout: None,
            retries: 0,
            extra_headers: Vec::new(),
        })
    }

    pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
        if self.api_key.is_none() {
            self.api_key = api_key;
        }
        self
    }

    /// Appends cross-chunk context to the configured prompt so sentence flow and
    /// spelling carry over from the previous chunks of the same dictation.
    pub fn with_prompt_context(mut self, context: Option<String>) -> Self {
        if let Some(context) = context {
            let context = context.trim();
            if context.is_empty() {
                return self;
            }
            self.prompt = Some(match self.prompt.take() {
                Some(prompt) if !prompt.trim().is_empty() => format!("{prompt} {context}"),
                _ => context.to_string(),
            });
        }
        self
    }

    fn form_fields(&self, provider_pin: Option<String>) -> Vec<(&'static str, String)> {
        let mut fields: Vec<(&'static str, String)> = vec![("model", self.model.clone())];
        if let Some(language) = self.language.as_deref() {
            fields.push(("language", language.to_string()));
        }
        fields.push(("response_format", "json".to_string()));
        if let Some(prompt) = self.prompt.as_deref() {
            fields.push(("prompt", prompt.to_string()));
        }
        if let Some(temperature) = self.temperature {
            fields.push(("temperature", temperature.to_string()));
        }
        if self.supports_provider_routing {
            let mut provider = serde_json::json!({
                "sort": self.provider_sort,
                "allow_fallbacks": self.allow_fallbacks,
            });
            if let Some(pin) = provider_pin {
                provider["only"] = serde_json::json!([pin]);
            }
            fields.push(("provider", provider.to_string()));
        }
        fields
    }
}

/// Transcribes 16 kHz mono WAV audio.
///
/// `executor` provides the timer used to bound each request when
/// `voice_input.cloud.timeout_secs` is non-zero.
pub async fn transcribe_cloud(
    executor: &gpui::BackgroundExecutor,
    client: &dyn HttpClient,
    params: &CloudParams,
    wav: &[u8],
) -> Result<String> {
    // Aggregators let the request name the provider to use. A pin is useful for
    // cost, but it hard-fails when the account is not allowed to use that
    // provider, so an unpinned attempt always follows a pinned one.
    let mut pins: Vec<Option<String>> = Vec::new();
    if let Some(forced) = params.forced_provider.clone() {
        pins.push(Some(forced));
    } else if params.pick_cheapest_provider {
        match cheapest_provider(client, params).await {
            Ok(Some(pin)) => pins.push(Some(pin)),
            Ok(None) => {}
            Err(error) => log::warn!(
                "voice input: could not determine the cheapest provider, routing freely: {error:#}"
            ),
        }
    }
    if params.supports_provider_routing && !pins.iter().any(Option::is_none) {
        pins.push(None);
    }

    let url = format!("{}/audio/transcriptions", params.endpoint);
    let attempts = params.retries.saturating_add(1);
    let mut last_error = None;

    for pin in pins {
        let fields = params.form_fields(pin.clone());
        let (content_type, body) = multipart_bytes(&fields, "file", "audio.wav", "audio/wav", wav);
        for attempt in 0..attempts {
            match send_transcription(executor, client, params, &url, &content_type, &body).await {
                Ok(text) => return Ok(text),
                Err(error) => {
                    let transient = is_transient(&error);
                    if attempt + 1 < attempts && transient {
                        log::warn!(
                            "voice input: transcription attempt {} of {} (provider {pin:?}) failed: {error:#}",
                            attempt + 1,
                            attempts
                        );
                    }
                    last_error = Some(error);
                    if !transient {
                        // The request itself is wrong; retrying or repinning will
                        // not help, so fall through to the unpinned attempt if any.
                        break;
                    }
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("transcription request was not attempted")))
}

async fn send_transcription(
    executor: &gpui::BackgroundExecutor,
    client: &dyn HttpClient,
    params: &CloudParams,
    url: &str,
    content_type: &str,
    body: &[u8],
) -> Result<String> {
    let attempt = async {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(url)
            .header("Content-Type", content_type);
        if let Some(api_key) = params.api_key.as_deref() {
            request = request.header("Authorization", format!("Bearer {}", api_key.trim()));
        }
        for (name, value) in &params.extra_headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let request = request
            .body(AsyncBody::from(body.to_vec()))
            .with_context(|| format!("invalid transcription endpoint {url}"))?;

        let mut response = client
            .send(request)
            .await
            .with_context(|| format!("could not reach {url}"))?;

        let status = response.status();
        let mut body = Vec::new();
        response
            .body_mut()
            .read_to_end(&mut body)
            .await
            .context("could not read the transcription response")?;
        let text = String::from_utf8_lossy(&body).into_owned();

        if !status.is_success() {
            return Err(TranscriptionFailure {
                status: status.as_u16(),
                body: text.trim().chars().take(500).collect(),
            }
            .into());
        }

        parse_transcription(&text)
    };

    match params.timeout {
        Some(timeout) => {
            futures::pin_mut!(attempt);
            let timer = executor.timer(timeout);
            futures::pin_mut!(timer);
            match futures::future::select(attempt, timer).await {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right(_) => Err(anyhow!(
                    "transcription timed out after {}s",
                    timeout.as_secs()
                )),
            }
        }
        None => attempt.await,
    }
}

/// A transcription request the endpoint rejected.
#[derive(Debug)]
struct TranscriptionFailure {
    status: u16,
    body: String,
}

impl std::fmt::Display for TranscriptionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the transcription request was rejected with HTTP {}",
            self.status
        )?;
        let body = self.body.replace('\n', " ");
        let body = body.trim();
        if !body.is_empty() {
            write!(formatter, ": {body}")?;
        }
        if let Some(hint) = status_hint(self.status) {
            write!(formatter, "\n\n{hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for TranscriptionFailure {}

/// A short, actionable explanation for the statuses a user can actually fix.
fn status_hint(status: u16) -> Option<&'static str> {
    Some(match status {
        401 => {
            "The API key was rejected. Check `voice_input.cloud.api_key` and that the key is stored in your keychain."
        }
        403 => {
            "The account is not allowed to use this model or provider. Check the model name and the allow-list for your API key."
        }
        404 => {
            "The endpoint or model was not found. Check `voice_input.cloud.endpoint` and `voice_input.cloud.model`."
        }
        413 => "The recording is too large for this endpoint. Lower `voice_input.chunk_secs`.",
        429 => {
            "Rate limited or out of credit. Check your balance, and raise `voice_input.cloud.retries` if the limit is transient."
        }
        _ => return None,
    })
}

/// Whether a failed request is worth repeating.
///
/// Client errors other than rate limits and timeouts will fail the same way no
/// matter how often they are retried.
fn is_transient(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<TranscriptionFailure>() {
        Some(failure) => matches!(failure.status, 408 | 425 | 429) || failure.status >= 500,
        None => true,
    }
}

/// Extracts the transcript from a JSON response, falling back to plain text.
fn parse_transcription(body: &str) -> Result<String> {
    let trimmed = body.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(text) = value.get("text").and_then(|text| text.as_str()) {
            return Ok(text.trim().to_string());
        }
        if let Some(error) = value.get("error") {
            anyhow::bail!("transcription endpoint returned an error: {error}");
        }
    }
    if trimmed.is_empty() {
        anyhow::bail!("transcription endpoint returned an empty response");
    }
    Ok(trimmed.to_string())
}

/// Finds the cheapest provider serving `params.model` on an aggregator.
async fn cheapest_provider(
    client: &dyn HttpClient,
    params: &CloudParams,
) -> Result<Option<String>> {
    let url = format!("{}/models/{}/endpoints", params.endpoint, params.model);
    let mut request = Request::builder().method(Method::GET).uri(&url);
    if let Some(api_key) = params.api_key.as_deref() {
        request = request.header("Authorization", format!("Bearer {}", api_key.trim()));
    }
    let request = request
        .body(AsyncBody::empty())
        .with_context(|| format!("invalid endpoint list URL {url}"))?;

    let mut response = client
        .send(request)
        .await
        .with_context(|| format!("could not reach {url}"))?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut body)
        .await
        .context("could not read the provider list")?;
    let body = String::from_utf8_lossy(&body).into_owned();

    if !status.is_success() {
        anyhow::bail!(
            "provider lookup failed with {status}: {}",
            body.trim().chars().take(500).collect::<String>()
        );
    }

    let value: serde_json::Value =
        serde_json::from_str(&body).context("provider list was not valid JSON")?;
    Ok(pick_cheapest_provider(&value))
}

fn pick_cheapest_provider(value: &serde_json::Value) -> Option<String> {
    let endpoints = value
        .get("data")
        .and_then(|data| data.get("endpoints"))
        .and_then(|endpoints| endpoints.as_array())?;

    endpoints
        .iter()
        .map(|endpoint| {
            let name = endpoint
                .get("provider_name")
                .or_else(|| endpoint.get("name"))
                .and_then(|name| name.as_str())
                .unwrap_or_default()
                .to_string();
            let price = endpoint.get("pricing").map(price_score).unwrap_or(f64::MAX);
            (name, price)
        })
        .filter(|(name, _)| !name.is_empty())
        .min_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(name, _)| name)
}

/// Extracts an API key from credentials previously saved for an endpoint.
///
/// Zed stores credentials as a JSON object with an `api_key` field, but plain
/// strings are also accepted for keys written by other tools.
pub fn parse_credential_api_key(bytes: &[u8]) -> Option<String> {
    let raw = String::from_utf8_lossy(bytes).trim().to_string();
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&raw) {
        if let Some(key) = map.get("api_key").and_then(|value| value.as_str()) {
            let key = key.trim();
            if !key.is_empty() {
                return Some(key.to_string());
            }
        }
    }
    (!raw.is_empty()).then_some(raw)
}

/// Endpoint whose keychain entry may hold an API key for the active engine.
///
/// Cloud engines key off the transcription endpoint; the local `server` engine
/// keys off the user-provided URL. Local binaries need no key, so this returns
/// `None` for them.
pub fn keychain_endpoint(settings: &VoiceInputSettings) -> Option<String> {
    match settings.content().engine() {
        VoiceInputEngine::Cloud => CloudParams::from_settings(settings)
            .ok()
            .map(|params| params.endpoint),
        VoiceInputEngine::Local => settings.content().local().endpoint,
    }
}

/// Ranks a pricing object by the most specific cost it advertises.
fn price_score(pricing: &serde_json::Value) -> f64 {
    for key in ["audio", "request", "per_second", "prompt"] {
        let value = pricing.get(key).and_then(|value| match value {
            serde_json::Value::String(text) => text.parse::<f64>().ok(),
            serde_json::Value::Number(number) => number.as_f64(),
            _ => None,
        });
        if let Some(value) = value {
            return value;
        }
    }
    f64::MAX
}

/// Builds a multipart/form-data body.
fn multipart_bytes(
    fields: &[(&str, String)],
    file_field: &str,
    file_name: &str,
    file_mime: &str,
    file_bytes: &[u8],
) -> (String, Vec<u8>) {
    let boundary = format!(
        "----myzedvoice{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );

    let mut body = Vec::with_capacity(file_bytes.len() + 512);
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{file_field}\"; filename=\"{file_name}\"\r\n\
             Content-Type: {file_mime}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_and_plain_text_responses() {
        assert_eq!(
            parse_transcription(r#"{"text":" hello "}"#).unwrap(),
            "hello"
        );
        assert_eq!(parse_transcription("hello world").unwrap(), "hello world");
        assert!(parse_transcription("  ").is_err());
    }

    #[test]
    fn picks_the_lowest_priced_endpoint() {
        let value = serde_json::json!({
            "data": { "endpoints": [
                { "provider_name": "Expensive", "pricing": { "audio": "0.0001" } },
                { "provider_name": "Cheap", "pricing": { "audio": "0.00001" } },
                { "provider_name": "NoPrice", "pricing": {} },
            ]}
        });
        assert_eq!(pick_cheapest_provider(&value), Some("Cheap".to_string()));
    }

    #[test]
    fn parses_api_key_from_credential_bytes() {
        assert_eq!(
            parse_credential_api_key(br#"{"api_key":" sk-123 "}"#).as_deref(),
            Some("sk-123")
        );
        assert_eq!(
            parse_credential_api_key(b"  raw-key  ").as_deref(),
            Some("raw-key")
        );
        assert_eq!(parse_credential_api_key(b"   "), None);
    }

    #[test]
    fn multipart_body_contains_fields_and_file() {
        let (content_type, body) = multipart_bytes(
            &[("model", "whisper-1".to_string())],
            "file",
            "audio.wav",
            "audio/wav",
            b"RIFF",
        );
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains(&format!("--{boundary}")));
        assert!(body.contains("name=\"model\""));
        assert!(body.contains("whisper-1"));
        assert!(body.contains("filename=\"audio.wav\""));
        assert!(body.contains("RIFF"));
        assert!(body.trim_end().ends_with(&format!("--{boundary}--")));
    }

    #[test]
    fn prompt_context_appends_to_the_configured_prompt() {
        let params = CloudParams::from_settings(&VoiceInputSettings::default())
            .unwrap()
            .with_prompt_context(Some("Zed, GPUI".to_string()));
        assert_eq!(params.prompt.as_deref(), Some("Zed, GPUI"));

        let mut with_prompt = params;
        with_prompt.prompt = Some("vocabulary".to_string());
        let combined = with_prompt
            .clone()
            .with_prompt_context(Some("prior text".to_string()));
        assert_eq!(combined.prompt.as_deref(), Some("vocabulary prior text"));

        let blank = with_prompt.with_prompt_context(Some("   ".to_string()));
        assert_eq!(blank.prompt.as_deref(), Some("vocabulary"));
    }
}
