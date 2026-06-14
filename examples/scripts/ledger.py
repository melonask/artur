#!/usr/bin/env python3
"""SQLite-backed demo ledger for Artur's ready-to-use paid job service."""

from __future__ import annotations

import argparse
import json
import os
import random
import sqlite3
import string
import sys
import time
from decimal import Decimal, InvalidOperation


CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"


def emit(payload: dict, code: int = 0) -> None:
    print(json.dumps(payload, separators=(",", ":")))
    raise SystemExit(code)


def request() -> dict:
    return json.loads(sys.stdin.read() or "{}")


def body(req: dict) -> dict:
    value = req.get("body_json")
    return value if isinstance(value, dict) else {}


def db_path() -> str:
    return os.environ.get("ARTUR_LEDGER_DB", "data/artur/service.sqlite3")


def connect() -> sqlite3.Connection:
    path = db_path()
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    conn = sqlite3.connect(path)
    conn.row_factory = sqlite3.Row
    init(conn)
    return conn


def init(conn: sqlite3.Connection) -> None:
    conn.executescript(
        """
        CREATE TABLE IF NOT EXISTS spaces (
          sid TEXT PRIMARY KEY,
          created_at INTEGER NOT NULL,
          metadata_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS balances (
          sid TEXT NOT NULL,
          chain TEXT NOT NULL,
          token TEXT NOT NULL,
          amount TEXT NOT NULL,
          updated_at INTEGER NOT NULL,
          PRIMARY KEY (sid, chain, token)
        );
        CREATE TABLE IF NOT EXISTS ledger_events (
          id TEXT PRIMARY KEY,
          sid TEXT NOT NULL,
          event_type TEXT NOT NULL,
          chain TEXT NOT NULL,
          token TEXT NOT NULL,
          usd_amount TEXT NOT NULL,
          token_amount TEXT NOT NULL,
          token_rate_usd TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          payload_json TEXT NOT NULL
        );
        """
    )
    conn.commit()


def ulid() -> str:
    timestamp_ms = int(time.time() * 1000)
    chars = []
    for shift in range(45, -1, -5):
        chars.append(CROCKFORD[(timestamp_ms >> shift) & 31])
    chars.extend(random.choice(CROCKFORD) for _ in range(16))
    return "".join(chars)


def decimal_field(source: dict, name: str) -> Decimal:
    try:
        value = Decimal(str(source[name]))
    except (KeyError, InvalidOperation):
        emit({"ok": False, "error": f"{name} must be a decimal string"}, 2)
        raise AssertionError("unreachable")
    if value <= 0:
        emit({"ok": False, "error": f"{name} must be greater than zero"}, 2)
    return value


def text_field(source: dict, name: str) -> str:
    value = str(source.get(name, "")).strip()
    if not value:
        emit({"ok": False, "error": f"{name} is required"}, 2)
    return value


def create_space(_: argparse.Namespace) -> None:
    req = request()
    sid = ulid()
    now = int(time.time())
    with connect() as conn:
        conn.execute(
            "INSERT INTO spaces (sid, created_at, metadata_json) VALUES (?1, ?2, ?3)",
            (sid, now, json.dumps(body(req), separators=(",", ":"))),
        )
    emit({"ok": True, "sid": sid, "created_at": now})


def top_up(_: argparse.Namespace) -> None:
    req = request()
    sid = text_field(req.get("params", {}), "sid")
    payload = body(req)
    chain = text_field(payload, "chain")
    token = text_field(payload, "token")
    usd_amount = decimal_field(payload, "usd_amount")
    token_rate_usd = decimal_field(payload, "token_rate_usd")
    token_amount = usd_amount / token_rate_usd
    now = int(time.time())
    event_id = ulid()
    with connect() as conn:
        existing = conn.execute(
            "SELECT amount FROM balances WHERE sid = ?1 AND chain = ?2 AND token = ?3",
            (sid, chain, token),
        ).fetchone()
        current = Decimal(existing["amount"]) if existing else Decimal("0")
        updated = current + token_amount
        conn.execute(
            "INSERT INTO balances (sid, chain, token, amount, updated_at) VALUES (?1, ?2, ?3, ?4, ?5) "
            "ON CONFLICT(sid, chain, token) DO UPDATE SET amount = excluded.amount, updated_at = excluded.updated_at",
            (sid, chain, token, str(updated), now),
        )
        conn.execute(
            "INSERT INTO ledger_events VALUES (?1, ?2, 'topup', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            (event_id, sid, chain, token, str(usd_amount), str(token_amount), str(token_rate_usd), now, json.dumps(payload, separators=(",", ":"))),
        )
    emit({"ok": True, "sid": sid, "chain": chain, "token": token, "usd_amount": str(usd_amount), "token_amount": str(token_amount), "balance": str(updated)})


def check_payment(_: argparse.Namespace) -> None:
    req = request()
    sid = text_field(req.get("params", {}), "sid")
    payload = body(req)
    chain = text_field(payload, "chain")
    token = text_field(payload, "token")
    cost_usd = decimal_field(payload, "cost_usd")
    token_rate_usd = decimal_field(payload, "token_rate_usd")
    token_amount = cost_usd / token_rate_usd
    x_payment = req.get("headers", {}).get("x-payment", "").strip()
    now = int(time.time())
    event_id = ulid()

    with connect() as conn:
        if x_payment:
            try:
                payment = json.loads(x_payment)
                paid_usd = Decimal(str(payment.get("amount_usd", "0")))
            except (json.JSONDecodeError, InvalidOperation):
                emit({"ok": True, "paid": False, "reason": "invalid x-payment header"})
                raise AssertionError("unreachable")
            if payment.get("scheme") == "x402-native" and paid_usd >= cost_usd:
                conn.execute(
                    "INSERT INTO ledger_events VALUES (?1, ?2, 'x402_job_payment', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    (event_id, sid, chain, token, str(cost_usd), str(token_amount), str(token_rate_usd), now, x_payment),
                )
                emit({"ok": True, "paid": True, "source": "x402", "event_id": event_id})
            emit({"ok": True, "paid": False, "reason": "insufficient x402 payment", "required_usd": str(cost_usd)})

        row = conn.execute(
            "SELECT amount FROM balances WHERE sid = ?1 AND chain = ?2 AND token = ?3",
            (sid, chain, token),
        ).fetchone()
        current = Decimal(row["amount"]) if row else Decimal("0")
        if current < token_amount:
            emit({"ok": True, "paid": False, "reason": "insufficient balance", "required_usd": str(cost_usd), "required_token_amount": str(token_amount), "balance": str(current)})
        updated = current - token_amount
        conn.execute(
            "UPDATE balances SET amount = ?1, updated_at = ?2 WHERE sid = ?3 AND chain = ?4 AND token = ?5",
            (str(updated), now, sid, chain, token),
        )
        conn.execute(
            "INSERT INTO ledger_events VALUES (?1, ?2, 'balance_job_payment', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            (event_id, sid, chain, token, str(cost_usd), str(token_amount), str(token_rate_usd), now, json.dumps(payload, separators=(",", ":"))),
        )
    emit({"ok": True, "paid": True, "source": "balance", "event_id": event_id, "remaining_balance": str(updated)})


def run_job(_: argparse.Namespace) -> None:
    req = request()
    payload = body(req)
    emit({"ok": True, "result_ulid": ulid(), "space": req.get("params", {}).get("sid"), "input": payload})


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("create-space").set_defaults(func=create_space)
    sub.add_parser("top-up").set_defaults(func=top_up)
    sub.add_parser("check-payment").set_defaults(func=check_payment)
    sub.add_parser("run-job").set_defaults(func=run_job)
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
