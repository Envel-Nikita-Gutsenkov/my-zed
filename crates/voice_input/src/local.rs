//! Local (on-device) transcription.
//!
//! Models are downloaded into the app data directory (or a directory chosen in
//! settings) and executed through a command-line binary (`whisper.cpp` or
//! `sherpa-onnx`). Downloads stream to disk while reporting progress, and
//! archives are unpacked with the system `tar` (bsdtar on Windows 10+, GNU tar
//! elsewhere).
//!
//! Nothing about the invocation is hardcoded beyond the arguments the engines
//! require: the model, binary, thread count, and any extra arguments all come
//! from `voice_input.local` settings.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context as _, Result, anyhow};
use futures::AsyncReadExt as _;
use http_client::{AsyncBody, HttpClient, Method, Request};
use smol::process::Command;
use util::ResultExt as _;

use crate::VoiceInputLocalEngine;
use crate::catalog::{self, Download, LocalEngine, LocalModelDef};
use crate::settings::VoiceInputSettings;

/// Shared counters for an in-progress model download.
#[derive(Clone, Debug, Default)]
pub struct DownloadProgress {
    pub downloaded: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
}

impl DownloadProgress {
    pub fn downloaded(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Fraction in `0.0..=1.0`, or `None` when the size is unknown.
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total();
        (total > 0).then(|| (self.downloaded() as f32 / total as f32).min(1.0))
    }
}

/// Directory that built-in models are downloaded into, honoring
/// `voice_input.local.models_dir`.
pub fn models_dir(settings: &VoiceInputSettings) -> PathBuf {
    let local = settings.content().local();
    local
        .models_dir()
        .map(PathBuf::from)
        .unwrap_or_else(catalog::default_models_dir)
}

/// Ensures the model is present on disk, downloading it if necessary.
///
/// Returns the primary path of the model: the model file for whisper.cpp, or
/// the unpacked model directory for sherpa-onnx.
pub async fn ensure_model(
    model: &LocalModelDef,
    base_dir: &Path,
    client: &dyn HttpClient,
    progress: &DownloadProgress,
) -> Result<PathBuf> {
    let dir = catalog::model_dir_in(base_dir, model);
    if catalog::is_downloaded_in(&dir, model) {
        return primary_path(model, &dir);
    }

    std::fs::create_dir_all(&dir)
        .with_context(|| format!("could not create model directory {}", dir.display()))?;

    match model.download {
        Download::File { url, file_name } => {
            let destination = dir.join(file_name);
            let partial = dir.join(format!("{file_name}.part"));
            download_to_file(client, url, &partial, progress).await?;
            std::fs::rename(&partial, &destination).with_context(|| {
                format!(
                    "could not move the downloaded model to {}",
                    destination.display()
                )
            })?;
            Ok(destination)
        }
        Download::Archive { url, file_name } => {
            let archive_path = dir.join(file_name);
            download_to_file(client, url, &archive_path, progress).await?;
            extract_archive(&archive_path, &dir).await?;
            std::fs::remove_file(&archive_path).log_err();
            primary_path(model, &dir)
        }
    }
}

fn primary_path(model: &LocalModelDef, dir: &Path) -> Result<PathBuf> {
    match model.download {
        Download::File { file_name, .. } => Ok(dir.join(file_name)),
        Download::Archive { .. } => {
            // Unpacked archives contain a single top-level directory that holds
            // the model files; sherpa's engine reads them from there.
            let entry = std::fs::read_dir(dir)
                .with_context(|| format!("could not read {}", dir.display()))?
                .filter_map(|entry| entry.ok())
                .find(|entry| entry.path().is_dir())
                .ok_or_else(|| anyhow!("no model directory found in {}", dir.display()))?;
            Ok(entry.path())
        }
    }
}

pub(crate) async fn download_to_file(
    client: &dyn HttpClient,
    url: &str,
    destination: &Path,
    progress: &DownloadProgress,
) -> Result<()> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(url)
        .body(AsyncBody::empty())
        .with_context(|| format!("invalid download URL {url}"))?;

    let mut response = client
        .send(request)
        .await
        .with_context(|| format!("could not reach {url}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("download failed with {status} for {url}");
    }

    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    progress.total.store(total, Ordering::Relaxed);

    let mut file = std::fs::File::create(destination)
        .with_context(|| format!("could not create {}", destination.display()))?;
    let mut buffer = vec![0u8; 256 * 1024];
    let body = response.body_mut();

    loop {
        let read = body
            .read(&mut buffer)
            .await
            .context("could not read the download")?;
        if read == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buffer[..read])
            .with_context(|| format!("could not write {}", destination.display()))?;
        progress
            .downloaded
            .fetch_add(read as u64, Ordering::Relaxed);
    }

    Ok(())
}

pub(crate) async fn extract_archive(archive: &Path, destination: &Path) -> Result<()> {
    let status = hidden_command(Path::new("tar"))
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .status()
        .await
        .context("could not run `tar` to unpack the archive")?;
    if !status.success() {
        anyhow::bail!("`tar` failed to unpack {}", archive.display());
    }
    Ok(())
}

/// Builds a command that does not flash a console window on Windows.
fn hidden_command(binary: &Path) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(binary);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Downloads and unpacks the prebuilt whisper.cpp binary into the directory the
/// app owns, so the local engine works without a manual install.
pub async fn install_whisper_binary(
    client: &dyn HttpClient,
    progress: &DownloadProgress,
) -> Result<PathBuf> {
    let archive = catalog::whisper_binary_archive().ok_or_else(|| {
        anyhow!(
            "there is no prebuilt whisper.cpp binary for this platform; install it yourself \
             and set `voice_input.local.binary_path`"
        )
    })?;
    let directory = catalog::binaries_dir();
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("could not create {}", directory.display()))?;

    let archive_path = directory.join(archive.file_name);
    let partial_path = directory.join(format!("{}.part", archive.file_name));
    download_to_file(client, &archive.url, &partial_path, progress).await?;
    std::fs::rename(&partial_path, &archive_path)
        .with_context(|| format!("could not rename {}", partial_path.display()))?;

    extract_archive(&archive_path, &directory).await?;
    std::fs::remove_file(&archive_path).log_err();

    let binary = catalog::find_executable(&directory, catalog::WHISPER_BINARY_NAMES)
        .ok_or_else(|| anyhow!("the downloaded archive did not contain whisper.cpp"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&binary)
            .map(|metadata| metadata.permissions().mode())
            .unwrap_or(0o755);
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(mode | 0o755))
            .with_context(|| format!("could not make {} executable", binary.display()))?;
    }
    log::info!("voice input: installed whisper.cpp at {}", binary.display());
    Ok(binary)
}

/// Transcribes 16 kHz mono WAV audio with a local model.
pub async fn transcribe_local(
    settings: &VoiceInputSettings,
    wav: &[u8],
    prompt_context: Option<&str>,
) -> Result<String> {
    let content = settings.content();
    let local = content.local();

    let language = content.language().unwrap_or("auto").to_string();
    let binary_path = local.binary_path().map(Path::new);
    let extra_args = local.extra_args();

    let (engine, model_path) = if let Some(path) = local.model_path() {
        let engine = catalog_engine(local.engine())?;
        (engine, PathBuf::from(path))
    } else {
        let model_id = local.model();
        let model = catalog::local_model(model_id)
            .ok_or_else(|| anyhow!("unknown `voice_input.local.model` {model_id:?}"))?;
        let dir = catalog::model_dir_in(&models_dir(settings), model);
        if !catalog::is_downloaded_in(&dir, model) {
            anyhow::bail!(
                "the local model {model_id:?} is not downloaded yet; open the microphone menu to fetch it or set `voice_input.local.model_path`"
            );
        }
        (model.engine, primary_path(model, &dir)?)
    };

    let binary = catalog::resolve_binary(engine, binary_path)?;
    let wav_path = write_temp_wav(wav)?;
    let result = match engine {
        LocalEngine::WhisperCpp => {
            run_whisper_cpp(
                &binary,
                &model_path,
                &wav_path,
                &language,
                local.threads(),
                prompt_context,
                extra_args,
            )
            .await
        }
        LocalEngine::SherpaOnnx => {
            run_sherpa_onnx(&binary, &model_path, &wav_path, extra_args).await
        }
    };
    std::fs::remove_file(&wav_path).log_err();
    result
}

fn catalog_engine(engine: VoiceInputLocalEngine) -> Result<LocalEngine> {
    match engine {
        VoiceInputLocalEngine::WhisperCpp => Ok(LocalEngine::WhisperCpp),
        VoiceInputLocalEngine::SherpaOnnx => Ok(LocalEngine::SherpaOnnx),
        VoiceInputLocalEngine::Server => anyhow::bail!(
            "the `server` local engine transcribes over HTTP; use the cloud request path instead of `transcribe_local`"
        ),
    }
}

async fn run_whisper_cpp(
    binary: &Path,
    model_path: &Path,
    wav_path: &Path,
    language: &str,
    threads: Option<usize>,
    prompt_context: Option<&str>,
    extra_args: &[String],
) -> Result<String> {
    let mut command = hidden_command(binary);
    command
        .arg("-m")
        .arg(model_path)
        .arg("-f")
        .arg(wav_path)
        .arg("-l")
        .arg(language)
        .arg("-np")
        .arg("-nt");
    if let Some(threads) = threads {
        command.arg("-t").arg(threads.to_string());
    }
    if let Some(context) = prompt_context.filter(|context| !context.trim().is_empty()) {
        command.arg("--prompt").arg(context);
    }
    command.args(extra_args);

    let output = command
        .output()
        .await
        .with_context(|| format!("could not run {}", binary.display()))?;

    if !output.status.success() {
        anyhow::bail!(
            "whisper.cpp failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn run_sherpa_onnx(
    binary: &Path,
    model_dir: &Path,
    wav_path: &Path,
    extra_args: &[String],
) -> Result<String> {
    let tokens = model_dir.join("tokens.txt");
    let encoder = find_file(model_dir, "encoder")
        .ok_or_else(|| anyhow!("no encoder model found in {}", model_dir.display()))?;
    let decoder = find_file(model_dir, "decoder")
        .ok_or_else(|| anyhow!("no decoder model found in {}", model_dir.display()))?;
    let joiner = find_file(model_dir, "joiner")
        .ok_or_else(|| anyhow!("no joiner model found in {}", model_dir.display()))?;

    let output = hidden_command(binary)
        .arg(format!("--tokens={}", tokens.display()))
        .arg(format!("--encoder={}", encoder.display()))
        .arg(format!("--decoder={}", decoder.display()))
        .arg(format!("--joiner={}", joiner.display()))
        .args(extra_args)
        .arg(wav_path)
        .output()
        .await
        .with_context(|| format!("could not run {}", binary.display()))?;

    if !output.status.success() {
        anyhow::bail!(
            "sherpa-onnx failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(parse_sherpa_output(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn find_file(dir: &Path, stem: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(stem))
        })
}

/// Sherpa prints a per-segment listing followed by a `{ "text": ... }` JSON
/// block; the JSON is the only part we want.
fn parse_sherpa_output(stdout: &str) -> String {
    let trimmed = stdout.trim();
    if let Some(start) = trimmed.rfind('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed[start..]) {
            if let Some(text) = value.get("text").and_then(|text| text.as_str()) {
                return text.trim().to_string();
            }
        }
    }
    trimmed.to_string()
}

fn write_temp_wav(wav: &[u8]) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "myzed-voice-{}.wav",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::write(&path, wav).with_context(|| format!("could not write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_from_sherpa_output() {
        let stdout = "0.0 -- hello\n1.0 -- world\n\n{\n  \"text\": \" hello world \"\n}\n";
        assert_eq!(parse_sherpa_output(stdout), "hello world");
    }

    #[test]
    fn falls_back_to_plain_sherpa_output() {
        assert_eq!(parse_sherpa_output("just text"), "just text");
    }
}
