#!/usr/bin/env python3
"""Run Mirage emulator smoke tests for Bluetooth Audio."""
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CANOPUS = Path(os.environ.get("CANOPUS_ROOT", ROOT / "../Canopus")).resolve()
RUNNER = CANOPUS / "scripts/emu/module_smoke.py"

if not RUNNER.is_file():
    sys.exit(f"error: Mirage module runner not found at {RUNNER}")

cmd = [sys.executable, str(RUNNER), "--module", str(ROOT)] + sys.argv[1:]
sys.exit(subprocess.call(cmd))
