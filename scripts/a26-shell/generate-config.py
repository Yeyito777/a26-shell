#!/usr/bin/env python3
import hashlib
import json
import os
import pathlib
import secrets
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: generate-config.py OUTPUT")

pin = os.environ.get("A26_SHELL_PIN")
if pin is None:
    raise SystemExit("A26_SHELL_PIN is not present in the environment")
if not (pin.isascii() and pin.isdigit() and len(pin) == 6):
    raise SystemExit("A26_SHELL_PIN must contain exactly six ASCII digits")

salt = secrets.token_bytes(32)
digest = hashlib.sha256(salt + pin.encode("ascii")).hexdigest()
config = {
    "pin_salt_hex": salt.hex(),
    "pin_hash_hex": digest,
    "pin_length": 6,
    "start_locked": True,
    "initial_volume": 50,
    "socket_path": "/run/a26-shell/control.sock",
}

output = pathlib.Path(sys.argv[1])
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(config, indent=2) + "\n")
output.chmod(0o600)

