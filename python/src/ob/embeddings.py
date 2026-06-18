"""Embedding vector storage layer.

Read/write embedding vectors via JSONL, with sharded storage and PID-file support.
"""

from __future__ import annotations

import re
from pathlib import Path
from ob.exceptions import OBStorageError
from ob.storage import jsonl_append, jsonl_iterate, shard_iterate_all


# Pattern matching PID files: embeddings.{model}.{pid}
# PID is a hex string (git commit-like), must not look like a shard name (2 hex chars)
_PID_FILE_RE = re.compile(r"^embeddings\.([^.]+)\.([0-9a-f]+)$")


def write_embedding(
    ob_dir: Path,
    pid: str,
    model: str,
    line_hash: str,
    embedding: list[float],
) -> None:
    """Write a single embedding record to a PID file.

    Args:
        ob_dir: Root directory of the repository.
        pid: Process/file identifier (e.g., file hash).
        model: Embedding model name (e.g., "MiniLM").
        line_hash: Hash of the line content this embedding represents.
        embedding: Full float vector.
    """
    pid_file = ob_dir / ".ob" / f"embeddings.{model}.{pid}"
    record: dict = {"line_hash": line_hash, "embedding": embedding}
    jsonl_append(pid_file, record)


def _check_unmerged_pid_files(ob_dir: Path, model: str) -> list[Path]:
    """Return list of unmerged PID files for the given model, or empty list."""
    ob_dir_path = Path(ob_dir)
    ob_dot = (ob_dir_path / ".ob").resolve()

    prefix = f"embeddings.{model}."
    candidates: list[Path] = []

    if not ob_dot.exists():
        return candidates

    for child in ob_dot.iterdir():
        if child.name.startswith(prefix) and child.is_file():
            suffix = child.name[len(prefix):]
            # A shard bucket is exactly 2 hex chars -- skip those
            if len(suffix) != 2 or not all(c in "0123456789abcdef" for c in suffix):
                candidates.append(child)

    return candidates


def read_embedding(
    ob_dir: Path, model: str, line_hash: str
) -> list[float] | None:
    """Read an embedding vector for a given line_hash.

    Searches across all shard files for the model.
    Raises OBStorageError if unmerged PID files are detected.

    Args:
        ob_dir: Root directory of the repository.
        model: Embedding model name.
        line_hash: Hash of the line content.

    Returns:
        The embedding vector, or None if not found.

    Raises:
        OBStorageError: If unmerged PID files exist.
    """
    unmerged = _check_unmerged_pid_files(ob_dir, model)
    if unmerged:
        raise OBStorageError(
            "Unmerged PID files detected. Run 'ob clean' first."
        )

    for record in shard_iterate_all(ob_dir, f"embeddings.{model}"):
        if record.get("line_hash") == line_hash:
            emb = record.get("embedding")
            if isinstance(emb, list):
                return emb
    return None


def read_all_embeddings(ob_dir: Path, model: str) -> list[dict]:
    """Read all embedding records for a model (sharded + PID files).

    Iterates both merged shard files and unmerged PID flat files.
    Deduplicates by line_hash -- last occurrence wins.

    Args:
        ob_dir: Root directory of the repository.
        model: Embedding model name.

    Returns:
        List of records, each with "line_hash" and "embedding" keys.
    """
    seen: dict[str, dict] = {}

    # First: iterate PID flat files
    ob_dir_path = Path(ob_dir)
    ob_dot = (ob_dir_path / ".ob").resolve()
    prefix = f"embeddings.{model}."

    if ob_dot.exists():
        for child in sorted(ob_dot.iterdir()):
            if child.name.startswith(prefix) and child.is_file():
                for record in jsonl_iterate(child):
                    lh = record.get("line_hash")
                    if isinstance(lh, str):
                        seen[lh] = record

    # Second: iterate merged shards (overrides PID file entries on collision)
    for record in shard_iterate_all(ob_dir, f"embeddings.{model}"):
        lh = record.get("line_hash")
        if isinstance(lh, str):
            seen[lh] = record

    return list(seen.values())
