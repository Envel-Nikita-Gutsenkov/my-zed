//! Microphone control and dictation flow for the message composer.
//!
//! Kept out of `thread_view.rs` so that file stays navigable: everything the
//! dictation feature owns (recording toggle, transcription pipeline, engine and
//! model menu) lives here.

use super::*;

/// A short status line shown inside the composer.
///
/// Dictation problems used to be raised as status toasts, which covered the
/// bottom of the window and, for errors, never expired. They are reported
/// inline instead: always dismissible, never overlapping other UI, and they
/// disappear as soon as a new recording starts.
pub(super) struct VoiceNotice {
    text: SharedString,
    is_error: bool,
    /// Offer "Retry" for a take that still has untranscribed chunks.
    can_retry: bool,
    /// Offer "Install" when the local engine or its model is missing.
    can_install: bool,
}

impl VoiceNotice {
    fn error(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
            can_retry: false,
            can_install: false,
        }
    }

    fn info(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
            can_retry: false,
            can_install: false,
        }
    }

    fn with_retry(mut self) -> Self {
        self.can_retry = true;
        self
    }

    fn with_install(mut self) -> Self {
        self.can_install = true;
        self
    }
}

/// Explains which part of the local engine is still being fetched.
fn local_install_progress_text(
    binary_missing: bool,
    model_missing: Option<(&str, &str)>,
) -> SharedString {
    let model = match model_missing {
        Some((name, size)) => format!("the {name} model ({size})"),
        None => String::new(),
    };
    match (binary_missing, model_missing.is_some()) {
        (true, true) => format!(
            "Downloading the local speech engine and {model} in the background. Progress is on \
             Settings → AI → Voice Input; press the microphone again when it finishes."
        )
        .into(),
        (true, false) => "Downloading the local speech engine in the background. Progress is \
             on Settings → AI → Voice Input; press the microphone again when it finishes."
            .into(),
        (false, true) => format!(
            "Downloading {model} in the background. Progress is on Settings → AI → Voice \
             Input; press the microphone again when it finishes."
        )
        .into(),
        (false, false) => SharedString::from("The local speech engine is ready."),
    }
}

/// Progress of an in-flight dictation in the message editor.
pub(super) enum VoiceInputState {
    Idle,
    Recording {
        recorder: Option<voice_input::Recorder>,
    },
    Transcribing,
}

/// A dictation broken into per-request chunks, with the index of the next chunk
/// to transcribe and the text produced so far.
pub(super) struct VoiceTake {
    chunks: Vec<Vec<u8>>,
    next: usize,
    transcript: String,
}

impl VoiceInputState {
    fn is_recording(&self) -> bool {
        matches!(self, VoiceInputState::Recording { .. })
    }

    fn is_busy(&self) -> bool {
        self.is_recording() || matches!(self, VoiceInputState::Transcribing)
    }
}

impl ThreadView {
    /// The microphone toggle plus a small chevron that opens the dictation
    /// settings menu.
    ///
    /// They are deliberately separate controls: wrapping the microphone in the
    /// popover made every click open the menu instead of starting a recording.
    pub(super) fn render_voice_input_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        if !VoiceInputSettings::get_global(cx).content().enabled() {
            return div().into_any_element();
        }

        let (icon, tooltip, selected) = match &self.voice_input {
            VoiceInputState::Idle => (IconName::Mic, "Voice Input", false),
            VoiceInputState::Recording { .. } => {
                (IconName::MicMute, "Stop Recording and Transcribe", true)
            }
            VoiceInputState::Transcribing => (IconName::Mic, "Transcribing…", false),
        };

        let is_busy = self.voice_input.is_busy();
        let can_retry = self.last_voice_take.is_some();
        let weak = cx.weak_entity();

        let mic = IconButton::new("voice-input", icon)
            .icon_size(IconSize::Small)
            .icon_color(Color::Muted)
            .aria_label(tooltip)
            .toggle_state(selected)
            .selected_icon(icon)
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_voice_input(window, cx);
            }))
            .when(selected, |button| {
                button.style(ButtonStyle::Tinted(TintColor::Error))
            })
            .when(!is_busy, |button| {
                button.tooltip(Tooltip::for_action_title(tooltip, &ToggleVoiceInput))
            })
            .when(is_busy, |button| button.tooltip(Tooltip::text(tooltip)));

        let menu = PopoverMenu::new("voice-input-menu")
            .trigger_with_tooltip(
                IconButton::new("voice-input-settings", IconName::ChevronDown)
                    .icon_size(IconSize::XSmall)
                    .icon_color(Color::Muted)
                    .aria_label("Voice Input Settings"),
                Tooltip::text("Voice Input Settings"),
            )
            .anchor(gpui::Anchor::BottomLeft)
            .offset(gpui::Point {
                x: px(0.0),
                y: px(2.0),
            })
            .menu(move |window, cx| {
                let content = VoiceInputSettings::get_global(cx).content().clone();
                Some(Self::build_voice_input_menu(
                    weak.clone(),
                    content,
                    can_retry,
                    window,
                    cx,
                ))
            });

        h_flex().gap_0p5().child(mic).child(menu).into_any_element()
    }

    fn build_voice_input_menu(
        weak: WeakEntity<Self>,
        content: voice_input::VoiceInputSettingsContent,
        can_retry: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<ContextMenu> {
        let engine = content.engine();
        let cloud = content.cloud();
        let local = content.local();

        ContextMenu::build(window, cx, move |menu, _window, _cx| {
            let weak_for_engine = weak.clone();
            let set_engine = move |target: voice_input::VoiceInputEngine| {
                let weak = weak_for_engine.clone();
                move |_: &mut Window, cx: &mut App| {
                    weak.update(cx, |this, cx| this.set_voice_engine(target, cx))
                        .ok();
                }
            };

            let weak_for_cloud = weak.clone();
            let cloud_provider = cloud.provider().to_string();
            let weak_for_local = weak.clone();
            let local_model = local.model().to_string();
            let weak_for_retry = weak.clone();

            menu.header("Voice Input")
                .toggleable_entry(
                    "Cloud API",
                    engine == voice_input::VoiceInputEngine::Cloud,
                    IconPosition::End,
                    None,
                    set_engine(voice_input::VoiceInputEngine::Cloud),
                )
                .toggleable_entry(
                    "Local Model",
                    engine == voice_input::VoiceInputEngine::Local,
                    IconPosition::End,
                    None,
                    set_engine(voice_input::VoiceInputEngine::Local),
                )
                .separator()
                .submenu("Cloud Provider", move |menu, _window, _cx| {
                    let mut menu = menu;
                    for preset in voice_input::CLOUD_PRESETS {
                        let weak = weak_for_cloud.clone();
                        let id = preset.id;
                        menu = menu.toggleable_entry(
                            preset.name,
                            cloud_provider == id,
                            IconPosition::End,
                            None,
                            move |_: &mut Window, cx: &mut App| {
                                weak.update(cx, |this, cx| this.set_cloud_provider(id, cx))
                                    .ok();
                            },
                        );
                    }
                    menu
                })
                .submenu("Local Model", move |menu, _window, cx| {
                    let mut menu = menu;
                    for model in voice_input::LOCAL_MODELS {
                        let weak = weak_for_local.clone();
                        let id = model.id;
                        let state = match voice_input::model_download_status(cx, id) {
                            Some(download) if download.is_downloading() => {
                                format!("downloading {:.0}%", download.fraction() * 100.0)
                            }
                            Some(download)
                                if download.status == voice_input::ModelDownloadStatus::Failed =>
                            {
                                "download failed".to_string()
                            }
                            _ if voice_input::local_model(id).is_some_and(|model| {
                                voice_input::model_is_downloaded(
                                    VoiceInputSettings::get_global(cx),
                                    model,
                                )
                            }) =>
                            {
                                "downloaded".to_string()
                            }
                            _ => format!("not downloaded, {}", model.approx_size),
                        };
                        let label = SharedString::from(format!("{} — {state}", model.name));
                        menu = menu.toggleable_entry(
                            label,
                            local_model == id,
                            IconPosition::End,
                            None,
                            move |_window: &mut Window, cx: &mut App| {
                                weak.update(cx, |this, cx| this.set_local_model(id, cx))
                                    .ok();
                            },
                        );
                    }
                    menu
                })
                .submenu("Manage Downloads", move |menu, _window, cx| {
                    let mut menu = menu;
                    for model in voice_input::LOCAL_MODELS {
                        let id = model.id;
                        let status = voice_input::model_download_status(cx, id);
                        let present = voice_input::local_model(id).is_some_and(|model| {
                            voice_input::model_is_downloaded(
                                VoiceInputSettings::get_global(cx),
                                model,
                            )
                        });
                        let downloading = status
                            .as_ref()
                            .is_some_and(voice_input::ModelDownload::is_downloading);

                        if downloading {
                            menu = menu.entry(
                                SharedString::from(format!("Cancel {} download", model.name)),
                                None,
                                move |_: &mut Window, cx: &mut App| {
                                    voice_input::cancel_model_download(cx, id);
                                },
                            );
                        } else if present {
                            menu = menu.entry(
                                SharedString::from(format!(
                                    "Delete {} (frees {})",
                                    model.name, model.approx_size
                                )),
                                None,
                                move |_: &mut Window, cx: &mut App| {
                                    voice_input::delete_model_download(cx, id);
                                },
                            );
                        } else {
                            menu = menu.entry(
                                SharedString::from(format!(
                                    "Download {} ({})",
                                    model.name, model.approx_size
                                )),
                                None,
                                move |_: &mut Window, cx: &mut App| {
                                    voice_input::start_model_download(cx, id);
                                },
                            );
                        }
                    }
                    menu
                })
                .separator()
                .when(can_retry, |menu| {
                    menu.entry(
                        "Retry Last Dictation",
                        None,
                        move |window: &mut Window, cx: &mut App| {
                            weak_for_retry
                                .update(cx, |this, cx| this.retry_voice_transcription(window, cx))
                                .ok();
                        },
                    )
                })
                .entry(
                    "Open Voice Input Settings",
                    None,
                    move |window: &mut Window, cx: &mut App| {
                        window.dispatch_action(
                            zed_actions::OpenSettingsAt {
                                path: "voice_input".into(),
                                target: None,
                            }
                            .boxed_clone(),
                            cx,
                        );
                    },
                )
        })
    }

    fn set_voice_engine(&mut self, engine: voice_input::VoiceInputEngine, cx: &mut Context<Self>) {
        let fs = self.thread.read(cx).project().read(cx).fs().clone();
        update_settings_file(fs, cx, move |settings, _cx| {
            settings.voice_input.get_or_insert_default().engine = Some(engine);
        });
        cx.notify();
    }

    fn set_cloud_provider(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let fs = self.thread.read(cx).project().read(cx).fs().clone();
        update_settings_file(fs, cx, move |settings, _cx| {
            let voice = settings.voice_input.get_or_insert_default();
            voice.engine = Some(voice_input::VoiceInputEngine::Cloud);
            voice.cloud.get_or_insert_default().provider = Some(provider.to_string());
        });
        cx.notify();
    }

    fn set_local_model(&mut self, model_id: &'static str, cx: &mut Context<Self>) {
        let fs = self.thread.read(cx).project().read(cx).fs().clone();
        update_settings_file(fs, cx, move |settings, _cx| {
            let voice = settings.voice_input.get_or_insert_default();
            voice.engine = Some(voice_input::VoiceInputEngine::Local);
            voice.local.get_or_insert_default().model = Some(model_id.to_string());
        });
        // Picking a model that is not on disk is an explicit request to use it,
        // so fetch it right away instead of failing on the first dictation.
        if voice_input::local_model(model_id).is_some_and(|model| {
            !voice_input::model_is_downloaded(VoiceInputSettings::get_global(cx), model)
        }) {
            self.start_model_download(model_id, cx);
        }
        cx.notify();
    }

    /// Starts the model download in the background if it is not running yet.
    ///
    /// The download itself is owned by the `voice_input` global so it survives
    /// menu and window changes; progress and cancellation are shown in the menu
    /// and on the Voice Input settings page, so no notification is raised here.
    fn start_model_download(&mut self, model_id: &'static str, cx: &mut Context<Self>) {
        voice_input::start_model_download(cx, model_id);
        cx.notify();
    }

    /// Starts whatever the selected engine still needs: the whisper.cpp binary
    /// for local dictation and the chosen model. Used by the inline notice, so a
    /// first-time user can fix a missing local engine without hunting through
    /// settings.
    fn install_voice_dependencies(&mut self, cx: &mut Context<Self>) {
        let content = VoiceInputSettings::get_global(cx).content().clone();
        if content.engine() != voice_input::VoiceInputEngine::Local {
            return;
        }

        let binary_missing = !voice_input::whisper_binary_ready();
        let model_id = content.local().model().to_string();
        let model = voice_input::local_model(&model_id);
        let model_missing = model.is_some_and(|model| {
            !voice_input::model_is_downloaded(VoiceInputSettings::get_global(cx), model)
        });

        let binary_started = binary_missing && voice_input::start_whisper_install(cx);
        let model_started = model_missing
            && model.is_some_and(|model| voice_input::start_model_download(cx, model.id));

        let mut notice = VoiceNotice::info(local_install_progress_text(
            binary_missing,
            model.map(|model| (model.name, model.approx_size)),
        ));
        // A download that failed (or was cancelled) is restarted from the notice;
        // while one is in flight the same click would be a no-op.
        if (binary_missing && !binary_started) || (model_missing && !model_started) {
            notice = notice.with_install();
        }
        self.set_voice_notice(notice, cx);
    }

    /// Whether a transcription attempt can start right now.
    ///
    /// The cloud engine has nothing to prepare. The local engine needs both the
    /// whisper.cpp binary and the selected model, and whichever is missing is
    /// installed instead of starting a recording that could not be transcribed.
    fn ensure_voice_engine_ready(&mut self, cx: &mut Context<Self>) -> bool {
        let content = VoiceInputSettings::get_global(cx).content().clone();
        if content.engine() != voice_input::VoiceInputEngine::Local {
            return true;
        }

        let model_id = content.local().model().to_string();
        let Some(model) = voice_input::local_model(&model_id) else {
            self.set_voice_notice(
                VoiceNotice::error(format!("Unknown local transcription model {model_id}")),
                cx,
            );
            return false;
        };

        let binary_ready = voice_input::whisper_binary_ready();
        let model_ready =
            voice_input::model_is_downloaded(VoiceInputSettings::get_global(cx), model);
        if binary_ready && model_ready {
            return true;
        }

        self.install_voice_dependencies(cx);
        false
    }

    pub(super) fn toggle_voice_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.voice_input.is_recording() {
            self.stop_voice_input(window, cx);
        } else {
            self.start_voice_input(window, cx);
        }
    }

    fn start_voice_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ensure_voice_engine_ready(cx) {
            return;
        }
        match voice_input::Recorder::start_from_settings(cx) {
            Ok(recorder) => {
                self.voice_input = VoiceInputState::Recording {
                    recorder: Some(recorder),
                };
                self.voice_notice = None;
                cx.notify();
                self.spawn_auto_stop_watcher(window, cx);
            }
            Err(error) => {
                self.set_voice_notice(
                    VoiceNotice::error(format!(
                        "Could not start recording: {error:#}. Another application may be using \
                         the microphone, or another input device can be picked under \
                         Settings → AI → Voice Input."
                    )),
                    cx,
                );
            }
        }
    }

    fn spawn_auto_stop_watcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let still_recording = this.update_in(cx, |this, window, cx| {
                    let reason = match &this.voice_input {
                        VoiceInputState::Recording {
                            recorder: Some(recorder),
                        } => {
                            let content = VoiceInputSettings::get_global(cx).content();
                            voice_input::auto_stop_reason(
                                recorder.elapsed(),
                                recorder.silence_duration(),
                                content,
                            )
                        }
                        _ => return false,
                    };
                    if reason.is_some() {
                        this.stop_voice_input(window, cx);
                    }
                    true
                });
                match still_recording {
                    Ok(true) => {}
                    _ => break,
                }
            }
        })
        .detach();
    }

    fn stop_voice_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let recorder = match std::mem::replace(&mut self.voice_input, VoiceInputState::Idle) {
            VoiceInputState::Recording {
                recorder: Some(recorder),
            } => recorder,
            other => {
                self.voice_input = other;
                return;
            }
        };

        let content = VoiceInputSettings::get_global(cx).content().clone();
        if voice_input::is_too_short(recorder.elapsed(), &content) {
            self.set_voice_notice(
                VoiceNotice::error("Recording was too short to transcribe"),
                cx,
            );
            return;
        }

        let (samples, sample_rate) = match recorder.finish_samples() {
            Ok(samples) => samples,
            Err(error) => {
                self.set_voice_notice(
                    VoiceNotice::error(format!("Could not finish recording: {error:#}")),
                    cx,
                );
                return;
            }
        };

        // Split into independent 16 kHz WAV chunks so a single slow or failing
        // request cannot discard the whole dictation. Each chunk is transcribed
        // separately and its text is inserted as soon as it arrives.
        let chunks = voice_input::split_and_encode(
            &samples,
            1,
            sample_rate,
            content.chunk_secs(),
            content.silence_threshold(),
        );
        drop(samples);

        if chunks.is_empty() {
            self.set_voice_notice(
                VoiceNotice::info("No speech was detected in that recording"),
                cx,
            );
            return;
        }

        self.last_voice_take = Some(VoiceTake {
            chunks,
            next: 0,
            transcript: String::new(),
        });
        cx.notify();
        self.run_voice_transcription(window, cx);
    }

    /// Transcribes the pending chunks of `last_voice_take`, inserting text as it
    /// arrives and leaving the remainder in place if a request fails.
    fn run_voice_transcription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.voice_input, VoiceInputState::Idle) {
            return;
        }
        let Some(take) = self.last_voice_take.take() else {
            return;
        };
        self.voice_input = VoiceInputState::Transcribing;
        cx.notify();

        let settings = VoiceInputSettings::get_global(cx).clone();
        let content = settings.content().clone();
        let insertion = content.insertion();
        let streaming = insertion.streaming();
        let trailing_space = insertion.trailing_space();
        let auto_submit = insertion.auto_submit();
        let retain_audio = content.retain_audio();
        let context_chars = content.context_chars();
        let keychain_endpoint = voice_input::keychain_endpoint(&settings);
        let client = cx.http_client();
        let executor = cx.background_executor().clone();

        cx.spawn_in(window, async move |this, cx| {
            let keychain_key = match keychain_endpoint {
                Some(endpoint) => match cx.update(|_window, cx| cx.read_credentials(&endpoint)) {
                    Ok(task) => match task.await {
                        Ok(Some((_, bytes))) => voice_input::parse_credential_api_key(&bytes),
                        _ => None,
                    },
                    Err(_) => None,
                },
                None => None,
            };

            let mut take = take;
            let mut failure: Option<anyhow::Error> = None;

            while take.next < take.chunks.len() {
                let chunk = take.chunks[take.next].clone();
                let prompt_context = (context_chars > 0)
                    .then(|| voice_input::prompt_tail(&take.transcript, context_chars))
                    .flatten();
                let result = voice_input::transcribe(
                    &executor,
                    client.clone(),
                    settings.clone(),
                    keychain_key.clone(),
                    prompt_context,
                    chunk,
                )
                .await;

                match result {
                    Ok(text) => {
                        take.next += 1;
                        // `append_chunk` drops silence annotations, repeated overlap
                        // between chunks and duplicated text, then spaces the result.
                        let before = take.transcript.len();
                        voice_input::append_chunk(&mut take.transcript, &text);
                        if take.transcript.len() == before {
                            continue;
                        }
                        if streaming {
                            let inserted = take.transcript[before..].to_string();
                            this.update_in(cx, |this, window, cx| {
                                this.insert_voice_text(&inserted, window, cx);
                            })
                            .ok();
                        }
                    }
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }

            this.update_in(cx, |this, window, cx| {
                this.finish_voice_take(
                    take,
                    streaming,
                    trailing_space,
                    auto_submit,
                    retain_audio,
                    failure,
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_voice_take(
        &mut self,
        mut take: VoiceTake,
        streaming: bool,
        trailing_space: bool,
        auto_submit: bool,
        retain_audio: bool,
        failure: Option<anyhow::Error>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.voice_input = VoiceInputState::Idle;

        let total = take.chunks.len();
        let completed = take.next;

        // In non-streaming mode nothing has been inserted yet, so insert what did
        // get transcribed before reporting a failure: spoken text is never lost.
        if !streaming && !take.transcript.is_empty() {
            // Leave a separating space so the remainder inserted by a retry does
            // not run into the partial text, even when `trailing_space` is off.
            let add_space = trailing_space || failure.is_some();
            let text = if add_space {
                format!("{} ", take.transcript)
            } else {
                take.transcript.clone()
            };
            self.insert_voice_text(&text, window, cx);
            // The partial transcript now lives in the editor; clear the
            // accumulator so a retry only appends the remaining chunks instead of
            // inserting the same text twice.
            if failure.is_some() {
                take.transcript.clear();
            }
        }

        match failure {
            Some(error) => {
                // Persist the audio that was not transcribed yet so it survives a
                // restart, and keep it in memory for an in-session retry.
                let remaining = &take.chunks[take.next.min(total)..];
                let saved_note = match voice_input::save_recordings(remaining) {
                    Ok(paths) if !paths.is_empty() => {
                        format!(" Audio saved to {}.", paths[0].display())
                    }
                    Ok(_) => String::new(),
                    Err(error) => {
                        log::warn!("voice input: could not save audio: {error:#}");
                        String::new()
                    }
                };
                self.last_voice_take = Some(take);
                self.set_voice_notice(
                    VoiceNotice::error(format!(
                        "Transcribed {completed} of {total} parts, then stopped: {error:#}.{saved_note} \
                         The recorded audio is kept, so Retry continues from the same place."
                    ))
                    .with_retry(),
                    cx,
                );
            }
            None => {
                if streaming && trailing_space && !take.transcript.is_empty() {
                    self.insert_voice_text(" ", window, cx);
                }
                if retain_audio {
                    voice_input::save_recordings(&take.chunks).log_err();
                }
                self.last_voice_take = None;
                if take.transcript.is_empty() && completed > 0 {
                    // Every chunk came back empty or was filtered as a hallucination,
                    // which usually means the microphone only picked up noise.
                    self.set_voice_notice(
                        VoiceNotice::info(
                            "Nothing was transcribed: the recording was silence or noise. Check \
                             the microphone in Settings → AI → Voice Input.",
                        ),
                        cx,
                    );
                } else {
                    self.voice_notice = None;
                }
                if auto_submit && !take.transcript.is_empty() {
                    self.send(window, cx);
                }
            }
        }
        cx.notify();
    }

    fn insert_voice_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = if voice_input::needs_leading_space(&self.message_editor.read(cx).text(cx), text)
        {
            format!(" {text}")
        } else {
            text.to_string()
        };
        self.message_editor.update(cx, |editor, cx| {
            editor.insert_text(&text, window, cx);
            editor.focus_handle(cx).focus(window, cx);
        });
    }

    fn retry_voice_transcription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.last_voice_take.is_some() && matches!(self.voice_input, VoiceInputState::Idle) {
            self.run_voice_transcription(window, cx);
        }
    }

    /// The inline dictation status line, if there is something to report.
    pub(super) fn render_voice_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notice = self.voice_notice.as_ref()?;
        let color = if notice.is_error {
            Color::Error
        } else {
            Color::Muted
        };
        let icon = if notice.is_error {
            IconName::Warning
        } else {
            IconName::Mic
        };

        let mut actions = h_flex().flex_shrink_0().gap_1().items_center();
        if notice.can_install {
            actions = actions.child(
                Button::new("voice-notice-install", "Install")
                    .style(ButtonStyle::Tinted(TintColor::Accent))
                    .label_size(LabelSize::Small)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.install_voice_dependencies(cx);
                    })),
            );
        }
        if notice.can_retry {
            actions = actions.child(
                Button::new("voice-notice-retry", "Retry")
                    .style(ButtonStyle::Outlined)
                    .label_size(LabelSize::Small)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.retry_voice_transcription(window, cx);
                    })),
            );
        }
        actions = actions.child(
            IconButton::new("voice-notice-dismiss", IconName::Close)
                .icon_size(IconSize::XSmall)
                .icon_color(Color::Muted)
                .tooltip(Tooltip::text("Dismiss"))
                .on_click(cx.listener(|this, _, _window, cx| {
                    this.voice_notice = None;
                    cx.notify();
                })),
        );

        Some(
            h_flex()
                .id("voice-notice")
                .mb_1()
                .px_2()
                .py_1()
                .gap_2()
                .items_center()
                .rounded_sm()
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .child(Icon::new(icon).size(IconSize::XSmall).color(color))
                .child(
                    div().flex_1().min_w_0().child(
                        Label::new(notice.text.clone())
                            .size(LabelSize::XSmall)
                            .color(color),
                    ),
                )
                .child(actions)
                .into_any_element(),
        )
    }

    fn set_voice_notice(&mut self, notice: VoiceNotice, cx: &mut Context<Self>) {
        self.voice_notice = Some(notice);
        cx.notify();
    }
}
