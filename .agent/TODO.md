# Task

Rework voice input UX in the personal `my-zed` fork: mic button must record (not open a menu), move it away from Send, replace hanging toast labels with a real model manager (size/status/download/delete), and make `#voice_input` settings page non-empty.

## Plan

- [x] Move dictation code out of `thread_view.rs` into `conversation_view/thread_view/voice_input_ui.rs`
- [x] Split mic button (records) from chevron (opens menu)
- [x] Move the two controls into the left composer toolbar cluster
- [x] Add `crates/voice_input/src/downloads.rs`: global download state, cancel, delete
- [x] Add `crates/settings_ui/src/pages/voice_input.rs`: real settings page with model manager
- [x] Register "Voice Input" subpage under AI page with `json_path: "voice_input"`
- [x] Replace persistent toasts: dismissable, errors stay until closed
- [x] Menu: per-model status, select, download, cancel, delete
- [x] CHANGELOG `1.31.0-my.7`
- [x] check + tests + clippy (lib) for voice_input / settings_ui / agent_ui
- [ ] Commit, tag `v1.31.0-my.7`, push, watch release
- [ ] Install into `~/.local/my-zed.app`

## Findings

- `PopoverMenu::trigger_with_tooltip` swallows clicks on the trigger — that was the "mic opens model list" bug. Separate action button + menu trigger fixes it.
- Settings UI is a curated list; arbitrary `#keys` only work if a page/subpage declares that `json_path`. `#voice_input` now resolves to exactly one `SubPageLink`, so `OpenSettingsAt` auto-opens the page.
- Download state lives in a `gpui::Global` (`voice_input::downloads`), so it survives menu/page close and is shared by the composer menu and the settings page; the `Task` inside the entry cancels the download when dropped.
- `StatusToast` supports `.auto_dismiss(...)` and `.dismiss_button(true)` — used to stop unremovable banners.
- Custom lints: use `smol::process::Command`; `smol::Timer` is denied.
- Local `cargo check` only; full release builds happen in CI (the machine OOMs otherwise).

## Structure

- `crates/voice_input/src/`: `catalog.rs` (models/presets), `downloads.rs` (global download state), `local.rs` (download/unpack/exec), `recorder.rs`, `transcribe.rs`, `wav.rs`, `voice_input.rs` (facade, default-deny exports).
- `crates/settings_ui/src/pages/voice_input.rs` — settings page.
- `crates/settings_ui/src/page_data.rs` — `voice_input_section()` inside `ai_page`.
- `crates/agent_ui/src/conversation_view/thread_view/voice_input_ui.rs` — button, menu, dictation pipeline.
