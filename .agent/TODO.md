# my-zed work TODO

Goal: a personal, lightweight Zed build with strong token-saving defaults, no
mandatory cloud account / telemetry, resource limits, and a persisted message
queue. Ships as a separate app ("my zed") that reuses the existing Zed config.

## Done
- [x] `tool_output_token_limit` setting (default 30000), applied per tool result.
- [x] `token_savings` settings block with many independent toggles.
- [x] Token-saving passes in `agent/src/thread.rs` (truncate tool output, drop
      old ephemeral tool output, dedupe repeats, strip ANSI, drop old images,
      keep latest read per path, omit old tool-use inputs, total token budget).
- [x] Defaults in `assets/settings/default.json` (auto_compact 100k, thinking
      collapsed, telemetry off, auto_update off, sign-in hidden).
- [x] Auto-continue mode ("ALL DONE") + composer toggle.
- [x] Message metadata: per-turn timestamp in thread controls.
- [x] Queued messages: reorder with arrow buttons.
- [x] Queued messages: numbered badge instead of plain dot.
- [x] Persist queued messages in the SQLite KVP store so crash/restart does not
      lose them.
- [x] Auto-shrink / thread maintenance: age-based and count-based pruning of
      stored threads plus dropping the oldest messages of oversized threads.
      All limits are settings (`agent.thread_maintenance`).
- [x] `resources` settings block + resource monitor status-bar item with a mini
      CPU graph and memory readout, sampled on a configurable timer.
- [x] Configurable log file size cap (`resources.log_file_size_limit_mb`).
- [x] Fast, low-memory build profiles (`release-lite`, `build-fast`).
- [x] Rename to "my zed" (display names, app ids, menus, bundle script).
- [x] Build verified with `cargo check -p settings_content -p zlog -p agent_ui
      -p agent -p zed`.
- [x] Token usage statistics window (input/output/total, "saved by trimming",
      cache read/write) with time-range filters and a per-bucket bar chart.
      Events persist in `token_usage_events` with a 90-day retention prune.
- [x] Open the stats window from a toolbar button and from the command palette
      (`agent::OpenTokenUsageStats`).
- [x] Local `dev` build version string includes the short commit hash.
- [x] Windows installer GitHub Actions workflow (manual `workflow_dispatch`
      only). Not buildable from Linux; needs Actions enabled on the repo.
- [x] Browser context server is now **on demand** (`project/src/context_server_store.rs`):
      the built-in `browser` server (`npx @playwright/mcp`) is no longer started
      at project open, so no Playwright/Chromium process exists until a chat
      enables browser access. `request_on_demand_server` /
      `release_on_demand_server` drive it; tests no longer spawn `npx`.
- [x] Wasm linear-memory reclamation (`extension_host`): a `ResourceLimiter`
      records each extension's linear-memory high-water mark and caps it at
      512 MiB; a background task (every 2 min) recycles any extension past
      96 MiB, at most once per 30 min cooldown, via the existing
      modified-extension reload path, freeing linear memory that otherwise
      only grows until restart. Unit test
      `test_wasm_memory_limiter_records_and_caps_growth`.

## Active tasks
- [x] CHANGELOG.md.
- [x] Local commit + push to origin/main.
- [x] Pushed both fixes; the build/test is left to GitHub CI (ci.yml runs on
      push to main: fmt, `cargo check -p zed`, and unit tests).
- [ ] Release installer build is triggered manually via the Release workflow
      (`workflow_dispatch`); `gh` is not authenticated here so it must be run
      by the user (or after `gh auth login`).

## Install details
- Binary: `~/.local/my-zed.app/bin/my-zed` (stripped `release-lite` build).
- Launcher: `~/.local/bin/my-zed` sets `XDG_DATA_HOME=~/.local/share/my-zed`
  so runtime data/chats/logs are separate under `~/.local/share/my-zed/zed`,
  while config (settings, themes, providers) is shared from `~/.config/zed` and
  API keys come from the system keyring (label `zed-github-account`, keyed by
  provider URL).
- Desktop entry: `~/.local/share/applications/dev.myzed.MyZed.desktop`
  (`Name=my zed`, `StartupWMClass=dev.myzed.MyZed-Dev`).
- Icon: `~/.local/my-zed.app/share/icons/my-zed.png`.
- Build command: `cargo build --profile release-lite -p zed --bin zed -j 6`
  (~20 min from scratch on this machine).
- Projects and chats were copied once from `~/.local/share/zed` into
  `~/.local/share/my-zed/zed` via `.agent/transfer-data.py` (SQLite backup API;
  `0-stable` -> `0-dev` because the build is on the `dev` channel).
- The build is on the `dev` release channel, so the launcher sets
  `ZED_DEVELOPMENT_USE_KEYCHAIN=1` to read API keys from the system keyring
  instead of the dev credentials file.

## Notes
- Existing Zed config at `~/.config/zed`. New app reuses it (change app id,
  not config dir) so API keys/settings carry over.
- `APP_NAME` in `crates/paths/src/paths.rs` stays "Zed" on purpose so the new
  build shares the existing config directory.
- Resource monitor settings live at `resources.*`; thread maintenance at
  `agent.thread_maintenance.*`; token savings at `agent.token_savings.*`.
