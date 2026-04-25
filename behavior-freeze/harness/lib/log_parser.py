"""Canonicalize RPCS3.log for deterministic diffing.

RPCS3.log format (observed in rpcs3/util/logs.cpp):

    <timestamp> <SEV> {<channel>} <thread-id> <msg>

We strip the volatile parts (timestamp, thread-id, absolute paths,
process IDs, ccache paths, build-specific version strings) and keep:

    <SEV> {<channel>} <msg-normalized>

This lets us snapshot a log as a stable golden.
"""

from __future__ import annotations

import re

# Matches e.g. "S U" severity markers or the inline level prefixes used
# by the log writer in rpcs3/util/logs.cpp. Kept loose on purpose:
# anything that doesn't match falls through unchanged.
_TIMESTAMP_RE = re.compile(r"^\d{2,4}[-:/]\d{1,2}[-:/.]\d{1,2}[T ]\d{1,2}:\d{1,2}:\d{1,2}(?:\.\d+)?\s*")
_LEADING_US_RE = re.compile(r"^[·· ]*\d+(?:\.\d+)?\s*(?:ms|us|µs)?\s*")
_TID_RE = re.compile(r"\{[0-9A-Fa-f]{4,16}\}")
_ADDR_RE = re.compile(r"0x[0-9A-Fa-f]{6,16}")
_ABS_PATH_RE = re.compile(r"([A-Za-z]:\\|/)[^\s:'\"]*?rpcs3[\\/][^\s:'\"]*")
_PID_RE = re.compile(r"\bpid=\d+\b")
_VERSION_RE = re.compile(r"\bv?\d+\.\d+\.\d+(?:-\w+)?(?:-\d+-g[0-9a-f]+)?\b")


def canonicalize_line(line: str) -> str:
    s = line.rstrip("\r\n")
    s = _TIMESTAMP_RE.sub("", s)
    s = _LEADING_US_RE.sub("", s)
    s = _TID_RE.sub("{TID}", s)
    s = _ADDR_RE.sub("0xADDR", s)
    s = _ABS_PATH_RE.sub("<RPCS3>/PATH", s)
    s = _PID_RE.sub("pid=PID", s)
    s = _VERSION_RE.sub("vVER", s)
    return s


def canonicalize(text: str) -> str:
    return "\n".join(canonicalize_line(l) for l in text.splitlines()) + ("\n" if text.endswith("\n") else "")
