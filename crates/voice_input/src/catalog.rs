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

pub const DEFAULT_CLOUD_PRESET: &str = "groq";

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
    /// One-line description of the tradeoffs shown in the model manager.
    pub description: &'static str,
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
        description: "Fastest English model. Best speed/accuracy balance on CPU.",
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
        description: "Recommended default. Multilingual, near-large accuracy at turbo speed.",
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
        description: "Highest accuracy, slowest on CPU. Use only if you have a fast machine.",
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
        description: "Balanced multilingual accuracy at a moderate cost.",
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
        description: "Good accuracy at moderate speed. Fits comfortably in RAM.",
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
        description: "Fast and low-memory, lower accuracy.",
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
        description: "Smallest and fastest, lowest accuracy. Good for quick notes.",
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

/// whisper.cpp release that ships prebuilt command-line binaries.
///
/// Upstream stopped attaching binaries to the newest tags, so a release known to
/// carry them is pinned instead of following `latest`.
const WHISPER_CPP_RELEASE: &str = "v1.9.2";

/// Executable names whisper.cpp and sherpa-onnx use across platforms.
pub const WHISPER_BINARY_NAMES: &[&str] = &["whisper-cli", "whisper-cpp", "main"];
pub const SHERPA_BINARY_NAMES: &[&str] = &["sherpa-onnx-offline"];

/// Archive with prebuilt whisper.cpp binaries for the current platform.
#[derive(Clone, Debug)]
pub struct WhisperBinaryArchive {
    pub url: String,
    pub file_name: &'static str,
}

/// Download for the managed whisper.cpp binary, or `None` on platforms where
/// upstream publishes no build (macOS).
pub fn whisper_binary_archive() -> Option<WhisperBinaryArchive> {
    let asset = if cfg!(target_os = "windows") {
        // Upstream ships no arm64 Windows build for the pinned release, but the
        // x64 binary runs under Windows' emulation on arm64 machines.
        if cfg!(target_arch = "x86") {
            "whisper-bin-Win32.zip"
        } else {
            "whisper-bin-x64.zip"
        }
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") {
            "whisper-bin-ubuntu-arm64.tar.gz"
        } else {
            "whisper-bin-ubuntu-x64.tar.gz"
        }
    } else {
        return None;
    };

    Some(WhisperBinaryArchive {
        url: format!(
            "https://github.com/ggml-org/whisper.cpp/releases/download/{WHISPER_CPP_RELEASE}/{asset}"
        ),
        file_name: asset,
    })
}

/// Directory the app installs local engine binaries into.
pub fn binaries_dir() -> PathBuf {
    paths::data_dir().join("voice-engines")
}

/// Finds a whisper.cpp binary, checking the managed install directory and then
/// `PATH`.
pub fn whisper_binary() -> Option<PathBuf> {
    find_executable(&binaries_dir(), WHISPER_BINARY_NAMES).or_else(|| {
        WHISPER_BINARY_NAMES
            .iter()
            .find_map(|name| which(name))
    })
}

/// Recursively searches `dir` for a file named like one of `names`.
pub fn find_executable(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    // Unpacked archives keep their files in a single top-level directory, so the
    // search has to be shallow but tolerant of that extra level.
    fn search(dir: &Path, names: &[&str], depth: usize) -> Option<PathBuf> {
        let mut directories = Vec::new();
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth > 0 {
                    directories.push(path);
                }
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if names.iter().any(|name| name.eq_ignore_ascii_case(stem)) {
                return Some(path);
            }
        }
        directories
            .into_iter()
            .find_map(|directory| search(&directory, names, depth - 1))
    }

    search(dir, names, 3)
}

/// Resolves the executable used for a local engine.
///
/// An explicit `binary_path` setting wins; then the binary installed by the app;
/// then the usual binary names looked up in `PATH`.
pub fn resolve_binary(engine: LocalEngine, configured: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(configured) = configured {
        if configured.is_file() {
            return Ok(configured.to_path_buf());
        }
        anyhow::bail!("configured binary {} does not exist", configured.display());
    }

    let candidates: &[&str] = match engine {
        LocalEngine::WhisperCpp => WHISPER_BINARY_NAMES,
        LocalEngine::SherpaOnnx => SHERPA_BINARY_NAMES,
    };

    if let Some(path) = find_executable(&binaries_dir(), candidates) {
        return Ok(path);
    }

    for candidate in candidates {
        if let Some(path) = which(candidate) {
            return Ok(path);
        }
    }

    anyhow::bail!(
        "could not find {} in PATH; install it from Settings → Voice Input or set \
         `voice_input.local.binary_path`",
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
