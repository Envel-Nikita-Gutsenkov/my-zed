use std::collections::VecDeque;
use std::time::Duration;

use gpui::{App, Context, IntoElement, ParentElement, Render, Subscription, Task, Window, div, px};
use settings::{RegisterSetting, Settings, SettingsStore};
use sysinfo::{
    CpuRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind,
};
use ui::prelude::*;
use workspace::{HideStatusItem, ItemHandle, StatusItemView, Workspace};

/// Runtime limits and monitor configuration, read from the `resources` settings
/// block.
#[derive(Clone, Debug, RegisterSetting)]
pub struct ResourcesSettings {
    pub monitor_enabled: bool,
    pub show_in_status_bar: bool,
    pub sample_interval_secs: u64,
    pub history_samples: usize,
    pub log_file_size_limit_mb: u64,
    pub trim_interval_secs: u64,
}

impl Settings for ResourcesSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let resources = content.resources.clone().unwrap_or_default();
        Self {
            monitor_enabled: resources.monitor_enabled(),
            show_in_status_bar: resources.show_in_status_bar(),
            sample_interval_secs: resources.sample_interval_secs(),
            history_samples: resources.history_samples(),
            log_file_size_limit_mb: resources.log_file_size_limit_mb(),
            trim_interval_secs: resources.trim_interval_secs(),
        }
    }
}

impl ResourcesSettings {
    fn apply_log_size_limit(&self) {
        if self.log_file_size_limit_mb == 0 {
            return;
        }
        let max_bytes = self.log_file_size_limit_mb.saturating_mul(1024 * 1024);
        zlog::set_max_file_size_bytes(max_bytes);
        log::info!(
            "resource monitor: log file size limit set to {} MB",
            self.log_file_size_limit_mb
        );
    }
}

/// Spawns a background task that periodically asks the allocator to return free
/// pages to the operating system when `resources.trim_interval_secs` is
/// non-zero. The interval is re-read every cycle, so toggling the setting takes
/// effect without a restart.
///
/// This is a no-op when the interval is `0` (the default), so it does not change
/// default behavior. See [`release_free_memory_to_os`] for the allocator hook.
pub fn init_memory_trimmer(cx: &mut App) {
    if cfg!(test) {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            // Poll while disabled so enabling the setting starts trimming
            // without a restart, and re-read after each sleep so disabling it
            // stops future trims.
            let interval_secs =
                cx.update(|cx| ResourcesSettings::get_global(cx).trim_interval_secs);
            if interval_secs == 0 {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                continue;
            }
            cx.background_executor()
                .timer(Duration::from_secs(interval_secs))
                .await;
            if cx.update(|cx| ResourcesSettings::get_global(cx).trim_interval_secs) == 0 {
                continue;
            }
            cx.background_executor()
                .spawn(async move { release_free_memory_to_os() })
                .await;
        }
    })
    .detach();
}

/// Releases unused heap pages back to the operating system.
///
/// With the default `mimalloc` allocator this calls `mi_collect(true)`, which
/// returns retained pages; otherwise on glibc it falls back to `malloc_trim`.
fn release_free_memory_to_os() {
    #[cfg(feature = "mimalloc")]
    {
        // SAFETY: `mi_collect` is exported by the mimalloc static library that
        // is linked through the `mimalloc` crate. It only touches mimalloc's own
        // runtime state and is safe to call from any thread.
        unsafe extern "C" {
            fn mi_collect(force: bool);
        }
        unsafe { mi_collect(true) };
    }
    #[cfg(all(not(feature = "mimalloc"), target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: `malloc_trim` is a glibc extension that only releases unused
        // heap pages and is safe to call from any thread.
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        unsafe { malloc_trim(0) };
    }
    #[cfg(all(
        not(feature = "mimalloc"),
        not(all(target_os = "linux", target_env = "gnu"))
    ))]
    {
        // No allocator-specific trim hook is available on this platform.
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ResourceSample {
    cpu_percent: f32,
    memory_bytes: u64,
}

/// Samples this process's CPU and memory usage on a timer and renders a compact
/// indicator with a mini graph in the status bar.
pub struct ResourceMonitor {
    sample: ResourceSample,
    cpu_history: VecDeque<f32>,
    memory_history: VecDeque<u64>,
    history_samples: usize,
    settings: ResourcesSettings,
    _task: Task<()>,
    _settings_subscription: Subscription,
}

impl ResourceMonitor {
    pub fn new(_workspace: &Workspace, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = ResourcesSettings::get_global(cx).clone();
        settings.apply_log_size_limit();

        let task = Self::spawn_sampler(cx, &settings);
        let history_samples = settings.history_samples;
        let settings_subscription = cx.observe_global::<SettingsStore>(|this, cx| {
            let settings = ResourcesSettings::get_global(cx).clone();
            settings.apply_log_size_limit();
            if this.settings.history_samples != settings.history_samples {
                this.cpu_history.clear();
                this.memory_history.clear();
            }
            this.history_samples = settings.history_samples;
            this.settings = settings;
            cx.notify();
        });

        Self {
            sample: ResourceSample::default(),
            cpu_history: VecDeque::with_capacity(history_samples),
            memory_history: VecDeque::with_capacity(history_samples),
            history_samples,
            settings,
            _task: task,
            _settings_subscription: settings_subscription,
        }
    }

    fn spawn_sampler(cx: &mut Context<Self>, settings: &ResourcesSettings) -> Task<()> {
        if cfg!(test) || !settings.monitor_enabled {
            return Task::ready(());
        }
        let interval = Duration::from_secs(settings.sample_interval_secs);
        let mut system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_processes(
                    ProcessRefreshKind::nothing()
                        .with_cpu()
                        .with_memory()
                        .without_tasks()
                        .with_exe(UpdateKind::Never),
                ),
        );
        // Discard the first sample: CPU usage needs two refreshes separated by
        // one interval to become meaningful.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                let (next_system, sample) = cx
                    .background_spawn(async move {
                        let sample = collect_sample(&mut system);
                        (system, sample)
                    })
                    .await;
                system = next_system;
                let result = this.update(cx, |this, cx| {
                    this.push_sample(sample);
                    cx.notify();
                });
                if result.is_err() {
                    break;
                }
            }
        })
    }

    fn push_sample(&mut self, sample: ResourceSample) {
        self.sample = sample;
        if self.cpu_history.len() >= self.history_samples {
            self.cpu_history.pop_front();
        }
        self.cpu_history.push_back(sample.cpu_percent);
        if self.memory_history.len() >= self.history_samples {
            self.memory_history.pop_front();
        }
        self.memory_history.push_back(sample.memory_bytes);
    }
}

fn collect_sample(system: &mut System) -> ResourceSample {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .without_tasks()
            .with_exe(UpdateKind::Never),
    );

    let Some(current_pid) = sysinfo::get_current_pid().ok() else {
        return ResourceSample::default();
    };
    let mut pids = vec![current_pid];
    let mut index = 0;
    // Collect the process tree (children spawned by the app, e.g. language
    // servers and terminals) so the numbers reflect the whole application.
    while index < pids.len() {
        let parent = pids[index];
        for (pid, process) in system.processes() {
            if process.parent() == Some(parent) && !pids.contains(pid) {
                pids.push(*pid);
            }
        }
        index += 1;
    }

    let mut total_cpu: f32 = 0.0;
    let mut total_memory: u64 = 0;
    for pid in &pids {
        if let Some(process) = system.process(*pid) {
            total_cpu += process.cpu_usage();
            total_memory = total_memory.saturating_add(process.memory());
        }
    }

    let cpu_count = system.cpus().len().max(1) as f32;
    ResourceSample {
        cpu_percent: (total_cpu / cpu_count).clamp(0.0, 100.0),
        memory_bytes: total_memory,
    }
}

fn format_memory(bytes: u64) -> String {
    let megabytes = bytes / (1024 * 1024);
    if megabytes >= 1024 {
        format!("{:.1} GB", megabytes as f64 / 1024.0)
    } else {
        format!("{megabytes} MB")
    }
}

impl Render for ResourceMonitor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.settings.monitor_enabled || !self.settings.show_in_status_bar {
            return div().hidden().into_any_element();
        }

        let accent = cx.theme().colors().text_accent;
        let muted = cx.theme().colors().text_muted;
        let cpu_values: Vec<f32> = self.cpu_history.iter().copied().collect();

        h_flex()
            .gap_2()
            .child(
                Label::new(format!("{:.0}%", self.sample.cpu_percent))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                h_flex()
                    .h(px(14.))
                    .w(px(64.))
                    .items_end()
                    .gap(px(1.))
                    .children(cpu_values.iter().map(|value| {
                        let height = (value.clamp(0.0, 100.0) / 100.0 * 14.0).max(1.0);
                        div()
                            .flex_1()
                            .h(px(height))
                            .rounded_sm()
                            .bg(if *value > 80.0 { muted } else { accent })
                    })),
            )
            .child(
                Label::new(format_memory(self.sample.memory_bytes))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .into_any_element()
    }
}

impl StatusItemView for ResourceMonitor {
    fn set_active_pane_item(
        &mut self,
        _active_pane_item: Option<&dyn ItemHandle>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        None
    }
}
