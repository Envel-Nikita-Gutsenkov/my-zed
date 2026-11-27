# Task

Maintain the personal Zed fork `my-zed` (`/home/nikita/zed-src`). Shipped
`v1.31.0-my.13` (token-stats undercount + queued-message drain). Open task:
diagnose and fix the editor holding ~2 GB RSS + ~2.5 GB swap, and ship the fix
as `v1.31.0-my.14`.

## Plan

- [x] Fix token-usage undercount (per-request `request_seq`) — my.13.
- [x] Fix queued-message stranding (`resume_if_paused`) — my.13.
- [x] Diagnose the RSS/swap growth on the running process (glibc `malloc_info`,
      chunk walk, heap text scan).
- [x] Identify the dominant retainer: `ActionLog` tracked every file the agent
      **read** (strong buffer + LSP + full `BufferDiff`) for the thread lifetime.
- [x] Fix: add `ActionLog::record_file_read` (read time only, no tracking) and use
      it for pure reads (`read_file_tool`, ACP `read_text_file`). Edits still use
      `buffer_read`, which creates the diff with the correct pre-edit base.
- [x] Regression test `test_record_file_read_does_not_track_buffer`; full
      `cargo test -p action_log` = 31 passing; clippy clean (one pre-existing
      `unnecessary_sort_by` in `agent`).
- [ ] Commit + push + tag `v1.31.0-my.14`, poll release, install into
      `~/.local/my-zed.app`.
- [ ] Verify `v1.31.0-my.13` release run 46055104251 finished before/alongside.

## Findings

### Memory (my.14)

- Running process snapshot: Rss 2087–2300 MiB, Swap 2515–2574 MiB, VmSize 64 GB.
- glibc is the allocator (no mimalloc). `malloc_info` shows only 2 arenas:
  arena0 ~2.98 GB, arena1 ~1.79 GB; `uordblks` 4.1–4.4 GB, `fordblks` 17 MiB
  ⇒ **genuinely live memory, not fragmentation**. `malloc_trim` won't help.
- Heap chunk walk (main `[heap]`, 2.9 GB, ~9M chunks): 6.9M chunks of 64–127 B
  (pointer-bearing nodes), ~851k of 32–63 B (Arc-like `{strong, ptr}`), ~650k of
  2–4 KB (mostly-zero integer arrays), a few cranelift-compiled code blobs
  (cranelift 0.135.9).
- Heap text scan: thousands of repeats of codebase source lines, `/home/nikita/
  .openclaw/workspace/ZedPRO/...` paths (399 unique `.rs` paths), env vars
  (PATH/KDE/`DEBUGINFOD_URLS`/cgroup) repeated ~2500× ⇒ many file contents +
  retained process/config envs.
- **Root cause**: `crates/action_log/src/action_log.rs` `tracked_buffers:
  BTreeMap<Entity<Buffer>, TrackedBuffer>` — strong `Entity` key, plus
  `TrackedBuffer.buffer` strong clone, plus a full `BufferDiff` (whole-file
  syntax tree) and `register_buffer_with_language_servers`. `buffer_read` is
  called for **every** `read_file_tool` read. `changed_buffers` filters
  `has_edits`, so read-only files never appear in review yet were retained
  forever ⇒ GBs over a 12 h session.
- `buffer_read` is also used by edit paths *before* applying edits (edit tool
  `edit_session.rs:758`, ACP `write_text_file`), so the diff base is still
  captured correctly after the fix.
- Agent DB is tiny (db 12 MB + threads.db 27 MB) ⇒ not conversation volume.
  Terminal scrollback, profiler, hang telemetry are all bounded.
- The user's `GLIBC_TUNABLES` is malformed — `glibc.malloc.mmap_threshold:
  1048576:glibc.malloc.trim_threshold:1048576` (missing `=`); ld.so logs
  "invalid GLIBC_TUNABLES … ignored". If they want those tunables, use `=`.
- fd pile (1566 fds) = sysinfo caching one `/proc/<pid>/stat` file per process
  per persistent `System`; `resource_monitor.rs` uses `ProcessesToUpdate::All`
  in a per-workspace `System` (≈4×358). Bounded, kernel-only, not the RSS.

### Prior (my.13)

- Token stats keyed `{thread}:{prompt}` collapsed a multi-request turn via `MAX`;
  now `{thread}:{prompt}:{request_seq}`.
- Queue stayed `Paused` after clean `EndTurn` if an earlier error/stop paused it;
  `resume_if_paused` re-arms on `EndTurn`.
- Release workflow concurrency `release-<ref>`, `cancel-in-progress: false`
  ⇒ bump version rather than retag while a run is in flight.
- `smol::process::windows::CommandExt` needed for Windows `creation_flags`.

## Structure

- `crates/action_log/src/action_log.rs` — tracked buffers, `buffer_read`,
  `record_file_read`, `changed_buffers`, `stale_buffers`.
- `crates/agent/src/tools/read_file_tool.rs` — calls `record_file_read` now.
- `crates/acp_thread/src/acp_thread.rs` — `read_text_file` uses
  `record_file_read`; `write_text_file` keeps `buffer_read`.
- `crates/agent/src/tools/edit_session.rs` — keeps `buffer_read` (edit base).
- `crates/zed/src/zed/resource_monitor.rs`, `reliability.rs` — memory logging.
- `CHANGELOG.md` — `## 1.31.0-my.14` section added.

## Next

- [ ] Tag `v1.31.0-my.14` (commit `77bb257`) is pushed; release run 46062902579 is
      building. Poll the direct asset URL (HEAD 200 ⇒ published):
      `https://github.com/Envel-Nikita-Gutsenkov/my-zed/releases/download/v1.31.0-my.14/zed-linux-x86_64.tar.gz`
      (no API rate limit). Windows build makes it a draft (~80 min after 20:20 UTC).
- [ ] Install: download `zed-linux-x86_64.tar.gz`, extract, `rsync -a --delete zed.app/ ~/.local/my-zed.app/`,
      verify `~/.local/my-zed.app/bin/zed --version` shows `77bb257`.
      NOTE: `main` is one commit ahead (`263ee83`, test-only); binary still 77bb257.
- [ ] Ask user to restart Zed and repeat the same auto-continue workload; compare
      `memory usage: heap in-use` in `~/.local/share/my-zed/zed/logs/Zed.log`
      (old build grew to ~4.4 GB heap in-use within one session).
- [ ] Follow-up (not shipped): `AcpThread::shared_buffers: HashMap<Entity<Buffer>, BufferSnapshot>`
      (`crates/acp_thread/src/acp_thread.rs:3788`) is an unbounded strong-retention
      cache, but only external-agent (ACP) fs handlers populate it, NOT the
      built-in agent, so it does not explain the user's heap.
- [ ] Follow-up candidate: reviewed/kept tracked buffers (edited files) are still
      retained forever; if edited-file count is large, prune entries with no
      unreviewed edits after review.
