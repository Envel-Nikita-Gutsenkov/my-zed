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
- Per-chat Auto mode, persisted message queue, resource monitor, token usage
  statistics, and a per-chat task log.

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
  output, cache read/write, total and estimated saved tokens, plus a
  per-bucket bar chart over selectable ranges. `Total` counts every processed
  token, cache hits included, so it lines up with provider dashboards instead
  of under-reporting. Usage is stored as one compact row per request in an
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
  subagent views. While Auto mode is on, routine tool permission prompts (the
  settings-driven "confirm" kind) are resolved as "allow once" instead of
  waiting for a click; disabled tools and explicit deny rules are still
  enforced, and prompts that require choosing between actions (for example run
  sandboxed vs. unsandboxed) still ask.
- Per-chat **Task Log** toggle (checklist button next to the composer). When
  on, the agent maintains a running `.agent/TODO.md` in the project root with
  its plan, key findings and a short file map, written in English and kept to
  roughly 1000 lines so it stays a compact summary rather than a growing log.
  It is re-read at the start of each request, which lets long conversations
  survive context compaction without re-exploring the codebase. Enabling the
  toggle also adds `.agent/` to each open worktree's `.gitignore`.
- Per-chat **browser access** toggle (globe button next to the composer) grants
  the agent the built-in browser tools. The Playwright-driven Chromium is an
  isolated, in-memory browser, and the `browser` server is **on demand**: it is
  not started when a project opens, so no Playwright or Chromium process exists
  until a chat turns browser access on. Its lifetime is bound to the toggle:
  turning browser access off shuts the browser server down (freeing its memory)
  instead of leaving a headless Chromium running until the app exits.

### Resources and telemetry

- The `resources` block is fully configurable (sample interval, history length,
  status bar visibility, log size limit, and `trim_interval_secs`, which
  periodically returns unused heap pages to the OS via `malloc_trim`). The
  status bar shows CPU load with a mini-graph and current memory use.
- The status-bar memory number is the **resident memory of the whole process
  tree**, not just the editor process: it adds up every child the app spawns,
  including language servers and the Playwright-driven Chromium behind the
  browser tools. A large reading can therefore be mostly a child process rather
  than the editor itself. The `memory usage` log line reports the editor
  process alone (resident, live heap, retained heap, `mmap`).
- Memory no longer ratchets upward across a long session. On Linux/glibc the
  process raises `M_MMAP_THRESHOLD`/`M_TRIM_THRESHOLD` at startup so large
  transient buffers are returned to the OS when freed instead of inflating the
  heap, and `resources.trim_interval_secs` now defaults to `60` instead of `0`.
  The hourly `memory usage` log line also reports live heap, retained (free)
  heap, and `mmap` totals so growth can be told apart from fragmentation. Note
  that trimming only returns already-free pages; it does not (and cannot)
  shrink live allocations.
- The number of glibc malloc arenas is now capped at 2 in the binary itself
  (overridable with `MALLOC_ARENA_MAX`) rather than only via the launcher's
  environment, so the heap cannot fragment into one largely-empty arena per
  worker thread when the app is started by other means.
- The built-in `browser` context server is on demand rather than eager: it is
  not spawned at project open, so the baseline process tree no longer includes
  Playwright and a headless Chromium until browser access is actually enabled in
  a chat. This also keeps the browser server out of test and headless projects,
  which previously started `npx @playwright/mcp` on every project.
- `heaptrack` profiling of this fork is documented in the README's Memory
  section: the editor is ~120 MB of heap at idle, open buffers add ~2–3 MB of
  tree-sitter parse trees each, and long sessions grow mainly through wasm
  linear memory (extensions) that only a restart reclaims.
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
