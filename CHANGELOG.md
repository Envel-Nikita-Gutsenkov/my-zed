# my zed — Changelog

`my zed` is a personal, lightweight fork of [Zed](https://github.com/zed-industries/zed)
based on upstream Zed `1.31.0`. This file describes how the fork differs from
upstream Zed. It is not a chronological log: the fork ships as a single version.

## 1.31.0-my.8

### Voice input

- Starting a recording now tries every available microphone before giving up:
  the device from `voice_input.device`, then the call device from `audio`
  settings, the system default, and finally any enumerable input device. A
  disconnected or busy headset no longer blocks dictation, and the error names
  the microphones that were tried.

## 1.31.0-my.7

### Voice input UX

- The microphone button now starts and stops recording directly; a separate
  chevron next to it opens the dictation menu. The button sits in the left
  toolbar cluster, away from Send.
- New `Settings → AI → Voice Input` page: enable switch, engine choice, cloud
  provider overview, and a model manager that lists every local model with its
  size, description, live download progress, and Download / Cancel / Delete /
  Use buttons. `#voice_input` in the settings search now opens this page.
- Local model downloads are owned by the app instead of a menu: they keep
  running when the menu closes, show a progress bar in settings, and can be
  cancelled or deleted from disk.
- Dictation notices are dismissable and no longer linger at the bottom of the
  window; download errors are reported where the download started.

### Voice input

- Dictate prompts into the agent panel with the microphone button. Start and
  stop with the button or `ctrl-alt-m`, and the transcript is inserted into the
  message editor. Long dictations are split into chunks that are transcribed
  in order, so a failed request never discards what you already said, and the
  audio is kept on disk for retrying.
- Dictation is configured entirely through the new `voice_input` settings
  section: engine (`cloud` or `local`), language, input device, chunk length,
  silence auto-stop, insertion behaviour (`trailing_space`, `auto_submit`,
  `streaming`) and audio retention.
- The `cloud` engine talks to any OpenAI-compatible transcription API. The
  default preset is OpenRouter's `whisper-large-v3-turbo`, which routes to the
  cheapest serving provider automatically; a specific provider can be pinned
  with `voice_input.cloud.provider_slug`. Presets for Groq, OpenAI and a custom
  endpoint are included, and every request parameter is overridable.
- The `local` engine runs downloaded models on-device with `whisper.cpp` or
  `sherpa-onnx` (Whisper and Parakeet models, including the int8 Parakeet
  variants). Models are downloaded from the microphone menu, with progress, into
  the app data directory or a directory chosen in settings.
- Long dictations are split into bounded chunks (default 120s) that are
  transcribed and inserted incrementally, so a slow or failing request cannot
  interrupt the rest of the recording. The tail of the transcript is sent as a
  prompt to keep terminology and flow coherent across chunk boundaries, with the
  context size controlled by `voice_input.context_chars`.
- Spoken audio is never lost on failure: the untranscribed remainder is kept in
  memory and saved under `<data dir>/voice-recordings`, and "Retry Last
  Dictation" in the microphone menu resumes from where it stopped.

## 1.31.0-my.5

### Fixed

- Windows installers are attached to their GitHub release again. The
  `publish-windows` and prune jobs no longer rely on a repository checkout to
  resolve the repository, so `gh release upload` works; earlier tagged builds
  published the Linux asset but silently failed to attach `my-zed-x86_64.exe`.
  No application code changed compared to `1.31.0-my.4`.

## 1.31.0-my.4

### Windows Explorer integration

- The installer now registers its own file associations, ProgIDs, context-menu
  keys and COM class instead of reusing the identifiers of upstream Zed. Before
  this, installing my zed could take over the "Open with Zed" Explorer entry
  and file-type associations owned by a regular Zed install, and the right-click
  entry could show the wrong title.

## 1.31.0-my.3

### Windows release build fixes

- Tag-triggered releases are now built on the `stable` channel, so the Windows
  installer reports itself as `my zed` (not `my zed Dev`) and behaves like a
  release build. Previously every tagged build hardcoded `dev`.
- On Windows the app now stores its data in `%LOCALAPPDATA%\my-zed` instead of
  `%LOCALAPPDATA%\Zed`, so it no longer shares databases and chats with a
  regular Zed install. Settings, keymaps and themes still live in the shared
  `%APPDATA%\Zed` config directory, and API keys are read from the Windows
  Credential Manager that regular Zed already populated, so both carry over
  without any extra steps.
- "Import from Zed" now works on Windows: it imports chats, workspace state and
  assets from `%LOCALAPPDATA%\Zed` into the fork's own data directory.

### Smaller downloads and faster builds

- Linux releases no longer build or bundle the musl `remote-server` binary, so
  the Linux build is faster and the download smaller. Self-hosted remote
  servers can be fetched separately when needed.

## 1.31.0-my.2

### Memory fixes

- The built-in browser block no longer launches `npx @playwright/mcp` and a
  headless Chromium for every project that opens. It now starts only when a
  chat actually asks for browser access, which removes that whole process tree
  (a large share of the app's resident memory) from the baseline footprint.
- Extension WebAssembly linear memory is now bounded per extension (512 MiB
  hard cap) and periodically reclaimed. Memory that a wasm linear memory has
  grown into is never returned to the OS on its own, so any extension that had
  grown past 96 MiB is now recycled at most every 30 minutes, giving its pages
  back instead of letting the footprint creep up until a restart.

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
  tree-sitter parse trees each, and long sessions used to grow through wasm
  linear memory (extensions) that only a restart reclaimed.
- Wasm linear memory is now reclaimed automatically instead of only on restart.
  Each wasm extension's linear memory is tracked by a `ResourceLimiter` and
  capped at 512 MB, and a background task reloads any extension whose linear
  memory crosses 96 MB (growing at least 32 MB since its last recycle), handing
  the old pages back to the process. The browser context server is also
  on-demand now, so no Playwright or Chromium process runs until a chat turns
  browser access on.
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
