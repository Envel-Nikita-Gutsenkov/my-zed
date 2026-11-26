//! Shared state for local model downloads.
//!
//! Downloads outlive the menu or settings page that started them, so their
//! status lives in a global instead of a view. Every entry is backed by cheap
//! atomics because the download itself runs on a background executor and cannot
//! touch the UI; views poll [`status`] and re-render while something is active.
//! The task is owned by the entry, so cancelling means dropping the entry.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use ::settings::Settings as _;
use gpui::{App, AppContext as _, Global, Task};

use crate::{
    catalog::{self, LOCAL_MODELS},
    local::{self, DownloadProgress},
    settings::VoiceInputSettings,
};

const STATUS_DOWNLOADING: u8 = 1;
const STATUS_READY: u8 = 2;
const STATUS_FAILED: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelDownloadStatus {
    Downloading,
    Ready,
    Failed,
}

/// Snapshot of a model's download state, taken once per render.
#[derive(Clone, Debug)]
pub struct ModelDownload {
    pub status: ModelDownloadStatus,
    pub downloaded: u64,
    pub total: u64,
    pub error: Option<String>,
}

impl ModelDownload {
    /// Fraction of the download that has completed, in `0.0..=1.0`.
    ///
    /// Servers that do not report a content length leave this at zero so the
    /// UI can render an indeterminate state instead.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.downloaded as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }

    pub fn is_downloading(&self) -> bool {
        self.status == ModelDownloadStatus::Downloading
    }
}

struct Entry {
    status: Arc<AtomicU8>,
    progress: DownloadProgress,
    error: Arc<Mutex<Option<String>>>,
    // Dropping the task cancels the download.
    _task: Task<()>,
}

impl Entry {
    fn snapshot(&self) -> ModelDownload {
        let status = match self.status.load(Ordering::Relaxed) {
            STATUS_READY => ModelDownloadStatus::Ready,
            STATUS_FAILED => ModelDownloadStatus::Failed,
            _ => ModelDownloadStatus::Downloading,
        };
        let error = self.error.lock().ok().and_then(|error| error.clone());
        ModelDownload {
            status,
            downloaded: self.progress.downloaded(),
            total: self.progress.total(),
            error,
        }
    }
}

#[derive(Default)]
pub struct ModelDownloads {
    entries: HashMap<&'static str, Entry>,
}

impl Global for ModelDownloads {}

pub fn init(cx: &mut App) {
    cx.set_global(ModelDownloads::default());
}

/// Current state of `model_id`, including models that are already on disk
/// without having been downloaded in this session.
pub fn status(cx: &App, model_id: &str) -> Option<ModelDownload> {
    if let Some(entry) = cx
        .try_global::<ModelDownloads>()
        .and_then(|downloads| downloads.entries.get(model_id))
    {
        return Some(entry.snapshot());
    }

    let model = catalog::local_model(model_id)?;
    let settings = VoiceInputSettings::get_global(cx);
    if crate::model_is_downloaded(settings, model) {
        return Some(ModelDownload {
            status: ModelDownloadStatus::Ready,
            downloaded: 0,
            total: 0,
            error: None,
        });
    }
    None
}

/// Whether any model is currently downloading. Views use this to decide
/// whether they need to keep polling for progress.
pub fn is_any_downloading(cx: &App) -> bool {
    cx.try_global::<ModelDownloads>().is_some_and(|downloads| {
        downloads
            .entries
            .values()
            .any(|entry| entry.snapshot().is_downloading())
    })
}

/// Starts downloading `model_id` unless it is already running.
pub fn start(cx: &mut App, model_id: &'static str) -> bool {
    let Some(model) = LOCAL_MODELS.iter().find(|model| model.id == model_id) else {
        log::error!("unknown local model {model_id}");
        return false;
    };
    if cx
        .try_global::<ModelDownloads>()
        .and_then(|downloads| downloads.entries.get(model_id))
        .is_some_and(|entry| entry.snapshot().is_downloading())
    {
        return false;
    }

    let base_dir = local::models_dir(VoiceInputSettings::get_global(cx));
    let client = cx.http_client();

    let status = Arc::new(AtomicU8::new(STATUS_DOWNLOADING));
    let progress = DownloadProgress::default();
    let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let task = cx.background_spawn({
        let status = status.clone();
        let progress = progress.clone();
        let error = error.clone();
        async move {
            match local::ensure_model(model, &base_dir, client.as_ref(), &progress).await {
                Ok(_) => status.store(STATUS_READY, Ordering::Relaxed),
                Err(download_error) => {
                    log::error!("could not download {model_id}: {download_error:#}");
                    if let Ok(mut slot) = error.lock() {
                        *slot = Some(format!("{download_error:#}"));
                    }
                    status.store(STATUS_FAILED, Ordering::Relaxed);
                }
            }
        }
    });

    if cx.try_global::<ModelDownloads>().is_none() {
        return false;
    }
    cx.global_mut::<ModelDownloads>().entries.insert(
        model_id,
        Entry {
            status,
            progress,
            error,
            _task: task,
        },
    );
    true
}

/// Stops an in-progress download and removes whatever was already fetched.
pub fn cancel(cx: &mut App, model_id: &'static str) {
    if cx.try_global::<ModelDownloads>().is_none() {
        return;
    }
    cx.global_mut::<ModelDownloads>().entries.remove(model_id);
    remove_files_later(cx, model_id, false);
}

/// Deletes a downloaded model from disk.
pub fn delete(cx: &mut App, model_id: &'static str) {
    if cx.try_global::<ModelDownloads>().is_some() {
        cx.global_mut::<ModelDownloads>().entries.remove(model_id);
    }
    remove_files_later(cx, model_id, true);
}

fn remove_files_later(cx: &mut App, model_id: &'static str, remove_whole_dir: bool) {
    let Some(model) = catalog::local_model(model_id) else {
        return;
    };
    let dir = local::models_dir(VoiceInputSettings::get_global(cx)).join(model.id);
    cx.background_spawn(async move {
        if remove_whole_dir {
            if let Err(error) = std::fs::remove_dir_all(&dir) {
                log::error!("could not delete {}: {error}", dir.display());
            }
            return;
        }

        // Dropping the task might have left a partially written archive behind.
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                log::error!("could not inspect {}: {error}", dir.display());
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_partial = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".part"));
            if is_partial && let Err(error) = std::fs::remove_file(&path) {
                log::error!("could not remove {}: {error}", path.display());
            }
        }
    })
    .detach();
}
