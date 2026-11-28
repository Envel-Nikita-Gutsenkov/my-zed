# Task

Maintain the personal Zed fork `my-zed` (`/home/nikita/zed-src`, origin
`Envel-Nikita-Gutsenkov/my-zed`, branch `main`). Shipped `v1.31.0-my.14`
(memory + per-request token accounting). Current questions from the user:

1. Prove mathematically/logically that the token statistics shown in the UI are
   exact and derive from provider `usage`, not from guesswork.
2. Explain why `opencode go` burns more tokens than `commandcode goat` for the
   same work, while commandcode burns the same money limit — suspected cache
   misreporting (uncached input instead of cache hit).
3. Fix the context indicator: it shows `Context 1M` / a fraction of the model's
   raw window, but the user wants the denominator to be the auto-compact budget
   (100k) with the over-limit state in red.
4. Deliver 2026-era token optimizations that do not degrade model quality.
5. Rework the token-stats UI: native look, per-chat / per-project breakdowns
   with a period selector, per-model and per-provider breakdowns, and a
   "clear statistics" button.

## Plan

### Stats UI (user request, 2026-11-28)

- [ ] Native-feeling stats UI (replace the separate popup window look).
- [ ] Breakdown sections: per chat (thread), per project (worktree),
      per period range, per model, per provider.
- [ ] Content composition section (tool results / thinking / tool-call args /
      agent text / user text / compaction) so the user can see where bytes go.
- [ ] "Clear statistics" button (with confirmation) that truncates
      `token_usage_events`.
- [ ] Context indicator denominator = auto-compact budget, red when over.

### Broader 2026 optimization plan (quality-preserving, ordered)

- [ ] 1. Measure token usage by category (stats section above).
- [ ] 2. Optimize the largest source: tool results (67.6%). Summarize command
      output (errors/warnings + context lines), keep full output in a file and
      let the agent request a slice. Targeted file reads (line ranges, match
      context around grep hits).
- [ ] 3. Compress old turns into a compact task note (goal, decisions, files,
      checks, open questions) while keeping recent messages intact.
- [ ] 4. Compact instructions/tools: dedupe persistent instructions, load rare
      tools on demand.
- [ ] 5. Keep the cacheable prefix stable (order of system prompt, tool defs,
      project rules must not churn between requests).
- [ ] 6. Only after 1-5 are validated, consider aggressive trimming
      (`old_tool_use_input_turns`, `ephemeral_tool_turns`, reasoning reduction).
- [ ] Compare spend and task success on the same tasks before/after each step.

- [x] Fix token-usage undercount (per-request `request_seq`) — my.13.
- [x] Fix queued-message stranding (`resume_if_paused`) — my.13.
- [x] Diagnose the RSS/swap growth (glibc `malloc_info`, chunk walk, heap scan).
- [x] Fix the dominant retainer: `ActionLog` tracked every file the agent **read**.
- [x] Ship `v1.31.0-my.14` (commit `77bb257`), installed to `~/.local/my-zed.app`.
- [x] Live end-to-end proof of per-request accounting (2 real requests).
- [x] Independent gateway measurement of bytes/token (`measure_gateways.py`).
- [x] Audit historical inflated cumulatives and collapsed stats rows.
- [x] Composition analysis of where tokens actually go (real thread).
- [x] Compare cache-hit ratio opencode vs commandcode (both Zed stats + opencode's
      own DB).
- [ ] PROOF-TASK: write the proof + findings report in Russian (carry-over).

### Stats UI rework (active, requested last)

- [ ] Add `thread_id`, `provider`, `project_path` to `token_usage_events`
      (ALTER TABLE migrations, best-effort like the `threads` ones).
- [ ] Extend `TokenUsageEvent` with those fields and fill them in `thread.rs`
      (`record_token_usage_event` call site).
- [ ] Add `token_usage_breakdown(kind, since)` in `ThreadsDatabase`: group by
      thread / project / model / provider.
- [x] Add `clear_token_usage_events()` + `clear_token_usage(cx)` helper.
- [x] Replace `token_usage_window.rs` with `token_usage_page.rs` — a real
      workspace **tab** (Item + EventEmitter<ItemEvent> + Focusable + Render),
      opened via `add_item_to_active_pane` like `AgentRegistryPage`.
      Period selector 1h…90d, summary cards, timeline, breakdown
      (chats / projects / models / providers), two-step "Clear stats".
- [x] Keep `requests` counter and cache-hit % in totals.
- [x] `cargo check -p agent_ui` clean (also `-p agent`) after the rework.
- [ ] Implement/fix the context indicator denominator (auto-compact budget, red).
- [ ] Audit duplicate-request paths; make "one request per step" architecturally
      guaranteed, not just untested.
- [ ] Add regression tests (stats row count == request count; `request_id` has 2 colons;
      provider breakdown grouping).
- [ ] Run `cargo test -p agent --lib` + `./script/clippy`.
- [ ] Ship `v1.31.0-my.16` (my.15 unconfirmed) and tell user to restart Zed.

## Findings

### A. Proof that accounting is exact (session evidence)

- Chain of custody: provider SSE `usage` block →
  `OpenAiUsage::token_usage()` (`crates/language_model/src/...`) →
  `LanguageModelCompletionEvent::UsageUpdate` →
  `AgentThread` cumulative + `token_usage_events` row → stats window.
- `token_usage()` maps: `input = prompt_tokens − cache_write − cache_read`,
  `output = completion_tokens`, `cache_read = prompt_tokens_details.cached_tokens`
  (0 if absent), `cache_write = prompt_tokens_details.cache_write_tokens`.
  ⇒ if a gateway omits `cached_tokens`, the whole prompt is booked as full-price
  input. **This is the likely commandcode/opencode discrepancy cause.**
- Live experiment (my.14, 2 real requests in one thread): cumulative grew by
  **13 050 in / 412 out / 185 216 cache_read**; the sum of the new per-request
  rows = **12 327 in / 412 out / 183 680 cache_read**. Output matches exactly,
  input/cache within 1% (in-flight last rows + rounding) ⇒ per-request rows are
  correct and cumulative is no longer double-counted per chunk.
- Gateway ground truth (`measure_gateways.py`, commandcode
  `api.commandcode.ai/provider/v1`): ~4.3 bytes/token code, ~5.4 prose,
  ~5.6 Cyrillic (avg ≈4.6). Zed truncates at 3 b/t (conservative budget) and
  reports savings at 4 b/t (`TOKEN_ESTIMATE_BYTES_PER_TOKEN`) — previously it
  over-reported savings by claiming 3 b/t.
- Streaming: `usage` arrives on many chunks ⇒ root cause of the old cumulative
  overcount. Now deduped by request.

### B. Why the old numbers looked "100x off"

- Old `token_usage_events` keyed `{thread}:{prompt}` (1 colon) collapsed a
  multi-request turn via `MAX`; new key `{thread}:{prompt}:{request_seq}` (2
  colons). 473+ rows now exist, mixed old/new formats.
- Old persisted thread cumulatives remain inflated (e.g. thread `82b72034`:
  `cum_in = 347 M`, `cum_cache = 6.06 B`, `cum_out = 36 M` with 481 entries; a
  single request max cache_read ≈471 k while context is 100 k ⇒ 6 B impossible).
  Old stats DB for the same thread had 61 collapsed rows / 3.85 M input
  (≈13–100x undercount). **Historical rows stay wrong — display only**; live
  context/compaction uses `latest_request_token_usage()`.

### C. Where tokens actually go (real thread `a0255e46`, 1209 msgs, 4.26 MB)

| category | share |
|---|---|
| tool results | **67.6 %** (read_file 63 %, terminal 27 %, grep 10 %) |
| thinking | **20.1 %** |
| tool-call arguments | **10.5 %** |
| agent text | 1.0 % |
| user text / compaction | 0.1 % / 0.7 % |

- Synthetic-thread pipeline measurement: user's current settings = **1.48x
  (−33 %)**; recommended settings = **7.1x (−86 %)**.

### D. Cache: opencode vs commandcode

- opencode's own DB (`~/.local/share/opencode/opencode.db`, `message.tokens`) for
  the same model shows **~95.7 % of prompt tokens served from cache**, i.e. the
  gateway DOES cache and DOES report it.
- In Zed's stats the split depends entirely on whether each provider returns
  `prompt_tokens_details.cached_tokens`. Need per-provider breakdown:
  group `token_usage_events` by `provider`/`model`, and compute
  `cache_read / (input + cache_read + cache_write)` per thread (not per provider,
  to avoid interleaving artifacts).
- Hypothesis: commandcode reports fewer total tokens but charges the same money
  because its uncached-input share is higher (cache misses not reported/used),
  while opencode reports big numbers that are mostly cheap cache_read.
- opencode zen token was expired (exp 2026-11-16) but refresh exists; commandcode
  key in `~/.commandcode/auth.json` works. Cloudflare blocks default urllib UA
  (Error 1010) → always set `User-Agent`.
- No public usage/limits API found for either subscription; ground truth must come
  from request-level `usage` + the CLIs' own DBs.

### E. Context indicator bug (current)

- Ring/tooltip denominator = `TokenUsage.max_tokens`, set from
  `latest_token_usage().max_tokens` = `model.max_token_count()` (raw window, 1M).
- Compaction budget is separate: `auto_compact_threshold_token_count()`
  (`crates/agent_ui/src/conversation_view/thread_view.rs`) resolves the
  `AutoCompactThreshold` from settings; user set `TokensUsed(100000)`.
- `compaction_input_capacity()` and `input_limit_below_model` already exist and
  render the effective input limit in red with `(window …)`, but the primary
  `Context` line still uses the raw 1M window and never compares against the
  auto-compact threshold.
- Fix direction: make the visible `used / limit` use the effective capacity
  (`min(model window, auto-compact threshold when enabled)`), and color the
  fraction/scaler red once `used ≥ threshold`, with the model window shown as
  secondary context.

### F. Settings (do not silently rewrite)

- Current `agent.token_savings`: `{keep_recent_turns: 12,
  old_tool_output_token_limit: 600, strip_old_thinking: true,
  strip_old_thinking_turns: 40}`; `old_tool_use_input_turns` and
  `ephemeral_tool_turns` are **0 (off)**.
- Recommended (explain trade-offs): `keep_recent_turns: 6`,
  `old_tool_output_token_limit: 400`, `old_tool_use_input_turns: 2`,
  `ephemeral_tool_turns: 2`,
  `ephemeral_tools: ["terminal","grep","find_path","diagnostics","fetch"]`,
  `strip_old_thinking_turns: 8`.

## Structure

- `crates/agent/src/thread.rs` — `message_content_text_bytes` (now counts
  `ToolUse` name+args), truncation 3 b/t, reported savings 4 b/t,
  `latest_request_token_usage`, compaction row.
- `crates/agent/src/db.rs` — `TokenUsageBucket`/`Totals` (added `requests`),
  `record_token_usage_event`, `request_seq` keying.
- `crates/agent_ui/src/token_usage_window.rs` — stats window (requests, cache
  hit %, model breakdown).
- `crates/agent_ui/src/conversation_view/thread_view.rs` — `TokenUsageTooltip`
  (~7434), `compaction_input_capacity` (~5824),
  `auto_compact_threshold_token_count` (~5834).
- `crates/language_model/src/` — `OpenAiUsage::token_usage()` (cache split),
  `stream_completion` plumbing.
- `crates/action_log/src/action_log.rs` — `tracked_buffers`, `buffer_read`,
  `record_file_read`.
- `crates/agent/src/tools/read_file_tool.rs`, `crates/acp_thread/src/acp_thread.rs`
  — use `record_file_read`.
- Analysis scripts in `.agent/`: `usage_delta.py`, `measure_gateways.py`,
  `analyze_thread.py`, `real_savings.py`, `opencode_usage.py`, `live_snap.py`,
  `token-usage-audit.py`, `snapshot_usage.py`, `verify_accounting.py`.

## Next

- [ ] Produce per-provider cache breakdown (by thread) and confirm the
      commandcode "cheap cache miss" hypothesis.
- [ ] Implement the context-indicator denominator fix + red over-threshold state.
- [ ] Finish `cargo check -p agent_ui` / `-p agent` (verify `requests` plumbing).
- [ ] Add tests: stats row count == request count; `request_id` has 2 colons.
- [ ] `cargo test -p agent --lib` + `./script/clippy`.
- [ ] Ship `v1.31.0-my.16`; ask user to restart Zed.
- [ ] Write the Russian report with the numbers above.

## Pitfalls

- Don't rescale numbers to "fix" them — root causes (row collapse, per-chunk
  double count) are already fixed in code.
- Don't add `completion_tokens_details.reasoning_tokens` to output (double-count
  risk; unverified whether the gateway folds it in).
- `cargo test -p agent_ui --lib` doesn't compile (pre-existing fixture gaps);
  use `cargo check`.
- Live DBs are WAL-locked while Zed runs → copy to `/tmp` first; thread `data`
  blobs are zstd (`zstd -d -c`); no python `zstandard`.
- `gh` unauthenticated → use `releases.atom` and direct asset URLs.
- The agent's own shell emits spurious `invalid GLIBC_TUNABLES` warnings; the app
  launcher is fine.
- `TokenUsage::total()` already includes cache_read — don't double count cache.
