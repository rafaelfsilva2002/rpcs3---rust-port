"""Rerun a scenario and compare against its stored baseline.

Usage:
    python compare_run.py --scenario=help_text
    python compare_run.py --scenario=nothing_to_boot --rpcs3-exe=/path/to/rpcs3-rust

This is the backbone of differential testing between the current C++
build (oracle) and a future Rust build: capture once with the C++ exe,
compare repeatedly — eventually with --rpcs3-exe pointing at the Rust
binary.
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import pathlib
import sys
import tempfile

from run_headless import run_headless, _default_exe
from lib.log_parser import canonicalize
from capture_baseline import SCENARIOS, BASELINES

ROOT = pathlib.Path(__file__).resolve().parents[1]


def _fingerprint(s: str) -> str:
    return hashlib.sha256(s.encode("utf-8")).hexdigest()


def compare(scenario: str, rpcs3_exe: pathlib.Path, show_diff: bool) -> int:
    baseline_dir = BASELINES / scenario
    manifest_path = baseline_dir / "manifest.json"
    if not manifest_path.exists():
        print(f"No baseline for '{scenario}' at {baseline_dir}", file=sys.stderr)
        print(f"Capture first: python capture_baseline.py --scenario={scenario}", file=sys.stderr)
        return 2

    baseline = json.loads(manifest_path.read_text(encoding="utf-8"))

    with tempfile.TemporaryDirectory(prefix="rpcs3-bf-") as tmp:
        cfg = pathlib.Path(tmp)
        result = run_headless(rpcs3_exe, SCENARIOS[scenario], config_dir=cfg, timeout_s=120)

    canon_log = canonicalize(result["log_text"])
    canon_stdout = canonicalize(result["stdout"])
    canon_stderr = canonicalize(result["stderr"])

    failures: list[str] = []

    if result["exit_code"] != baseline["exit_code"]:
        failures.append(
            f"exit_code differs: baseline={baseline['exit_code']} actual={result['exit_code']}"
        )

    for label, actual, key in [
        ("stdout", canon_stdout, "sha256_stdout"),
        ("stderr", canon_stderr, "sha256_stderr"),
        ("log", canon_log, "sha256_log"),
    ]:
        expected_hash = baseline[key]
        actual_hash = _fingerprint(actual)
        if expected_hash != actual_hash:
            failures.append(f"{label} hash differs: expected {expected_hash}, got {actual_hash}")
            if show_diff:
                expected_text = (baseline_dir / f"{label.replace('log', 'RPCS3.log')}.canon.txt").read_text(
                    encoding="utf-8"
                )
                diff = difflib.unified_diff(
                    expected_text.splitlines(keepends=True),
                    actual.splitlines(keepends=True),
                    fromfile=f"baseline/{label}",
                    tofile=f"actual/{label}",
                    n=3,
                )
                sys.stderr.write("".join(diff))

    if failures:
        print(f"FAIL '{scenario}':", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        return 1

    print(f"OK '{scenario}' matches baseline.")
    return 0


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--scenario", required=True)
    p.add_argument("--rpcs3-exe", type=pathlib.Path, default=_default_exe())
    p.add_argument("--show-diff", action="store_true")
    ns = p.parse_args(argv)

    if not ns.rpcs3_exe.exists():
        print(f"ERROR: rpcs3 binary not found at {ns.rpcs3_exe}", file=sys.stderr)
        return 2

    return compare(ns.scenario, ns.rpcs3_exe, ns.show_diff)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
