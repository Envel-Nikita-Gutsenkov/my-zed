//! Dedicated settings page for voice input (dictation).
//!
//! Most of the page is a model manager: the built-in local models, their
//! download status and the buttons that download, cancel or delete them. The
//! download state itself lives in `voice_input::downloads`, so downloads keep
//! running while this window is closed and the page only polls for progress.

use std::path::{Path, PathBuf};

use gpui::{App, ScrollHandle, SharedString, Window, prelude::*};
use settings::Settings as _;
use ui::{
    Button, ButtonStyle, Divider, Icon, IconName, IconSize, Label, LabelSize, ProgressBar,
    SwitchField, TintColor, ToggleState, prelude::*,
};
use util::ResultExt as _;
use voice_input::{
    CLOUD_PRESETS, LOCAL_MODELS, LocalModelDef, ModelDownload, ModelDownloadStatus,
    VoiceInputEngine, VoiceInputSettings, model_download_status, whisper_binary_ready,
    whisper_install_status,
};

use crate::components::SettingsSectionHeader;
use crate::{SettingsUiFile, SettingsWindow, update_settings_file};

const ENABLE_DESCRIPTION: &str =
    "Show the microphone button in the agent panel and allow dictation with Ctrl-Alt-M.";

pub(crate) fn render_voice_input_page(
    _settings_window: &SettingsWindow,
    scroll_handle: &ScrollHandle,
    window: &mut Window,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let settings = VoiceInputSettings::get_global(cx).content().clone();
    let engine = settings.engine();
    let models_dir = models_dir(&settings);
    poll_downloads(window, cx);

    v_flex()
        .id("voice-input-page")
        .min_w_0()
        .size_full()
        .pt_2p5()
        .px_8()
        .pb_16()
        .gap_4()
        .overflow_y_scroll()
        .track_scroll(scroll_handle)
        .child(
            Label::new("Voice Input")
                .size(LabelSize::Large)
                .color(Color::Default),
        )
        .child(
            Label::new(
                "Record speech from the microphone button in the agent panel and insert the \
                 transcription into the message you are writing.",
            )
            .size(LabelSize::Small)
            .color(Color::Muted),
        )
        .child(
            SwitchField::new(
                "voice-input-enabled",
                Some("Enable Voice Input"),
                Some(ENABLE_DESCRIPTION.into()),
                toggle_state(settings.enabled()),
                move |state, window, cx| {
                    let enabled = *state == ToggleState::Selected;
                    set_enabled(enabled, window, cx);
                },
            )
            .tab_index(0),
        )
        .child(Divider::horizontal())
        .child(
            v_flex()
                .gap_2()
                .child(SettingsSectionHeader::new("Engine"))
                .child(
                    Label::new(
                        "Cloud sends audio to an OpenAI-compatible transcription API and needs no \
                         local download. Local runs entirely on this machine with a model below.",
                    )
                    .size(LabelSize::Small)
                    .color(Color::Muted),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(engine_button(
                            "voice-input-engine-cloud",
                            "Cloud API",
                            engine == VoiceInputEngine::Cloud,
                            VoiceInputEngine::Cloud,
                        ))
                        .child(engine_button(
                            "voice-input-engine-local",
                            "Local model",
                            engine == VoiceInputEngine::Local,
                            VoiceInputEngine::Local,
                        )),
                ),
        )
        .child(Divider::horizontal())
        .child(microphone_section(&settings, cx))
        .child(Divider::horizontal())
        .child(cloud_section(&settings, engine, cx))
        .child(Divider::horizontal())
        .child(local_section(&settings, &models_dir, engine, cx))
        .into_any_element()
}

/// While a download is running the page re-renders so the progress bar moves.
/// The callback is only registered when something is actually downloading, so
/// an idle settings window does no work at all.
fn poll_downloads(window: &mut Window, cx: &mut Context<SettingsWindow>) {
    if !voice_input::is_any_downloading(cx) {
        return;
    }
    let this = cx.weak_entity();
    window.on_next_frame(move |_, cx| {
        this.update(cx, |_, cx| cx.notify()).log_err();
    });
}

fn models_dir(settings: &voice_input::VoiceInputSettingsContent) -> PathBuf {
    settings
        .local()
        .models_dir()
        .map(PathBuf::from)
        .unwrap_or_else(voice_input::models_dir)
}

fn toggle_state(enabled: bool) -> ToggleState {
    if enabled {
        ToggleState::Selected
    } else {
        ToggleState::Unselected
    }
}

fn engine_button(
    id: &'static str,
    label: &'static str,
    selected: bool,
    engine: VoiceInputEngine,
) -> AnyElement {
    Button::new(id, label)
        .style(if selected {
            ButtonStyle::Filled
        } else {
            ButtonStyle::Outlined
        })
        .label_size(LabelSize::Small)
        .on_click(move |_, window, cx| {
            update_settings_file(
                SettingsUiFile::User,
                Some("voice_input.engine"),
                window,
                cx,
                move |content, _| {
                    content.voice_input.get_or_insert_default().engine = Some(engine);
                },
            )
            .log_err();
            if let Some(window) = window.window_handle().downcast::<SettingsWindow>() {
                window.update(cx, |_, _, cx| cx.notify()).log_err();
            }
        })
        .into_any_element()
}

fn set_enabled(enabled: bool, window: &mut Window, cx: &mut App) {
    update_settings_file(
        SettingsUiFile::User,
        Some("voice_input.enabled"),
        window,
        cx,
        move |content, _| {
            content.voice_input.get_or_insert_default().enabled = Some(enabled);
        },
    )
    .log_err();
    notify_settings(window, cx);
}

/// Microphone picker.
///
/// Dictation falls back to other devices when the configured one cannot be
/// opened, but a user whose default is a dead PipeWire/ALSA node needs a way to
/// pin a working device without editing JSON.
fn microphone_section(
    settings: &voice_input::VoiceInputSettingsContent,
    cx: &mut App,
) -> AnyElement {
    let configured = settings.device().map(|device| device.to_string());
    let devices = voice_input::input_devices();
    let audio_device = audio::AudioSettings::get_global(cx)
        .input_audio_device
        .clone();

    let mut rows = v_flex().gap_1().child(device_row(
        "voice-input-device-default",
        "System default",
        configured.is_none(),
        None,
        cx,
    ));

    if devices.is_empty() {
        rows = rows.child(
            Label::new("No microphone was found. Connect one and reopen this page.")
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        );
    }

    for device in devices {
        let label = if device.is_default {
            format!("{} (default)", device.name)
        } else {
            device.name.clone()
        };
        let selected = match &configured {
            Some(configured) => configured == &device.id,
            None => false,
        };
        rows = rows.child(device_row(
            device.id.clone(),
            label,
            selected,
            Some(device.id.clone()),
            cx,
        ));
    }

    v_flex()
        .gap_2()
        .child(SettingsSectionHeader::new("Microphone"))
        .child(
            Label::new(match &audio_device {
                Some(_) => {
                    "The system default follows the input device chosen for audio calls when one \
                     is set, and otherwise lets the operating system pick."
                }
                None => "The system default lets the operating system pick the input device.",
            })
            .size(LabelSize::Small)
            .color(Color::Muted),
        )
        .child(rows)
        .into_any_element()
}

fn device_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    device: Option<String>,
    cx: &App,
) -> AnyElement {
    let id: SharedString = id.into();
    h_flex()
        .id(SharedString::from(format!("voice-input-device-{id}")))
        .w_full()
        .gap_2()
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().colors().border_variant)
        .justify_between()
        .items_center()
        .child(Label::new(label).size(LabelSize::Small))
        .child(if selected {
            Icon::new(IconName::Check)
                .size(IconSize::Small)
                .color(Color::Accent)
                .into_any_element()
        } else {
            Button::new(
                SharedString::from(format!("voice-input-device-use-{id}")),
                "Use",
            )
            .style(ButtonStyle::Outlined)
            .label_size(LabelSize::XSmall)
            .on_click(move |_, window, cx| {
                let device = device.clone();
                update_settings_file(
                    SettingsUiFile::User,
                    Some("voice_input.device"),
                    window,
                    cx,
                    move |content, _| {
                        content.voice_input.get_or_insert_default().device = device;
                    },
                )
                .log_err();
                notify_settings(window, cx);
            })
            .into_any_element()
        })
        .into_any_element()
}

fn cloud_section(
    settings: &voice_input::VoiceInputSettingsContent,
    engine: VoiceInputEngine,
    cx: &mut App,
) -> AnyElement {
    let selected_provider = settings.cloud().provider().to_string();
    let dimmed = engine != VoiceInputEngine::Cloud;

    let rows = CLOUD_PRESETS.iter().map(|preset| {
        let selected = preset.id == selected_provider;
        let provider_id = preset.id;
        let key_status = if settings
            .cloud()
            .api_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty())
        {
            "set in settings".to_string()
        } else if std::env::var(preset.api_key_env).is_ok_and(|value| !value.is_empty()) {
            format!("found in {}", preset.api_key_env)
        } else {
            format!(
                "missing — set {} or `voice_input.cloud.api_key`",
                preset.api_key_env
            )
        };
        let mut row = h_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_3()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .justify_between()
            .child(
                v_flex()
                    .gap_1()
                    .child(Label::new(preset.name).size(LabelSize::Default))
                    .child(
                        Label::new(format!("Model: {} · API key: {key_status}", preset.model))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            );

        if selected {
            row = row.child(
                h_flex()
                    .gap_1()
                    .child(Icon::new(IconName::Check).size(IconSize::Small))
                    .child(Label::new("Selected").size(LabelSize::Small)),
            );
        } else {
            row = row.child(
                Button::new(
                    SharedString::from(format!("voice-provider-{provider_id}")),
                    "Use",
                )
                .style(ButtonStyle::Outlined)
                .label_size(LabelSize::Small)
                .on_click({
                    let provider_id = provider_id.to_string();
                    move |_, window, cx| {
                        update_settings_file(
                            SettingsUiFile::User,
                            Some("voice_input.cloud.provider"),
                            window,
                            cx,
                            {
                                let provider_id = provider_id.clone();
                                move |content, _| {
                                    content
                                        .voice_input
                                        .get_or_insert_default()
                                        .cloud
                                        .get_or_insert_default()
                                        .provider = Some(provider_id);
                                }
                            },
                        )
                        .log_err();
                        notify_settings(window, cx);
                    }
                }),
            );
        }

        row.into_any_element()
    });

    v_flex()
        .gap_2()
        .when(dimmed, |this| this.opacity(0.6))
        .child(SettingsSectionHeader::new("Cloud Provider"))
        .child(
            Label::new(
                "Set the API key for the chosen provider in your environment (or as \
                 `voice_input.cloud.api_key` in settings.json).",
            )
            .size(LabelSize::Small)
            .color(Color::Muted),
        )
        .children(rows)
        .into_any_element()
}

fn local_section(
    settings: &voice_input::VoiceInputSettingsContent,
    models_dir: &Path,
    engine: VoiceInputEngine,
    cx: &mut App,
) -> AnyElement {
    let selected_model = settings.local().model().to_string();
    let dimmed = engine != VoiceInputEngine::Local;

    let rows = LOCAL_MODELS
        .iter()
        .map(|model| {
            render_model_row(
                model,
                model.id == selected_model,
                model_download_status(cx, model.id),
                cx,
            )
        })
        .collect::<Vec<_>>();

    v_flex()
        .gap_2()
        .when(dimmed, |this| this.opacity(0.6))
        .child(SettingsSectionHeader::new("Local Models"))
        .child(
            Label::new(format!(
                "Downloaded into {}. Nothing here is downloaded until you ask for it, and a model \
                 can be deleted at any time to free the disk space.",
                models_dir.display()
            ))
            .size(LabelSize::Small)
            .color(Color::Muted),
        )
        .child(render_engine_row(engine, cx))
        .children(rows)
        .into_any_element()
}

/// The whisper.cpp binary the local engine runs.
///
/// It is a separate download from the model, and without it a local dictation
/// cannot start at all, so its state is shown above the model list.
fn render_engine_row(engine: VoiceInputEngine, cx: &mut App) -> AnyElement {
    let status = whisper_install_status(cx);
    let downloading = status
        .as_ref()
        .is_some_and(|state| state.status == ModelDownloadStatus::Downloading);
    let failed = status
        .as_ref()
        .is_some_and(|state| state.status == ModelDownloadStatus::Failed);
    let ready = whisper_binary_ready();

    let status_label = if ready {
        Label::new("Ready")
            .size(LabelSize::XSmall)
            .color(Color::Success)
    } else if downloading {
        Label::new(format!("Downloading… {}", format_bytes(status.as_ref().map_or(0, |s| s.downloaded))))
            .size(LabelSize::XSmall)
            .color(Color::Muted)
    } else if failed {
        Label::new("Download failed")
            .size(LabelSize::XSmall)
            .color(Color::Error)
    } else {
        Label::new("Not installed")
            .size(LabelSize::XSmall)
            .color(Color::Muted)
    };

    let controls = if ready {
        h_flex()
    } else if downloading {
        h_flex().child(
            Button::new("voice-engine-cancel", "Cancel")
                .style(ButtonStyle::Outlined)
                .label_size(LabelSize::Small)
                .on_click(move |_, window, cx| {
                    voice_input::cancel_whisper_install(cx);
                    notify_settings(window, cx);
                }),
        )
    } else {
        h_flex().child(
            Button::new("voice-engine-install", "Install")
                .style(ButtonStyle::Tinted(TintColor::Accent))
                .label_size(LabelSize::Small)
                .on_click(move |_, window, cx| {
                    voice_input::start_whisper_install(cx);
                    notify_settings(window, cx);
                }),
        )
    };

    v_flex()
        .w_full()
        .px_3()
        .py_2()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().colors().border_variant)
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .justify_between()
                .items_center()
                .child(
                    v_flex()
                        .gap_1()
                        .child(Label::new("Speech engine (whisper.cpp)").size(LabelSize::Default))
                        .child(
                            Label::new(
                                "Transcribes audio on this machine. Downloaded once, about 12 MB.",
                            )
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        ),
                )
                .child(controls),
        )
        .child(status_label)
        .when(engine != VoiceInputEngine::Local, |this| {
            this.child(
                Label::new("Used when the engine above is set to Local model.")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
        })
        .into_any_element()
}

fn render_model_row(
    model: &'static LocalModelDef,
    selected: bool,
    download: Option<ModelDownload>,
    cx: &App,
) -> AnyElement {
    let on_disk = download
        .as_ref()
        .is_some_and(|state| state.status == ModelDownloadStatus::Ready);
    let downloading = download
        .as_ref()
        .is_some_and(|state| state.status == ModelDownloadStatus::Downloading);
    let failed = download
        .as_ref()
        .is_some_and(|state| state.status == ModelDownloadStatus::Failed);

    let mut controls = h_flex().gap_2().items_center();

    if downloading {
        controls = controls.child(
            Button::new(
                SharedString::from(format!("voice-model-cancel-{}", model.id)),
                "Cancel",
            )
            .style(ButtonStyle::Outlined)
            .label_size(LabelSize::Small)
            .on_click(move |_, window, cx| {
                voice_input::cancel_model_download(cx, model.id);
                notify_settings(window, cx);
            }),
        );
    } else {
        if on_disk {
            controls = controls.child(
                Button::new(
                    SharedString::from(format!("voice-model-delete-{}", model.id)),
                    "Delete",
                )
                .style(ButtonStyle::Outlined)
                .label_size(LabelSize::Small)
                .on_click(move |_, window, cx| {
                    voice_input::delete_model_download(cx, model.id);
                    notify_settings(window, cx);
                }),
            );
        } else {
            controls = controls.child(
                Button::new(
                    SharedString::from(format!("voice-model-download-{}", model.id)),
                    if failed { "Retry" } else { "Download" },
                )
                .style(ButtonStyle::Outlined)
                .label_size(LabelSize::Small)
                .on_click(move |_, window, cx| {
                    voice_input::start_model_download(cx, model.id);
                    notify_settings(window, cx);
                }),
            );
        }

        if on_disk && !selected {
            controls = controls.child(
                Button::new(
                    SharedString::from(format!("voice-model-use-{}", model.id)),
                    "Use",
                )
                .style(ButtonStyle::Filled)
                .label_size(LabelSize::Small)
                .on_click(move |_, window, cx| {
                    update_settings_file(
                        SettingsUiFile::User,
                        Some("voice_input.local.model"),
                        window,
                        cx,
                        move |content, _| {
                            content
                                .voice_input
                                .get_or_insert_default()
                                .local
                                .get_or_insert_default()
                                .model = Some(model.id.to_string());
                        },
                    )
                    .log_err();
                    notify_settings(window, cx);
                }),
            );
        }
    }

    let mut header = h_flex()
        .w_full()
        .gap_2()
        .justify_between()
        .items_start()
        .child(
            v_flex()
                .gap_1()
                .child(Label::new(model.name).size(LabelSize::Default))
                .child(
                    Label::new(model.description)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new(model.approx_size)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                ),
        )
        .child(controls);

    if selected {
        header = header.child(
            Icon::new(IconName::Check)
                .size(IconSize::Small)
                .color(Color::Accent),
        );
    }

    v_flex()
        .id(SharedString::from(format!("voice-model-{}", model.id)))
        .w_full()
        .px_3()
        .py_2()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().colors().border_variant)
        .child(header)
        .when_some(download, |this, state| {
            this.child(match state.status {
                ModelDownloadStatus::Downloading => v_flex()
                    .gap_1()
                    .child(
                        Label::new(if state.total > 0 {
                            format!(
                                "Downloading… {} of {}",
                                format_bytes(state.downloaded),
                                format_bytes(state.total)
                            )
                        } else {
                            "Downloading…".to_string()
                        })
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
                    .child(ProgressBar::new(
                        SharedString::from(format!("voice-model-progress-{}", model.id)),
                        state.fraction(),
                        1.0,
                        cx,
                    ))
                    .into_any_element(),
                ModelDownloadStatus::Failed => v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Icon::new(IconName::Warning)
                                    .size(IconSize::XSmall)
                                    .color(Color::Error),
                            )
                            .child(
                                Label::new("Download failed")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Error),
                            ),
                    )
                    .when_some(state.error, |this, error| {
                        this.child(
                            Label::new(error)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    })
                    .into_any_element(),
                ModelDownloadStatus::Ready => Label::new("Downloaded")
                    .size(LabelSize::XSmall)
                    .color(Color::Success)
                    .into_any_element(),
            })
        })
        .into_any_element()
}

fn notify_settings(window: &mut Window, cx: &mut App) {
    if let Some(handle) = window.window_handle().downcast::<SettingsWindow>() {
        handle.update(cx, |_, _, cx| cx.notify()).log_err();
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 3] = ["KB", "MB", "GB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
