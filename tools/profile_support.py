"""Stable evidence primitives, independent of any historical experiment.

Inventories retain the existing [byte size, SHA-256] wire format. A build label
always owns a separate Cargo target so frozen revisions/features cannot reuse
an executable merely because copied source timestamps happen to be older.
"""
import hashlib
import json
from pathlib import Path

BUILD_ENV = {
    "CARGO_INCREMENTAL": "0",
    "CARGO_BUILD_JOBS": "2",
    "CARGO_PROFILE_RELEASE_DEBUG": "1",
}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def inventory(root):
    return {
        str(path.relative_to(root)): [path.stat().st_size, digest(path)]
        for path in sorted(root.rglob("*"))
        if path.is_file() and "__pycache__" not in path.parts
    }


def target_directory(root, label):
    """A revision/feature-specific build directory, never an opt-in policy."""
    if not label or label in {".", ".."} or Path(label).name != label or "\\" in label:
        raise ValueError("build label must be one directory name")
    return root / label
