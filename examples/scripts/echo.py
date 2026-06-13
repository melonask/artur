#!/usr/bin/env python3
import argparse
import json
import sys

parser = argparse.ArgumentParser()
parser.add_argument("--name", default="")
parser.add_argument("--source", default="")
args = parser.parse_args()
stdin = sys.stdin.read()
try:
    request = json.loads(stdin) if stdin else None
except json.JSONDecodeError:
    request = stdin

print(json.dumps({
    "ok": True,
    "name": args.name,
    "source": args.source,
    "request": request,
}, separators=(",", ":")))
