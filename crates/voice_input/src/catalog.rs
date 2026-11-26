//! Built-in cloud presets and local speech-recognition models.

use std::path::{Path, PathBuf};

/// A preset for an OpenAI-compatible transcription endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloudPreset {
    /// Identifier used in the `voice_input.cloud.provider` setting.
    pub id: &'static str,
    /// Human-readable name shown in the UI.
    pub name: &'static str,
    /// Base URL; the request is sent to `<endpoint>/audio/transcriptions`.
    pub endpoint: &'static str,
    /// Default model name for this provider.
    pub model: &'static str,
    /// Environment variable consulted when no key is configured.
    pub api_key_env: &'static str,
    /// Whether provider routing (`cheapest` / forced provider) is supported.
    pub supports_provider_routing: bool,
}

pub const CLOUD_PRESETS: &[CloudPreset] = &[
    CloudPreset {
        id: "openrouter",
        name: "OpenRouter (cheapest provider)",
        endpoint: "https://openrouter.ai/api/v1",
        model: "openai/whisper-large-v3-turbo",
        api_key_env: "OPENROUTER_API_KEY",
        supports_provider_routing: true,
    },
    CloudPreset {
        id: "groq",
        name: "Groq",
        endpoint: "https://api.groq.com/openai/v1",
        model: "whisper-large-v3-turbo",
        api_key_env: "GROQ_API_KEY",
        supports_provider_routing: false,
    },
    CloudPreset {
        id: "openai",
        name: "OpenAI",
        endpoint: "https://api.openai.com/v1",
        model: "whisper-1",
        api_key_env: "OPENAI_API_KEY",
        supports_provider_routing: false,
    },
    CloudPreset {
        id: "custom",
        name: "Custom OpenAI-compatible endpoint",
        endpoint: "",
        model: "",
        api_key_env: "MYZED_VOICE_API_KEY",
        supports_provider_routing: false,
    },
];

pub const DEFAULT_CLOUD_PRESET: &str = "openrouter";

/// Well-known providers on OpenRouter, offered for forced routing.
pub const OPENROUTER_PROVIDERS: &[(&str, &str)] = &[
    ("Groq", "Groq"),
    ("DeepInfra", "DeepInfra"),
    ("Fireworks", "Fireworks"),
    ("Together", "Together"),
    ("OpenAI", "OpenAI"),
    ("Cerebras", "Cerebras"),
];

pub fn cloud_preset(id: &str) -> Option<&'static CloudPreset> {
    CLOUD_PRESETS.iter().find(|preset| preset.id == id)
}

/// How a local model is executed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalEngine {
    /// A `whisper.cpp` command-line binary.
    WhisperCpp,
    /// A `sherpa-onnx` command-line binary.
    SherpaOnnx,
}

/// A local model that can be downloaded and run on this machine.
#[derive(Clone, Copy, Debug)]
pub struct LocalModelDef {
    /// Identifier used in the `voice_input.local.model` setting.
    pub id: &'static str,
    /// Human-readable name shown in the UI.
    pub name: &'static str,
    pub engine: LocalEngine,
    /// Approximate download size, for display only.
    pub approx_size: &'static str,
    /// Files fetched into the model directory. Either a single model file or a
    /// `.tar.bz2` archive that is unpacked into the model directory.
    pub download: Download,
}

#[derive(Clone, Copy, Debug)]
pub enum Download {
    /// Downloaded to `<model_dir>/<file_name>`.
    File {
        url: &'static str,
        file_name: &'static str,
    },
    /// Downloaded and unpacked in place with the system `tar`.
    Archive {
        url: &'static str,
        file_name: &'static str,
    },
}

const fn whisper_cpp_model(url: &'static str, file_name: &'static str) -> Download {
    Download::File { url, file_name }
}

pub const LOCAL_MODELS: &[LocalModelDef] = &[
    LocalModelDef {
        id: "parakeet-tdt-0.6b-v2",
        name: "Parakeet TDT 0.6B v2 (sherpa-onnx, int8)",
        engine: LocalEngine::SherpaOnnx,
        approx_size: "~660 MB",
        download: Download::Archive {
            url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2",
            file_name: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2",
        },
    },
    LocalModelDef {
        id: "whisper-large-v3-turbo",
        name: "Whisper Large v3 Turbo (whisper.cpp)",
        engine: LocalEngine::WhisperCpp,
        approx_size: "~1.6 GB",
        download: whisper_cpp_model(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
            "ggml-large-v3-turbo.bin",
        ),
    },
    LocalModelDef {
        id: "whisper-large-v3",
        name: "Whisper Large v3 (whisper.cpp)",
        engine: LocalEngine::WhisperCpp,
        approx_size: "~3.1 GB",
        download: whisper_cpp_model(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
            "ggml-large-v3.bin",
        ),
    },
    LocalModelDef {
        id: "whisper-medium",
        name: "Whisper Medium (whisper.cpp)",
        engine: LocalEngine::WhisperCpp,
        approx_size: "~1.5 GB",
        download: whisper_cpp_model(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
            "ggml-medium.bin",
        ),
    },
    LocalModelDef {
        id: "whisper-small",
        name: "Whisper Small (whisper.cpp)",
        engine: LocalEngine::WhisperCpp,
        approx_size: "~466 MB",
        download: whisper_cpp_model(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
            "ggml-small.bin",
        ),
    },
    LocalModelDef {
        id: "whisper-base",
        name: "Whisper Base (whisper.cpp)",
        engine: LocalEngine::WhisperCpp,
        approx_size: "~142 MB",
        download: whisper_cpp_model(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
            "ggml-base.bin",
        ),
    },
    LocalModelDef {
        id: "whisper-tiny",
        name: "Whisper Tiny (whisper.cpp)",
        engine: LocalEngine::WhisperCpp,
        approx_size: "~75 MB",
        download: whisper_cpp_model(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
            "ggml-tiny.bin",
        ),
    },
];

pub const DEFAULT_LOCAL_MODEL: &str = "whisper-large-v3-turbo";

pub fn local_model(id: &str) -> Option<&'static LocalModelDef> {
    LOCAL_MODELS.iter().find(|model| model.id == id)
}

/// Directory that holds downloaded local models.
pub fn models_dir() -> PathBuf {
    default_models_dir()
}

/// The built-in download directory used when no override is configured.
pub fn default_models_dir() -> PathBuf {
    paths::data_dir().join("voice-models")
}

/// Directory for a specific local model.
pub fn model_dir(model: &LocalModelDef) -> PathBuf {
    model_dir_in(&models_dir(), model)
}

/// Directory for `model` inside a caller-supplied base directory.
pub fn model_dir_in(base: &Path, model: &LocalModelDef) -> PathBuf {
    base.join(model.id)
}

/// Whether the files for `model` are already present.
pub fn is_downloaded(model: &LocalModelDef) -> bool {
    is_downloaded_in(&model_dir(model), model)
}

/// Whether the files for the local model `id` are already present.
pub fn is_downloaded_by_id(id: &str) -> bool {
    local_model(id).is_some_and(is_downloaded)
}

/// Whether the files for `model` are present in `dir`.
pub fn is_downloaded_in(dir: &Path, model: &LocalModelDef) -> bool {
    match model.download {
        Download::File { file_name, .. } => dir.join(file_name).is_file(),
        Download::Archive { .. } => {
            // Unpacked archives contain a single top-level directory.
            dir.is_dir()
                && std::fs::read_dir(dir)
                    .map(|mut entries| entries.next().is_some())
                    .unwrap_or(false)
        }
    }
}

/// Resolves the executable used for a local engine.
///
/// An explicit `binary_path` setting wins; otherwise the usual binary names are
/// looked up in `PATH`.
pub fn resolve_binary(engine: LocalEngine, configured: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(configured) = configured {
        if configured.is_file() {
            return Ok(configured.to_path_buf());
        }
        anyhow::bail!("configured binary {} does not exist", configured.display());
    }

    let candidates: &[&str] = match engine {
        LocalEngine::WhisperCpp => &["whisper-cli", "whisper-cpp", "main"],
        LocalEngine::SherpaOnnx => &["sherpa-onnx-offline"],
    };

    for candidate in candidates {
        if let Some(path) = which(candidate) {
            return Ok(path);
        }
    }

    anyhow::bail!(
        "could not find {} in PATH; set `voice_input.local.binary_path` in your settings",
        candidates.join(" / ")
    )
}

fn which(program: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .map(|extension| extension.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };

    for directory in std::env::split_paths(&path_var) {
        for extension in &extensions {
            let candidate = if extension.is_empty() {
                directory.join(program)
            } else {
                directory.join(format!("{program}{extension}"))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}
