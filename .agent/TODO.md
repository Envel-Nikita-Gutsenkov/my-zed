# Task

Ship a working dictation (voice input) experience in the personal Zed fork
`my-zed` on Linux and Windows: mic button records directly, real model manager,
working `Settings → AI → Voice Input`, no floating bottom toast, and sensible
transcription/cleanup algorithms. Released through the tag-driven workflow as
`v1.31.0-my.12`.

## Plan

- [x] Mic button starts/stops recording directly, moved to the left toolbar.
- [x] Download registry with status/progress/error/cancel/delete.
- [x] Settings page registered as an AI subpage (`json_path: voice_input`).
- [x] Replace bottom status toasts with an inline, dismissable composer notice.
- [x] Local engine becomes the default; auto-install `whisper.cpp` binary +
      model on first use, with progress.
- [x] Cloud preset default switched from OpenRouter (no
      `/audio/transcriptions`) to Groq; OpenRouter preset and the provider
      routing it needed removed. Attempt ordering extracted into
      `provider_pins` with tests.
- [x] Transcript cleanup: overlap dedup, punctuation/case joining,
      hallucination/annotation dropping (`crates/voice_input/src/text.rs`).
- [x] Silence trimming before transcription; silent takes reported instead of
      sent to the model.
- [x] Windows release build fixed (`async-process` `CommandExt` instead of
      `std`'s, which has no `creation_flags`).
- [x] Drop the pinned "Command Code Compact" compaction/summary provider from
      `assets/settings/default.json` so `/compact` uses the default model.
- [x] `cargo test -p voice_input` (48), `cargo test -p settings_content`,
      `cargo check -p voice_input -p settings_ui -p agent_ui` clean; clippy has
      only pre-existing warnings in untouched files.
- [ ] Release run for `v1.31.0-my.12`: Linux asset published, Windows asset
      published.
- [ ] Install the released build into `~/.local/my-zed.app` and verify
      `bin/zed --version`.

## Findings

- OpenRouter has no transcription endpoint, so the previous default cloud path
  could never work; Groq (`https://api.groq.com/openai/v1`,
  `whisper-large-v3-turbo`, `GROQ_API_KEY`) is the default now. Only aggregators
  support provider routing, so `supports_provider_routing` is false for every
  remaining preset and the "cheapest provider" lookup is never triggered.
- `smol::process::Command` (async-process 2.5.0, Zed fork) needs
  `smol::process::windows::CommandExt` for `creation_flags`. Using `std`'s
  `CommandExt` compiles on Linux and fails only on Windows — that broke the
  `v1.31.0-my.10`/`.11` Windows release builds.
- whisper.cpp release `v1.9.2` ships `whisper-bin-x64.zip` (x64 + Win32) and
  `whisper-bin-ubuntu-{x64,arm64}.tar.gz`; newer tags ship no binaries. The
  Windows arm64 case falls back to the x64 zip (runs under emulation).
  Binaries find their sibling `.so`s via RPATH; verified `whisper-cli` from the
  tarball transcribes `samples/jfk.wav` correctly with `-mc 0 -np -nt`.
- `whisper-cli` accepts `-l auto` for language detection; that is what an unset
  `voice_input.language` maps to.
- Compaction falls back to the thread's model when
  `agent.compaction_model` is unset (`crates/agent/src/thread.rs:5551`).
- Release workflow concurrency group is `release-<ref>` with
  `cancel-in-progress: false`, so re-pushing the same tag queues behind the old
  run; bump the version instead of retagging when a run is already in flight.
- Local user config lives at `~/.config/zed/settings.json`; its pinned
  compaction/summary providers were removed (backup: `settings.json.bak`).

## Structure

- `crates/voice_input/src/{catalog,downloads,local,recorder,text,transcribe,wav}.rs`
- `crates/settings_content/src/voice_input.rs` (settings schema + defaults)
- `crates/settings_ui/src/pages/voice_input.rs` (AI → Voice Input page)
- `crates/agent_ui/src/conversation_view/thread_view/voice_input_ui.rs`
  (composer mic control, notice, dictation pipeline, voice menu)
- `crates/agent_ui/src/conversation_view/thread_view.rs` (composer layout)
- `assets/settings/default.json` (agent defaults, `openai_compatible` presets)

## Next

Poll the `v1.31.0-my.12` release run, install the Linux asset, then confirm the
Windows asset appears.
