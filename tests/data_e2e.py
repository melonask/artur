#!/usr/bin/env python3
"""End-to-end data flow test for the Docker Compose service."""

from __future__ import annotations

import json
import sys
import time
import urllib.error
import urllib.request
from decimal import Decimal


BASE = "http://127.0.0.1:46796"


def request(method: str, path: str, payload: dict | None = None, headers: dict | None = None):
    data = None if payload is None else json.dumps(payload).encode()
    req = urllib.request.Request(
        BASE + path,
        data=data,
        method=method,
        headers={"content-type": "application/json", **(headers or {})},
    )
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            raw = response.read().decode()
            return response.status, dict(response.headers), json.loads(raw or "{}")
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode()
        return exc.code, dict(exc.headers), json.loads(raw or "{}")


def assert_true(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def main() -> None:
    for _ in range(60):
        try:
            status, _, body = request("GET", "/v1/health")
            if status == 200 and body.get("ok"):
                break
        except Exception:
            pass
        time.sleep(1)
    else:
        raise AssertionError("service did not become healthy")

    job_payload = {"chain": "eip155:8453", "token": "USDC", "cost_usd": "0.50", "token_rate_usd": "1.00", "task": "report"}

    status, _, created = request("POST", "/v1/spaces", {"owner": "gha"})
    assert_true(status == 200, f"create space failed: {created}")
    sid = created["json"]["sid"]

    status, headers, unpaid = request("POST", f"/v1/spaces/{sid}/jobs/immediate", job_payload)
    assert_true(status == 402, f"expected 402 for unpaid job, got {status}: {unpaid}")
    assert_true(headers.get("x402-version") == "1", "missing x402-version response header")

    status, _, topup = request("POST", f"/v1/spaces/{sid}/topups", {"chain": "eip155:8453", "token": "USDC", "usd_amount": "2.00", "token_rate_usd": "1.00"})
    assert_true(status == 200 and Decimal(topup["json"]["balance"]) == Decimal("2.00"), f"topup failed: {topup}")

    status, _, paid_job = request("POST", f"/v1/spaces/{sid}/jobs/immediate", job_payload)
    assert_true(status == 200 and paid_job["json"]["space"] == sid, f"paid immediate job failed: {paid_job}")

    status, _, balances = request("GET", f"/v1/spaces/{sid}/balances")
    assert_true(status == 200 and Decimal(balances["balances"][0]["amount"]) == Decimal("1.50"), f"unexpected balances: {balances}")

    status, _, async_start = request("POST", f"/v1/spaces/{sid}/jobs/async", job_payload)
    assert_true(status == 200 and async_start["status"] == "running", f"async job did not start: {async_start}")
    job_id = async_start["job_id"]
    for _ in range(40):
        status, _, job = request("GET", f"/v1/jobs/{job_id}")
        assert_true(status == 200, f"job lookup failed: {job}")
        if job["status"] == "completed":
            assert_true(job["result"]["json"]["space"] == sid, f"unexpected async result: {job}")
            break
        time.sleep(0.1)
    else:
        raise AssertionError("async job did not complete")

    status, _, x402_space = request("POST", "/v1/spaces", {"owner": "x402"})
    sid2 = x402_space["json"]["sid"]
    status, _, x402_job = request(
        "POST",
        f"/v1/spaces/{sid2}/jobs/immediate",
        job_payload,
        {"x-payment": json.dumps({"scheme": "x402-native", "amount_usd": "0.50"}, separators=(",", ":"))},
    )
    assert_true(status == 200 and x402_job["json"]["space"] == sid2, f"x402-paid job failed: {x402_job}")

    print(json.dumps({"ok": True, "space": sid, "x402_space": sid2, "job_id": job_id}))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"data e2e failed: {exc}", file=sys.stderr)
        raise
