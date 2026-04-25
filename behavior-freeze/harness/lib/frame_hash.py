"""Stable hash for captured frame payloads (RSX .rrc files or raw RGBA8).

Placeholder for P1 scope. The hash MUST be:
  - independent of little/big endian of the host
  - independent of file mtime / filesystem layout
  - stable across OSes

TODO(P1): implement when we have a homebrew fixture producing a .rrc.
The format is defined by c_fc_magic="RRC" / c_fc_version=0x6 in
rpcs3/Emu/RSX/Capture/rsx_replay.h:13-14. We must serialize only the
logical content (replay_commands + memory_map + reg_state), not the
surrounding file structure, to avoid false negatives when the serial
format evolves.
"""

from __future__ import annotations

import hashlib
import pathlib


def hash_rrc_file(path: pathlib.Path) -> str:
    """Raw SHA-256 of the file bytes. Good enough for a first cut, but
    will flake if the serial format changes. Upgrade to structural
    hashing when needed."""
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()
