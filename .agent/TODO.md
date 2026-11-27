# Task

Rework the voice-input (dictation) UX in the personal Zed fork `my-zed`:
mic button must start/stop recording directly, a real model manager
(download/delete/cancel, sizes, status, errors), and a working
`#voice_input` settings page instead of an empty one. Ship via the tag-driven
release workflow.

## Plan

- [x] Split the microphone button from the settings popover (mic = toggle,
      separate chevron = menu).
- [x] Move the mic cluster from next to Send to the left toolbar cluster.
- [x] Add a global download registry in `voice_input` (`downloads.rs`) with
      status/progress/error/cancel/delete.
- [x] Build `settings_ui::pages::voice_input` page (enable switch, engine
      toggle, cloud presets with API-key availability, local model list with
      size/description/status/Download/Delete/Cancel + progress bar) and
      register it as an "AI" subpage with `json_path: "voice_input"`.
- [x] Replace persistent status toasts for downloads with inline state.
- [x] Clarify microphone-start errors in the composer.
- [x] `cargo check`/`clippy --lib` for `voice_input`, `settings_ui`, `agent_ui`
      (clean); `cargo test -p voice_input` (26 pass).
- [x] Commit, tag `v1.31.0-my.7`, push, verify release assets, install into
      `~/.local/my-zed.app`.
- [x] Microphone resilience: iterate over configured / audio-setting / default /
      enumerated devices, retry rejected stream configs, log the device used.
- [x] Microphone picker in Settings → AI → Voice Input (`voice_input.device`).
- [x] Released `v1.31.0-my.8` (device fallback) and `v1.31.0-my.9` (picker +
      config retry), installed my.9 into `~/.local/my-zed.app`.
- [ ] Windows asset appears on the my.9 release once its ~2 h job finishes
      (no action needed).
- [x] Make recording start resilient: `Recorder::start` takes a preference
      list and tries device → call device → system default → every enumerated
      input device, aggregating errors (tag `v1.31.0-my.8`).

## Findings

- `PopoverMenu::trigger_with_tooltip` swallows clicks on the trigger — that was
  the "mic opens the model menu" bug. Fix: separate `IconButton` (mic) and
  `PopoverMenu` trigger (chevron).
- `OpenSettingsAt { path: "voice_input" }` only works if a `SubPageLink` with
  matching `json_path` exists in `settings_ui::settings_data`; arbitrary keys do
  not auto-render. Registered in `ai_page`.
- New `voice_input/src/downloads.rs`: `ModelDownloads` global (atomics per
  model), `start/status/cancel/delete/is_any_downloading`. `Entry` owns the
  `Task`, so dropping the entry cancels the download.
- `cp.background_spawn` needs `AppContext`; `cx.try_global_mut` does not exist —
  use `try_global` then `global_mut`.
- `VoiceInputCloudSettingsContent.api_key` is a field, not a method.
- Download sizes/descriptions live in `LocalModelDef` (`description`,
  `approx_size`).
- Release assets are published per platform as jobs finish; the Linux asset
  (111 MB) is enough to install on this machine.
- Install path `~/.local/my-zed.app` (rsync `--delete` from the extracted
  `zed.app`), verify with `bin/zed --version`.
- Commit `2ed289d` (API-key availability display + mic error hints) landed on
  `main` after the `my.7` tag, so it ships in the next release.
- This cpal fork (`RustAudio/cpal` rev e1612d5) has no `DeviceTrait::name`;
  use `description()?.name()` for a human-readable device name.

## Structure

- `crates/voice_input/src/` — `voice_input.rs` (init, transcribe, recordings),
  `catalog.rs` (cloud presets, local model defs), `local.rs` (download/run),
  `downloads.rs` (global download registry), `recorder.rs`, `transcribe.rs`,
  `wav.rs`, `settings.rs`.
- `crates/settings_ui/src/pages/voice_input.rs` — the Voice Input settings page.
- `crates/settings_ui/src/page_data.rs` — `voice_input_section()` in `ai_page`.
- `crates/agent_ui/src/conversation_view/thread_view/voice_input_ui.rs` —
  composer mic control, dictation pipeline, voice menu.
- `crates/agent_ui/src/conversation_view/thread_view.rs` — toolbar layout,
  `VoiceInputState`/`VoiceTake` fields.

## Next

Nothing blocking. Follow-ups if requested: memory-growth investigation
(`heaptrack`) and faster Windows release builds.
