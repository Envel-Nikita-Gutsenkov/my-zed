use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::MergeFrom;

/// Settings for the built-in process resource monitor and for the lightweight
/// runtime limits that reduce CPU and disk usage.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema, MergeFrom)]
#[serde(rename_all = "snake_case")]
pub struct ResourcesSettingsContent {
    /// Whether to sample this process's CPU and memory usage at all. When
    /// disabled the monitor does no work and the status bar item is hidden.
    ///
    /// Default: true
    pub monitor_enabled: Option<bool>,
    /// Whether to show a compact CPU/memory indicator with a mini graph in the
    /// status bar.
    ///
    /// Default: true
    pub show_in_status_bar: Option<bool>,
    /// Seconds between resource samples. Larger values use less CPU.
    ///
    /// Default: 2
    pub sample_interval_secs: Option<u64>,
    /// Number of samples kept for the mini graph and the tooltip graph.
    ///
    /// Default: 60
    pub history_samples: Option<usize>,
    /// Cap on the size of the rotating log file, in megabytes. The value is
    /// applied when settings load and on change. `0` keeps the built-in
    /// default (1 MB).
    ///
    /// Default: 0
    pub log_file_size_limit_mb: Option<u64>,
    /// Seconds between background attempts to return unused heap pages to the
    /// operating system. `0` disables the periodic trim.
    ///
    /// On the default glibc allocator this calls `malloc_trim`; with the opt-in
    /// `mimalloc` feature it calls `mi_collect` instead. Reduces resident memory
    /// after the editor has handled bursts of large allocations, at the cost of
    /// a small amount of background work.
    ///
    /// Default: 60
    pub trim_interval_secs: Option<u64>,
}

impl ResourcesSettingsContent {
    pub fn monitor_enabled(&self) -> bool {
        self.monitor_enabled.unwrap_or(true)
    }

    pub fn show_in_status_bar(&self) -> bool {
        self.show_in_status_bar.unwrap_or(true)
    }

    pub fn sample_interval_secs(&self) -> u64 {
        self.sample_interval_secs.unwrap_or(2).max(1)
    }

    pub fn history_samples(&self) -> usize {
        self.history_samples.unwrap_or(60).clamp(8, 600)
    }

    pub fn log_file_size_limit_mb(&self) -> u64 {
        self.log_file_size_limit_mb.unwrap_or(0)
    }

    pub fn trim_interval_secs(&self) -> u64 {
        self.trim_interval_secs.unwrap_or(60)
    }
}
