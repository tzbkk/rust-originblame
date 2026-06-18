"""Track function for recording data provenance with WAL lifecycle."""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from ob import embeddings, indexer
from ob.exceptions import OBTrackError
from ob.register import find_sections_by_path
from ob.source import get_active_section_hashes, get_active_sources
from ob.util import compute_hash, find_ob_dir, normalize_file

__all__ = ["TrackResult", "track"]


@dataclass
class TrackResult:
    """Result of a track() call."""

    line_hash: str
    file: str
    sources: list[str]
    written: bool


def track(
    data: dict | str,
    file: str,
    embedding: list[float] | None = None,
    model: str | None = None,
    *,
    source: str | list[str] | None = None,
    ob_dir: Path | None = None,
) -> TrackResult:
    """Record a data provenance entry.

    Writes an index entry (and optional embedding) via the WAL mechanism.
    Idempotent: if an identical entry already exists in merged storage,
    returns written=False without writing anything new.

    Args:
        data: The data to track (dict or str).
        file: File path associated with this tracking entry.
        source: Optional source. A file path resolves to all sections
            registered for that path; a list of section hashes is used
            directly.  If *None*, the source stack is used (backward
            compatible with ``source.append()``).
        embedding: Optional embedding vector. Requires *model*.
        model: Optional embedding model name. Requires *embedding*.
        ob_dir: Repository root directory. Auto-detected if None.

    Returns:
        TrackResult with the line_hash, file, sources, and whether
        a new entry was written.

    Raises:
        OBTrackError: If no source is available, embedding/model mismatch,
            or lock is held by this process.
        OBStorageError: If unmerged pid files exist (from read_manifest).
        TypeError: If data is not dict or str.
    """
    if ob_dir is None:
        ob_dir = find_ob_dir()

    file = normalize_file(file, ob_dir)

    # Resolve section hashes from explicit source or source stack.
    if source is not None:
        if isinstance(source, str):
            records = find_sections_by_path(ob_dir, source)
            if not records:
                raise OBTrackError(
                    f"No sections found for: {source}. "
                    "Register a section first with 'ob register.add'."
                )
            section_hashes = {r["section_hash"] for r in records}
        else:
            section_hashes = set(source)
    else:
        if not get_active_sources():
            raise OBTrackError("source stack is empty; call source.append() or pass source= explicitly")
        section_hashes = get_active_section_hashes()

    # Validate embedding/model pairing.
    if embedding is not None and model is None:
        raise OBTrackError("embedding provided but model is None")
    if model is not None and embedding is None:
        raise OBTrackError("model provided but embedding is None")

    # Validate data type.
    if not isinstance(data, (dict, str)):
        raise TypeError(f"track requires dict or str, got {type(data).__name__}")

    # Compute hash and gather sources.
    line_hash = compute_hash(data)
    sources = sorted(section_hashes)

    pid = os.getpid()

    # Idempotency: check if already tracked.
    existing = indexer.read_manifest(ob_dir, line_hash, file, sources, own_pid=pid)
    if existing is not None:
        return TrackResult(
            line_hash=line_hash,
            file=file,
            sources=sources,
            written=False,
        )

    pid = os.getpid()

    # Idempotency: check if already tracked.
    existing = indexer.read_manifest(ob_dir, line_hash, file, sources, own_pid=pid)
    if existing is not None:
        return TrackResult(
            line_hash=line_hash,
            file=file,
            sources=sources,
            written=False,
        )

    # WAL lifecycle: acquire lock, write, release.
    indexer.acquire_lock(ob_dir, pid)
    try:
        entry = {
            "line_hash": line_hash,
            "file": file,
            "sources": sources,
            "source_type": "track",
            "revoked": False,
        }
        indexer.track_write(ob_dir, pid, entry)

        # Cache the write so read_manifest skips shard_read on next call
        indexer._dedup_key(ob_dir).add((line_hash, file, tuple(sources)))

        if embedding is not None and model is not None:
            embeddings.write_embedding(ob_dir, str(pid), model, line_hash, embedding)
    finally:
        indexer.release_lock(ob_dir, pid)

    return TrackResult(
        line_hash=line_hash,
        file=file,
        sources=sources,
        written=True,
    )
