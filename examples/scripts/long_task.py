#!/usr/bin/env python3
import json
import sys
import time

body = sys.stdin.read()
time.sleep(0.1)
print(json.dumps({"ok": True, "received": body}, separators=(",", ":")))
