# Task

Make dictation work end-to-end on Linux and Windows in the personal Zed fork
`my-zed`: defaults that can actually transcribe, a managed whisper.cpp engine,
a composer-local status line (never a bottom toast), and well-considered
transcript merging/cleanup.

## Plan

- [x] Mic button split from the chevron menu, moved to the left toolbar.
- [x] Global download registry (`voice_input/src/downloads.rs`).
- [x] `Settings → AI → Voice Input` page in `settings_ui`.
- [x] Microphone picker + device/config fallbacks (`v1.31.0-my.8`, `.9`).
- [x] Remove the persistent bottom `StatusToast` for voice input; use an inline
      `VoiceNotice` above the composer editor, dismissable and auto-cleared on
      success.
- [x] New `crates/voice_input/src/text.rs` with `clean_chunk`, `append_chunk`,
      `prompt_tail` and tests (overlap, punctuation, capitalization,
      hallucinations, annotations).
- [x] Managed whisper.cpp binary install (download + extract + chmod) with
      status in settings and in the composer notice.
- [x] Cloud defaults fixed: Groq is the default preset, OpenRouter preset and its
      provider-routing code removed.
- [x] Local engine is the default engine.
- [x] `cargo check`/`cargo test -p voice_input` (41 pass), `settings_content`
      tests, `clippy --lib` for `voice_input`, `settings_ui`, `agent_ui` (only
      pre-existing warnings in `agent`/`agent_ui`).
- [ ] Commit, tag `v1.31.0-my.10`, push, wait for the Linux asset, install into
      `~/.local/my-zed.app`, confirm the Windows asset appears.

## Findings

- OpenRouter has no `/audio/transcriptions` endpoint; any default pointing there
  fails with 404. Groq (`https://api.groq.com/openai/v1`,
  `whisper-large-v3-turbo`, `GROQ_API_KEY`) is the working default.
- `VoiceInputEngine` default changed to `Local` in
  `crates/settings_content/src/voice_input.rs`; cloud provider fallback is
  `groq`.
- whisper.cpp binaries only exist for `v1.9.2` (and old b-tags); newer tags ship
  none. Windows assets for v1.9.2: `whisper-bin-x64.zip`, `whisper-bin-Win32.zip`
  (no arm64 — x64 runs emulated). Linux: `whisper-bin-ubuntu-{x64,arm64}.tar.gz`.
- Archive extraction uses system `tar` (bsdtar on Windows handles zip).
- `append_chunk` bug fixed: punctuation-only tokens shifted the overlap offset;
  `word_offset` now skips tokens that normalize to empty.
- Capitalization only applies when no overlap was removed (continuation chunks
  keep the model's case).
- `()` attached to a word (`getUserName()`) is preserved; standalone `( )` is
  treated as an annotation.
- Whisper hallucination list lives in `text.rs` and drops whole-chunk
  hallucinations only.
- Notice placement: `.children(self.render_voice_notice(cx))` inside the
  composer `v_flex` (before `message_editor`), so it pushes content instead of
  overlaying the window.
- Default local model is `whisper-large-v3-turbo` (~1.6 GB); the notice now
  names the model and its size.
- Repo rules: `README.md` keeps the two `> [!IMPORTANT]` lines; no `mod.rs`;
  `.agent/TODO.md` is git-ignored, commit with `git add -f`.
- `gh` is unauthenticated; use `curl` against the GitHub API. `cargo` needs
  `PATH=/home/nikita/.cargo/bin:$PATH`.

## Structure

- `crates/voice_input/src/` — `voice_input.rs` (init/transcribe/recordings,
  re-exports), `catalog.rs` (cloud presets, local models, whisper archive),
  `local.rs` (download/extract/run), `downloads.rs` (global registry),
  `text.rs` (cleanup + merging), `recorder.rs`, `transcribe.rs`, `wav.rs`.
- `crates/settings_content/src/voice_input.rs` — settings schema + defaults.
- `crates/settings_ui/src/pages/voice_input.rs` — settings page (engine row,
  cloud presets, local models).
- `crates/agent_ui/src/conversation_view/thread_view/voice_input_ui.rs` —
  mic control, dictation pipeline, `VoiceNotice`, menu.
- `crates/agent_ui/src/conversation_view/thread_view.rs` — composer layout,
  `voice_notice` field, `VoiceInputState`/`VoiceTake`.

## Next

Commit, tag `v1.31.0-my.10`, push, install the Linux build, verify Windows.
