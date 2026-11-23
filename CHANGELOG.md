# my zed — Changelog

`my zed` is a personal, lightweight fork of [Zed](https://github.com/zed-industries/zed)
based on upstream Zed `1.31.0`. This file describes how the fork differs from
upstream Zed. It is not a chronological log: the fork ships as a single version.

## 1.31.0-my.1

### Highlights

- Personal, self-contained build that installs alongside Zed and never
  self-updates to upstream.
- Aggressive, configurable token saving for agent requests.
- No telemetry, no metrics, no mandatory cloud account.
- Per-chat Auto mode, persisted message queue, resource monitor, and token
  usage statistics.

### Agent and token savings

- `agent.tool_output_token_limit` caps a single tool result before it reaches
  the model, so one noisy command cannot flood the context.
- `agent.token_savings` applies per-request context trimming on top of
  auto-compaction instead of replacing it. Every pass is an independent toggle:
  `keep_recent_turns`, `old_tool_output_token_limit`, `strip_ansi`,
  `dedupe_repeated_tool_results`, `drop_old_images`, `strip_old_thinking`,
  `strip_old_thinking_turns`, `total_request_token_budget`, `ephemeral_tools`.
- `strip_old_thinking_turns` removes reasoning only from turns older than the
  threshold, so recent thinking is preserved.
- A turn no longer stops silently when the model returns a completion with only
  reasoning and neither text nor a tool call. Such an empty response resumes
  the turn instead of ending it without doing anything.
- Token usage statistics window (`agent: Open Token Usage Stats`): input,
  output, total and estimated saved tokens, plus a per-bucket bar chart over
  selectable ranges. Usage is stored as one compact row per request in an
  append-only table and pruned after 90 days.

### Long-running chats

- `agent.thread_maintenance` prunes stored chats by age and count, and shrinks
  oversized chats by replacing old turns with a summary while keeping the most
  recent messages intact. All limits are settings.

### Chat UX

- Per-turn stats under the response: duration, token count and completion time.
- Message queue shows positions and highlights the next message to be sent,
  with controls to reorder entries.
- The queue is persisted and survives a crash or restart; a restored queue
  starts paused.
- Auto mode is per chat, persisted, marked in the chat list, and hidden for
  subagent views.

### Resources and telemetry

- The `resources` block is fully configurable (sample interval, history length,
  status bar visibility, log size limit, and an optional `trim_interval_secs`
  that periodically returns unused heap pages to the OS via `malloc_trim`). The
  status bar shows CPU load with a mini-graph and current memory use.
- Microsoft's mimalloc is available as an opt-in Cargo feature (`--features
  mimalloc`). It is not enabled by default because, measured on this fork, it
  used more resident memory than glibc unless also tuned with
  `MIMALLOC_ARENA_EAGER_COMMIT=0 MIMALLOC_PURGE_DELAY=0`. See the README for the
  numbers.
- Telemetry, metrics and auto-update are disabled by default. Auto-update is
  disabled at build level, so the fork cannot silently replace itself with
  upstream Zed. Sign-in, the user menu and the avatar are hidden.

### Naming and installation

- The app is renamed to `my zed` with its own app id, name, menu, About dialog
  and launcher, and installs alongside Zed without interfering with it.
- Existing settings are picked up automatically (config, themes, keymaps,
  providers and API keys from the system keyring). Extensions and language
  servers are reused through symlinks; only working data is kept separate.

### Build

- `release-lite` profile (no LTO or debug info) for fast, low-memory local
  builds, and a `build-fast` profile for iterating on the UI.
- The about/version string includes a short commit hash so local builds are
  easy to tell apart.

[1.31.0-my.1]: https://github.com/Envel-Nikita-Gutsenkov/my-zed/releases/tag/v1.31.0-my.1
