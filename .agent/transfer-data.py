#!/usr/bin/env python3
"""Copy Zed's projects and chats into the my-zed data directory.

Uses SQLite's online backup API so the source database can be in use by a
running Zed instance and the copy is still consistent. The build is on the
`dev` channel, so Zed's `0-stable` database is written into my-zed's `0-dev`
slot.
"""

import os
import sqlite3

HOME = os.path.expanduser("~")
SRC = os.path.join(HOME, ".local/share/zed")
DST = os.path.join(HOME, ".local/share/my-zed/zed")

PAIRS = [
    (os.path.join(SRC, "db/0-stable/db.sqlite"), os.path.join(DST, "db/0-dev/db.sqlite")),
    (os.path.join(SRC, "db/0-global/db.sqlite"), os.path.join(DST, "db/0-global/db.sqlite")),
    (os.path.join(SRC, "threads/threads.db"), os.path.join(DST, "threads/threads.db")),
]


def copy_database(source: str, target: str) -> None:
    if not os.path.exists(source):
        print(f"skip (missing source): {source}")
        return
    os.makedirs(os.path.dirname(target), exist_ok=True)
    for suffix in ("", "-wal", "-shm"):
        stale = target + suffix
        if os.path.exists(stale):
            os.remove(stale)

    source_connection = sqlite3.connect(f"file:{source}?mode=ro", uri=True)
    target_connection = sqlite3.connect(target)
    try:
        source_connection.backup(target_connection)
    finally:
        target_connection.close()
        source_connection.close()
    print(f"copied {source} -> {target}")


def main() -> None:
    for source, target in PAIRS:
        copy_database(source, target)


if __name__ == "__main__":
    main()
