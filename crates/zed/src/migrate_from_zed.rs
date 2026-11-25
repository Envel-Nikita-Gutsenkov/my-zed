//! Imports chats, workspace state and downloaded assets from a regular Zed
//! install into this build's data directory.
//!
//! The heavy lifting happens on the next launch: writing the marker file here
//! and restarting lets us copy the source database before this build's own
//! `AppDatabase` opens (and possibly migrates) its files.

use anyhow::{Context as _, Result, anyhow};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const PENDING_MIGRATION_FILE: &str = ".migrate-from-stable";

/// Directory name this fork uses for its data on Windows (see
/// `paths::APP_DIR_NAME_WINDOWS`).
const FORK_DIR_NAME: &str = "my-zed";

/// Asset directories that can be copied verbatim when the destination does not
/// already exist. Most are already symlinked to the stock install, so they are
/// typically skipped.
const ASSET_DIRS: &[&str] = &[
    "extensions",
    "languages",
    "prompts",
    "debug_adapters",
    "external_agents",
    "node",
    "prettier",
    "remote_servers",
    "prompt_overrides",
    "copilot",
];

fn pending_marker_path() -> PathBuf {
    paths::data_dir().join(PENDING_MIGRATION_FILE)
}

/// Resolves the data directory of a regular (non-fork) Zed install.
///
/// On Linux the fork runs with `XDG_DATA_HOME=$HOME/.local/share/my-zed`, so its
/// own data directory is `$XDG_DATA_HOME/zed` and the stock install lives one
/// level up at `$XDG_DATA_HOME/../zed`. On Windows the fork's data directory is
/// `%LOCALAPPDATA%\my-zed`, so the stock install is the sibling `%LOCALAPPDATA%\Zed`.
pub fn stock_data_dir() -> Option<PathBuf> {
    let own = paths::data_dir();
    let own_name = own.file_name().and_then(|name| name.to_str());

    if own_name.is_some_and(|name| name.eq_ignore_ascii_case(FORK_DIR_NAME)) {
        // Windows: %LOCALAPPDATA%\my-zed -> %LOCALAPPDATA%\Zed
        Some(own.parent()?.join(paths::APP_NAME))
    } else if own
        .parent()
        .and_then(|base| base.file_name())
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == FORK_DIR_NAME)
    {
        // Linux: .../my-zed/zed -> .../zed
        Some(own.parent()?.parent()?.join("zed"))
    } else {
        Some(own.clone())
    }
}

/// Writes the marker file so the import runs on the next launch.
pub fn queue_import() -> Result<PathBuf> {
    let source = stock_data_dir()
        .ok_or_else(|| anyhow!("could not locate the regular Zed data directory"))?;
    if !source.exists() {
        return Err(anyhow!("no regular Zed data found at {}", source.display()));
    }
    if source == *paths::data_dir() {
        return Err(anyhow!(
            "this build already uses {}; nothing to import",
            source.display()
        ));
    }

    let marker = pending_marker_path();
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&marker, source.to_string_lossy().as_bytes())
        .with_context(|| format!("writing {}", marker.display()))?;
    log::info!(
        "queued import from {} into {}",
        source.display(),
        paths::data_dir().display()
    );
    Ok(source)
}

/// Runs a queued import, if any. Must be called before `db::AppDatabase::new()`
/// so the copied database is opened rather than overwritten.
pub fn run_pending_import() {
    let marker = pending_marker_path();
    let source = match std::fs::read(&marker) {
        Ok(bytes) => PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            log::error!("could not read {}: {err}", marker.display());
            return;
        }
    };

    match import_from(&source) {
        Ok(summary) => log::info!("imported data from {}: {summary}", source.display()),
        Err(err) => log::error!("failed to import data from {}: {err:#}", source.display()),
    }

    // Always clear the marker so a broken import does not retry forever.
    if let Err(err) = std::fs::remove_file(&marker) {
        log::error!("could not remove {}: {err}", marker.display());
    }
}

fn import_from(source: &Path) -> Result<String> {
    if !source.is_dir() {
        return Err(anyhow!("{} is not a directory", source.display()));
    }

    let mut parts = Vec::new();

    let source_threads = source.join("threads");
    if source_threads.is_dir() {
        let destination = paths::data_dir().join("threads");
        backup_and_replace(&source_threads, &destination)?;
        parts.push("chats".to_string());
    }

    let source_db = source.join("db");
    if source_db.is_dir() {
        let destination_db = paths::database_dir();

        if let Some(channel_dir) = newest_channel_dir(&source_db)? {
            let destination = db::db_path(&destination_db, *release_channel::RELEASE_CHANNEL)
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| anyhow!("database path has no parent"))?;
            backup_and_replace(&channel_dir, &destination)?;
            parts.push("workspace state".to_string());
        }

        let source_global = source_db.join("0-global");
        if source_global.is_dir() {
            let destination = db::db_path(&destination_db, db::GlobalDbScope)
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| anyhow!("global database path has no parent"))?;
            backup_and_replace(&source_global, &destination)?;
            parts.push("global database".to_string());
        }
    }

    for name in ASSET_DIRS {
        let source_dir = source.join(name);
        let destination = paths::data_dir().join(name);
        if source_dir.is_dir() && !destination.exists() {
            copy_dir(&source_dir, &destination)?;
            parts.push((*name).to_string());
        }
    }

    if parts.is_empty() {
        return Err(anyhow!("nothing to import from {}", source.display()));
    }
    Ok(parts.join(", "))
}

/// Picks the release-channel database directory (skipping `0-global`) with the
/// most recently modified `db.sqlite`, so a stable install migrates cleanly
/// into this build's channel.
fn newest_channel_dir(db_dir: &Path) -> Result<Option<PathBuf>> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in
        std::fs::read_dir(db_dir).with_context(|| format!("reading {}", db_dir.display()))?
    {
        let entry = entry?;
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };
        if !name.starts_with("0-") || name == "0-global" {
            continue;
        }
        let path = entry.path();
        let db_file = path.join("db.sqlite");
        let modified = match std::fs::metadata(&db_file).and_then(|meta| meta.modified()) {
            Ok(modified) => modified,
            Err(_) => continue,
        };
        if best
            .as_ref()
            .is_none_or(|(best_time, _)| modified > *best_time)
        {
            best = Some((modified, path));
        }
    }
    Ok(best.map(|(_, path)| path))
}

/// Moves an existing destination aside as `<name>.backup-<unix_secs>`, then
/// copies `source` into its place.
fn backup_and_replace(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        let backup = destination.with_file_name(format!(
            "{}.backup-{}",
            destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("data"),
            unix_seconds()
        ));
        std::fs::rename(destination, &backup)
            .with_context(|| format!("moving {} to {}", destination.display(), backup.display()))?;
        log::info!(
            "backed up {} to {}",
            destination.display(),
            backup.display()
        );
    }
    copy_dir(source, destination)
}

fn copy_dir(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("creating {}", destination.display()))?;
    for entry in
        std::fs::read_dir(source).with_context(|| format!("reading {}", source.display()))?
    {
        let entry = entry?;
        let entry_path = entry.path();
        let target = destination.join(entry.file_name());
        // Follow symlinks so we copy real content rather than dangling links.
        let metadata = std::fs::metadata(&entry_path)
            .with_context(|| format!("reading metadata for {}", entry_path.display()))?;
        if metadata.is_dir() {
            copy_dir(&entry_path, &target)?;
        } else if metadata.is_file() {
            std::fs::copy(&entry_path, &target).with_context(|| {
                format!("copying {} to {}", entry_path.display(), target.display())
            })?;
        }
    }
    Ok(())
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
