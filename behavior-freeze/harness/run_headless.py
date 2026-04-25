"""Thin wrapper over `rpcs3 --headless` for capturing observable output.

This does not link against rpcs3 at all. It only spawns the already-built
binary with well-known CLI flags and captures stdout/stderr/RPCS3.log to
a snapshot directory. Every behavior-freeze scenario is implemented as a
function in `scenarios/`.
"""

from __future__ import annotations

import argparse
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

# Default search for the rpcs3 binary. Override via --rpcs3-exe.
def _default_exe() -> pathlib.Path:
    root = pathlib.Path(__file__).resolve().parents[2]
    candidates = [
        root / "build" / "bin" / "rpcs3.exe",
        root / "build" / "bin" / "rpcs3",
        root / "bin" / "rpcs3.exe",
        root / "bin" / "rpcs3",
    ]
    for c in candidates:
        if c.exists():
            return c
    return candidates[0]  # will fail loudly at run-time


def run_headless(
    exe: pathlib.Path,
    args: list[str],
    *,
    config_dir: pathlib.Path,
    timeout_s: int = 120,
) -> dict:
    """Spawn `rpcs3 --headless <args>` with an isolated config dir.

    Returns a dict with exit_code, stdout, stderr, log_path, log_text.
    """
    env = os.environ.copy()
    # Portable mode: RPCS3 reads RPCS3_CONFIG_DIR first
    # (Utilities/File.cpp:2128-2223). This isolates the test from the
    # user's installation.
    env["RPCS3_CONFIG_DIR"] = str(config_dir)

    proc = subprocess.run(
        [str(exe), "--headless", *args],
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout_s,
    )

    log_path = _find_log(config_dir)
    log_text = log_path.read_text(encoding="utf-8", errors="replace") if log_path else ""

    return {
        "exit_code": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
        "log_path": str(log_path) if log_path else None,
        "log_text": log_text,
    }


def _find_log(config_dir: pathlib.Path) -> pathlib.Path | None:
    # Windows: <config_dir>/log/RPCS3.log (Utilities/File.cpp:2267-2275)
    # Unix: cache dir
    for candidate in [
        config_dir / "log" / "RPCS3.log",
        config_dir / "RPCS3.log",
    ]:
        if candidate.exists():
            return candidate
    return None


def _parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Run rpcs3 --headless and capture output")
    p.add_argument("--rpcs3-exe", type=pathlib.Path, default=_default_exe())
    p.add_argument("--out-dir", type=pathlib.Path, required=True, help="Where to store captured outputs")
    p.add_argument("--timeout", type=int, default=120)
    p.add_argument("rpcs3_args", nargs=argparse.REMAINDER, help="Args forwarded to rpcs3")
    return p.parse_args(argv)


def main(argv: list[str]) -> int:
    args = _parse_args(argv)
    if not args.rpcs3_exe.exists():
        print(f"ERROR: rpcs3 binary not found at {args.rpcs3_exe}", file=sys.stderr)
        print("Build with: cmake --build build --target rpcs3", file=sys.stderr)
        return 2

    args.out_dir.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="rpcs3-bf-") as tmp:
        cfg = pathlib.Path(tmp)
        result = run_headless(
            args.rpcs3_exe,
            args.rpcs3_args,
            config_dir=cfg,
            timeout_s=args.timeout,
        )

        (args.out_dir / "exit_code.txt").write_text(str(result["exit_code"]), encoding="utf-8")
        (args.out_dir / "stdout.txt").write_text(result["stdout"], encoding="utf-8")
        (args.out_dir / "stderr.txt").write_text(result["stderr"], encoding="utf-8")
        (args.out_dir / "RPCS3.log").write_text(result["log_text"], encoding="utf-8")

    print(f"Captured headless run to {args.out_dir} (exit={result['exit_code']})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
