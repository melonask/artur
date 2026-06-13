#!/usr/bin/env python3
import argparse
import json
import os
import sqlite3
import sys

parser = argparse.ArgumentParser()
parser.add_argument("--sid", required=True)
args = parser.parse_args()

db = os.environ.get("ARTUR_SPACE_DB") or "artur-example-space.sqlite3"
conn = sqlite3.connect(db)
conn.execute("CREATE TABLE IF NOT EXISTS spaces (sid TEXT PRIMARY KEY, created_at INTEGER NOT NULL, payload TEXT NOT NULL)")
row = conn.execute("SELECT sid, created_at, payload FROM spaces WHERE sid = ?", (args.sid,)).fetchone()
if not row:
    print(json.dumps({"ok": False, "error": "not_found", "sid": args.sid}, separators=(",", ":")))
    sys.exit(3)

print(json.dumps({
    "ok": True,
    "sid": row[0],
    "created_at": row[1],
    "wallets": [],
    "deposits": [],
    "expenses": [],
}, separators=(",", ":")))
