#!/usr/bin/env python3
"""Demo application script for POST /v1/space.

This is deliberately outside Artur. Replace it with your own app code that can
verify a challenge, allocate resources, call Rust/Python/Node tools, and persist
results in whatever database you use.
"""
import json
import os
import random
import sqlite3
import string
import subprocess
import sys
import time

body = sys.stdin.read()
payload = json.loads(body or "{}")

def db_path() -> str:
    return os.environ.get("ARTUR_SPACE_DB") or "artur-example-space.sqlite3"

def init_db(conn: sqlite3.Connection) -> None:
    conn.execute(
        "CREATE TABLE IF NOT EXISTS spaces (sid TEXT PRIMARY KEY, created_at INTEGER NOT NULL, payload TEXT NOT NULL)"
    )
    conn.commit()

def new_sid() -> str:
    return "".join(random.choice(string.digits) for _ in range(24))

def verify_with_external_cli() -> dict:
    """Optionally verify with `challenge verify` if you require it.

    The exact `challenge` CLI JSON contract may differ by crate/version. This
    example keeps verification app-owned: Artur only starts the process.
    """
    if os.environ.get("ARTUR_SKIP_CHALLENGE_VERIFY") == "1":
        return {"verified": True, "skipped": True}
    if not payload.get("challenge") or not payload.get("solution"):
        return {"verified": False, "error": "missing challenge or solution"}

    secret = os.environ.get("ARTUR_CHALLENGE_HMAC_SECRET", "")
    key_secret = os.environ.get("ARTUR_CHALLENGE_HMAC_KEY_SECRET", "")
    if not secret or not key_secret:
        return {"verified": False, "error": "missing challenge secrets"}

    completed = subprocess.run(
        [
            "challenge",
            "verify",
            "--challenge",
            json.dumps(payload["challenge"], separators=(",", ":")),
            "--solution",
            json.dumps(payload["solution"], separators=(",", ":")),
            "--secret",
            secret,
            "--key-secret",
            key_secret,
        ],
        text=True,
        capture_output=True,
        timeout=10,
    )
    return {
        "verified": completed.returncode == 0,
        "status_code": completed.returncode,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
    }

verification = verify_with_external_cli()
if not verification.get("verified"):
    print(json.dumps({"ok": False, "verification": verification}, separators=(",", ":")))
    sys.exit(2)

created_at = int(time.time())
sid = new_sid()
conn = sqlite3.connect(db_path())
init_db(conn)
conn.execute(
    "INSERT INTO spaces (sid, created_at, payload) VALUES (?, ?, ?)",
    (sid, created_at, json.dumps(payload, separators=(",", ":"))),
)
conn.commit()

print(json.dumps({
    "ok": True,
    "sid": sid,
    "created_at": created_at,
    "verification": verification,
    "wallets": [],
    "deposits": [],
    "expenses": [],
}, separators=(",", ":")))
