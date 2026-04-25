"""Capture a baseline for a named scenario.

Usage:
    python capture_baseline.py --scenario=help_text
    python capture_baseline.py --scenario=nothing_to_boot
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import sys
import tempfile

from run_headless import run_headless, _default_exe
from lib.log_parser import canonicalize

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASELINES = ROOT / "baselines"

# Map scenario name -> list of rpcs3 args (beyond --headless).
# Each scenario must be reproducible without any outside state.
SCENARIOS: dict[str, list[str]] = {
    # Prints the QCommandLineParser help text and exits.
    # Depends only on rpcs3/rpcs3.cpp:387-410 + 807-856.
    "help_text": ["--help"],

    # Passing a non-existent path forces BootGame → nothing_to_boot.
    # Depends on rpcs3/Emu/System.cpp:936 (BootGame).
    "nothing_to_boot": ["/nonexistent/path/EBOOT.BIN"],
}


def _fingerprint(s: str) -> str:
    return hashlib.sha256(s.encode("utf-8")).hexdigest()


def capture(scenario: str, rpcs3_exe: pathlib.Path, force: bool) -> int:
    if scenario not in SCENARIOS:
        print(f"Unknown scenario: {scenario}. Known: {list(SCENARIOS)}", file=sys.stderr)
        return 2

    args = SCENARIOS[scenario]
    out_dir = BASELINES / scenario
    if out_dir.exists() and not force:
        print(f"Baseline already exists at {out_dir}. Use --force to overwrite.", file=sys.stderr)
        return 1

    out_dir.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="rpcs3-bf-") as tmp:
        cfg = pathlib.Path(tmp)
        result = run_headless(rpcs3_exe, args, config_dir=cfg, timeout_s=120)

    canon_log = canonicalize(result["log_text"])
    canon_stdout = canonicalize(result["stdout"])
    canon_stderr = canonicalize(result["stderr"])

    (out_dir / "exit_code.txt").write_text(str(result["exit_code"]), encoding="utf-8")
    (out_dir / "stdout.canon.txt").write_text(canon_stdout, encoding="utf-8")
    (out_dir / "stderr.canon.txt").write_text(canon_stderr, encoding="utf-8")
    (out_dir / "RPCS3.log.canon.txt").write_text(canon_log, encoding="utf-8")

    manifest = {
        "scenario": scenario,
        "args": args,
        "rpcs3_exe": str(rpcs3_exe),
        "exit_code": result["exit_code"],
        "sha256_stdout": _fingerprint(canon_stdout),
        "sha256_stderr": _fingerprint(canon_stderr),
        "sha256_log": _fingerprint(canon_log),
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    print(f"Captured baseline for '{scenario}' at {out_dir}")
    print(f"  exit_code={result['exit_code']}")
    print(f"  sha256_log={manifest['sha256_log']}")
    return 0


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--scenario", required=True)
    p.add_argument("--rpcs3-exe", type=pathlib.Path, default=_default_exe())
    p.add_argument("--force", action="store_true")
    ns = p.parse_args(argv)

    if not ns.rpcs3_exe.exists():
        print(f"ERROR: rpcs3 binary not found at {ns.rpcs3_exe}", file=sys.stderr)
        print("Build it first: cmake --build build --target rpcs3", file=sys.stderr)
        return 2

    return capture(ns.scenario, ns.rpcs3_exe, ns.force)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
