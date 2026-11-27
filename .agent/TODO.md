# Task

Ship a working dictation (voice input) experience in the personal Zed fork
`my-zed` on Linux and Windows, plus fix two agent regressions: token-usage
statistics undercounting multi-step turns, and queued messages not draining
after a clean turn. Released through the tag-driven workflow as
`v1.31.0-my.13`.

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
- [x] Release run for `v1.31.0-my.12`: Linux + Windows assets published.
- [x] Install the released my.12 build and verify `bin/zed --version`.
- [x] Fix token-usage undercount: per-request `request_seq` in the stats key.
- [x] Fix queued-message stranding: resume a stale queue pause on a clean
      `EndTurn` before auto-send (`resume_if_paused`).
- [ ] Commit both fixes, push, tag `v1.31.0-my.13`.
- [ ] Poll the release run until Linux + Windows assets exist.
- [ ] Install my.13 into `~/.local/my-zed.app` and verify `bin/zed --version`.

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

Both fixes implemented and compiling:

- Token-usage: fixed in `crates/agent/src/thread.rs` (per-request `request_seq`).
- Queue: `MessageQueue::resume_if_paused()` added; `Stopped`/`EndTurn` branch in
  `conversation_view.rs` re-arms a stale pause before
  `send_queued_after_confirmed_idle`.
- `cargo check -p agent -p agent_ui` clean; agent tests pass.
- agent_ui tests still fail to COMPILE (pre-existing: `auto_continue`,
  `thread_maintenance`, `token_savings` missing in test struct literals in
  `agent_ui.rs`, `thread_metadata_store.rs`, etc.). Not chased per handoff; the
  queue change is validated by compilation + reasoning, not a new test.
- Remaining: commit, push, tag `v1.31.0-my.13`, poll release, install.

### Token-usage statistics findings

- Root cause: `record_token_usage_stats` keyed rows by
  `format!("{thread_id}:{prompt_id}")`, i.e. ONE row per user turn. But a turn
  issues many model requests (one per tool-call step), each resetting
  `current_request_token_usage` at the start of `run_turn_internal`'s loop
  (~L3807) while `prompt_id` stays constant. The DB upsert keeps `MAX(...)`,
  so every step after the first was collapsed into the largest single request
  — undercounting by roughly the number of tool-call steps per turn (~10x here).
- `cumulative_token_usage` / context indicator were already correct; the bug is
  only in the persisted `token_usage_events` stats window
  (`crates/agent_ui/src/token_usage_window.rs`, `ThreadsDatabase::token_usage_buckets`
  which does `SUM(...)` grouped by time bucket).
- Fix: added `Thread.request_seq: u64`; new `begin_request_token_usage()` bumps it
  and resets `current_request_token_usage`; all 3 reset sites (run_turn loop,
  `compact`, compaction in `perform_compaction_if_needed`) use it. Stats key is
  now `format!("{thread_id}:{prompt_id}:{request_seq}")` via
  `usage_stats_request_id()`. `MAX` still dedupes streaming updates within one
  request; distinct steps now get distinct rows that `SUM` correctly.
- Reasoning tokens: provider `completion_tokens` already includes reasoning
  (OpenAI/Anthropic aggregate), so output totals are not the gap.
- Retention is 90 days (`db.rs` TOKEN_USAGE_RETENTION_SECONDS), matching the
  widest 90d range, so retention is not the gap.
- Tests: added `test_usage_stats_request_id_is_unique_per_request` (passes);
  `cargo test -p agent --lib token_usage` (7) and `compaction_usage` pass;
  `cargo clippy -p agent --lib` only a pre-existing `thread_store.rs` warning.

### Queued-message dispatch findings

- Both `AcpThreadEvent::ActivityConfirmedIdle`
  (`conversation_view.rs:2200`) and `AcpThreadEvent::Stopped` (`~2419`) call
  `send_queued_after_confirmed_idle` ->
  `ThreadView::send_queued_message_after_generation_stopped`
  (`thread_view.rs:3185`).
- That method only sends when ALL hold: `foreground_activity() == Idle`,
  `!is_loading_contents`, `processing_state == AutoProcess`, and the front
  queued entry's editor is NOT focused. On a normal `EndTurn` it DOES send.
- The queue is **paused** (and stays paused until the user sends a new message)
  on: manual stop (`cancel_generation`, `stop_current_and_send_new_message`),
  `StopReason::MaxTokens`, `StopReason::Refusal`, `StopReason::Error`, auth
  required, a failed `validate_queued_entry`, and thread recovery restore.
- On MaxTokens/Refusal/Error the handler `return`s before the queue-send code,
  so those turns end with the queue frozen. This is upstream behaviour, not
  introduced by the fork (`git log -S` only shows the base import `730fb04`).
- `add_to_queue` creates a read-only queued editor and does NOT focus it, so
  the focus gate is normally false; it only blocks if the user clicked into
  the queued message to edit it.
- Most likely cause of the user's report: the turn ended with an error/refusal/
  token-limit (or a manual stop), which pauses the queue with no later
  auto-resume. Fixed: on a clean `EndTurn`, a stale `Paused` queue is re-armed
  (`resume_if_paused`) before the auto-send attempt; error/manual-stop paths
  still keep it paused.
