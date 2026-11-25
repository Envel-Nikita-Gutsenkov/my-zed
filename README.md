> [!IMPORTANT]
> Remove this line to confirm you've reviewed this PR before submitting.

# my zed

> [!WARNING]
> **Personal project.** This is my personal, opinionated fork of
> [Zed](https://github.com/zed-industries/zed). It is not affiliated with or
> endorsed by Zed Industries. It is published as-is, without support,
> guarantees or a stable API, and it may change or be abandoned at any time.
> Use upstream Zed if you want a supported editor.

A lightweight, self-contained build of Zed focused on aggressive token saving
for agent requests, no telemetry, no mandatory cloud account, and comfortable
long-running chats. It installs alongside an existing Zed and never
self-updates to upstream.

## Features

- **Token savings.** Configurable per-request context trimming on top of
  auto-compaction: tool-output caps, old-turn truncation, ANSI stripping,
  de-duplication, image dropping, and reasoning stripping with a separate
  "very old" threshold.
- **No tracking.** Telemetry, metrics and auto-update are disabled; sign-in is
  hidden and the build cannot replace itself with upstream Zed.
- **Long-running chats.** Age/count based chat maintenance, message queue that
  survives restarts, per-chat Auto mode, and a token usage statistics window
  whose totals include cached tokens. With Auto mode on, routine tool
  confirmation prompts are answered automatically, so the agent keeps working
  instead of stopping for a click.
- **Task log.** A per-chat Task Log toggle keeps a running `.agent/TODO.md` in
  the project (plan, key findings, file map, in English, roughly 1000 lines) so
  long tasks keep their thread across context compaction; `.agent/` is added to
  `.gitignore` automatically.
- **Lower memory use.** Allocator tuning and periodic heap trimming are on by
  default so freed pages return to the OS instead of making the process only ever
  grow; the `mimalloc` cargo feature remains available as an opt-in allocator.
  See [Memory](#memory).
- **Resource monitoring.** CPU and memory readout in the status bar.

See [CHANGELOG.md](./CHANGELOG.md) for the full list of differences from
upstream Zed.

## Installation

Only the **three most recent** releases are kept on GitHub; older releases and
their tags are removed automatically. Download the installer for your platform
from the [Releases](../../releases) page:

- **Windows** — `ZedEditorUserSetup-x64-*.exe`
- **Linux** — `zed-linux-*.tar.gz` (extract and run `bin/zed`)

Expect the app to be called `my zed` and installed next to Zed. Existing Zed
settings, themes, keymaps and providers are picked up automatically; working
data (databases, logs, caches) is kept in a separate directory so the two can
run side by side.

## Memory

The default build uses the system allocator (glibc `malloc` on Linux) with two
memory-oriented tweaks that are on by default:

- **Allocator tuning (Linux/glibc).** At startup the app raises glibc's
  `M_MMAP_THRESHOLD`/`M_TRIM_THRESHOLD` handling so that large transient buffers
  (decoded images, rendering scratch space, diffs) are served by `mmap` and
  handed back to the kernel as soon as they are freed instead of inflating the
  heap. Without this, resident memory ratchets up with every peak and never comes
  back down, which looks like a leak even when nothing is retained. Set
  `MALLOC_ARENA_MAX` yourself if you want to trade allocator parallelism for a
  smaller footprint.
- **Periodic heap trim.** `resources.trim_interval_secs` defaults to `60`.
  Every interval the app asks the allocator to return free pages to the OS
  (`malloc_trim(0)` with glibc, `mi_collect(true)` with mimalloc). The value is
  re-read each cycle, so it can be changed without a restart, and `0` disables it.

The remembered memory log line (`memory usage: resident ... MiB`) now also
reports how the glibc heap splits into live allocations, free-but-retained
space, and `mmap`, so it is possible to tell real growth from fragmentation.

What the status-bar number actually measures, and what trimming can and cannot
do:

- **It is the whole process tree.** The status-bar memory reading is the summed
  resident memory of the editor **and every child process it spawned** — language
  servers, terminals, the extension host, and the browser. On this machine a
  reading of ~2.7 GB was mostly a headless Chromium (~600 MB across its helper
  processes) plus Playwright's Node processes, on top of ~1 GB for the editor.
  Only the editor process is reported in the `memory usage` log line.
- **Trimming only returns free pages.** Both allocator tuning and the periodic
  trim act on memory the program has already released. If the log shows live
  heap (`heap in-use`) climbing while retained heap stays near zero, the growth
  is real, live allocations and no trim setting will shrink it — it has to be
  traced with a profiler and fixed at the allocation site.

The browser tools are backed by an isolated, in-memory Chromium. Its lifetime is
tied to the per-chat browser toggle: turning browser access off shuts the browser
server down (killing Playwright and its Chromium process tree) instead of leaving
a headless Chromium resident until the app exits.

An alternative allocator is still available: build with `--features mimalloc` to
use Microsoft's mimalloc. It returns freed pages more eagerly than glibc, which
helps long-running sessions, but it is not the default: measured on this fork it
used more resident memory than glibc unless also tuned:

| Allocator | Idle RSS (fresh instance) |
| --- | --- |
| glibc (default) | ~262 MB |
| mimalloc (stock) | ~427 MB |
| mimalloc + `MIMALLOC_ARENA_EAGER_COMMIT=0 MIMALLOC_PURGE_DELAY=0` | ~279 MB |

If you enable it, also set those two environment variables (for example in the
launcher) to avoid the idle regression. Add `MIMALLOC_SHOW_STATS=1` to print
allocator statistics on exit.

## Building from source

Rust and the platform toolchain are required.

```sh
# Linux
./script/linux          # installs system dependencies
cargo build --profile release-lite -p zed

# Windows (PowerShell): install the Visual Studio C++ build tools, LLVM and
# Inno Setup, then:
cargo build --profile release-lite -p zed
```

To reproduce a release artifact locally:

```sh
./script/bundle-linux                     # Linux tarball
./script/bundle-windows.ps1 -Architecture x86_64   # Windows installer
```

## Licensing

Zed's source code is licensed primarily under GPL-3.0-or-later, with
Apache-2.0 components where marked. This fork keeps the same licensing.
See [LICENSE-GPL](./LICENSE-GPL), [LICENSE-APACHE](./LICENSE-APACHE) and
upstream's [licensing notes](https://github.com/zed-industries/zed#licensing).
