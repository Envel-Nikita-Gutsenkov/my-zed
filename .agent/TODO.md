# Task

Add dictation (voice input) to the agent chat in the `my-zed` fork: a mic button
in the composer, settings for an OpenAI-compatible cloud API and for local
models. Cloud default is OpenRouter `whisper-large-v3-turbo` with cheapest-provider
routing (or a forced provider); local supports Parakeet and Whisper models.

## Plan

- [x] `settings_content`: `VoiceInputSettingsContent` + `voice_input` field
- [x] `crates/voice_input` crate: `wav`, `recorder`, `catalog`, `transcribe`,
      `local`, `settings`, lib root
- [x] Workspace `Cargo.toml`: member + workspace dep
- [x] `settings` crate `vscode_import.rs` initializer
- [x] `agent_ui` integration: `ToggleVoiceInput`, dep, mic button + menu,
      chunked async pipeline, retry
- [x] `zed` needs no change (`agent_ui::init` calls `voice_input::init`)
- [x] keybinding `ctrl-alt-m` in linux/windows/macos keymaps
- [x] Robustness: `split_and_encode`, `finish_samples`, chunked streaming insert,
      cross-chunk prompt context, audio recovery + retry
- [x] Removed unimplemented `activation` knob (no dead settings)
- [x] Tests: wav chunking, prompt context, settings defaults/overrides
- [x] `cargo test -p voice_input -p settings_content` pass
- [x] `cargo clippy -p voice_input` clean; `cargo clippy -p agent_ui --lib` clean
- [x] `cargo fmt --all`
- [x] CHANGELOG `## 1.31.0-my.6`
- [ ] Commit, push, tag `v1.31.0-my.6`, monitor CI, install artifact

## Findings

- Action namespace `agent`; `actions!(agent, [...])` in `crates/agent_ui/src/agent_ui.rs`.
- Composer: `ThreadView::render_message_editor`; mic button added before
  `render_send_button`; voice methods around line 6960-7530 in `thread_view.rs`.
- `voice_input::transcribe(executor, http_client, settings, keychain_key,
  prompt_context, wav) -> Task<Result<String>>`.
- Lints: `std::process::Command` and `smol::Timer`/`async_io::Timer` are
  disallowed; use `smol::process::Command` (async) and
  `gpui::BackgroundExecutor::timer`. `transcribe_cloud` takes `&BackgroundExecutor`.
- Test fixtures in `agent_ui` fail `--all-targets` compile due to pre-existing
  drift (`auto_continue`, `token_savings`); use `--lib` for agent_ui clippy.
- `async_io::Timer` == `smol::Timer` (re-export) so both trip the lint.
- README IMPORTANT banner already present.

## Next

Commit + tag + push + monitor CI, then install `zed-linux-x86_64.tar.gz`.
