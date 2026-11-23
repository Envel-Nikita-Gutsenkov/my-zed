#!/bin/sh
# Launcher for the separate "my zed" build.
#
# Config is shared with the existing Zed install: settings, themes, keymaps and
# the `Command Code` provider config all come from ~/.config/zed.
#
# Only the runtime data (databases, chats, logs, caches) is kept in a dedicated
# folder so both apps can run side by side without fighting over the same
# SQLite databases. Zed appends /zed to XDG_DATA_HOME, so the actual data lands
# in ~/.local/share/my-zed/zed. Projects and chats were copied there once.
#
# This build is on the dev release channel, which otherwise stores credentials
# in a local file; force the system keyring so the existing API keys are used.
export XDG_DATA_HOME="$HOME/.local/share/my-zed"
export ZED_DEVELOPMENT_USE_KEYCHAIN=1
exec "$HOME/.local/my-zed.app/bin/my-zed" "$@"
