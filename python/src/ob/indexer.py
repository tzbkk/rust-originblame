"""Indexer storage with PID-file-based writes and WAL lock mechanism."""

from __future__ import annotations

from pathlib import Path

from ob.exceptions import OBStorageError, OBTrackError
from ob.storage import (
    LAYER_MANIFEST,
    jsonl_append,
    jsonl_read,
    shard_iterate_all,
    shard_read,
)

__all__ = [
    "track_write",
    "track_write_embedding",
    "get_lock_path",
    "acquire_lock",
    "release_lock",
    "read_manifest",
    "read_all_manifest",
    "list_pid_files",
    "is_revoked",
    "invalidate_manifest_cache",
]

# Indexer dedup cache: ob_dir_str -> {(line_hash, file, sources_tuple): True}
_dedup_cache: dict[str, set[tuple[str, str, tuple[str, ...]]]] = {}


def _dedup_key(ob_dir: Path) -> set[tuple[str, str, tuple[str, ...]]]:
    key = str(ob_dir.resolve())
    if key not in _dedup_cache:
        _dedup_cache[key] = set()
    return _dedup_cache[key]


def invalidate_manifest_cache(ob_dir: Path | None = None) -> None:
    """Clear the in-memory manifest dedup cache.

    If ``ob_dir`` is given, clears only that repository's cache entries
    (dedup and unmerged-PID check); otherwise clears the caches for all
    repositories.

    Args:
        ob_dir: Repository root directory. If *None*, all caches are cleared.
    """
    if ob_dir is None:
        _dedup_cache.clear()
        _unmerged_checked.clear()
    else:
        _dedup_cache.pop(str(ob_dir.resolve()), None)
        _unmerged_checked.pop(str(ob_dir.resolve()), None)


def _pid_file_path(ob_dir: Path, pid: int) -> Path:

    return ob_dir / ".ob" / f"docidx.{pid}"


def track_write(ob_dir: Path, pid: int, entry: dict) -> None:
    """Write an index entry to the PID-specific flat file (not sharded).

    Args:
        ob_dir: Repository root directory.
        pid: Process ID used for PID-file naming.
        entry: Record dict with keys: ``line_hash``, ``file``,
            ``sources``, ``source_type``, ``revoked``.
    """
    path = _pid_file_path(ob_dir, pid)
    jsonl_append(path, entry)


def track_write_embedding(
    ob_dir: Path, pid: int, model: str, line_hash: str, embedding: list[float]
) -> None:
    """Append an embedding record to the PID-specific embedding file.

    Args:
        ob_dir: Repository root directory.
        pid: Process ID used for PID-file naming.
        model: Embedding model name.
        line_hash: 64-char SHA-256 hex of the tracked line.
        embedding: Float vector.
    """
    path = ob_dir / ".ob" / f"embeddings.{model}.{pid}"
    jsonl_append(path, {"line_hash": line_hash, "embedding": embedding})


def get_lock_path(ob_dir: Path, pid: int) -> Path:
    """Return the path to the WAL lock file for a given PID.

    Args:
        ob_dir: Repository root directory.
        pid: Process ID.

    Returns:
        Path to ``.ob/lock.{pid}``.
    """
    return ob_dir / ".ob" / f"lock.{pid}"


def acquire_lock(ob_dir: Path, pid: int) -> None:
    """Acquire the WAL write lock by creating a lock file.

    Args:
        ob_dir: Repository root directory.
        pid: Process ID.

    Raises:
        OBTrackError: If a lock file already exists for this PID
            (re-entrant write attempted).
    """
    lock_path = get_lock_path(ob_dir, pid)
    if lock_path.exists():
        raise OBTrackError(f"lock file already exists for pid {pid}")
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    lock_path.touch()


def release_lock(ob_dir: Path, pid: int) -> None:
    """Release the WAL write lock by removing the lock file.

    No-op if the lock file does not exist.

    Args:
        ob_dir: Repository root directory.
        pid: Process ID.
    """
    lock_path = get_lock_path(ob_dir, pid)
    lock_path.unlink(missing_ok=True)


def list_pid_files(ob_dir: Path) -> list[Path]:
    """List all unmerged PID files (``docidx.*``) in the ``.ob/`` directory.

    Args:
        ob_dir: Repository root directory.

    Returns:
        List of Paths to PID files. Empty list if ``.ob/`` does not exist.
    """
    ob = ob_dir / ".ob"
    if not ob.exists():
        return []
    pid_files = []
    for p in ob.glob("docidx.*"):
        if p.is_file():
            pid_files.append(p)
    return pid_files


_unmerged_checked: dict[str, bool] = {}


def _check_unmerged_pid_files(ob_dir: Path, own_pid: int | None = None) -> None:
    key = str(ob_dir.resolve())
    if _unmerged_checked.get(key):
        return
    pid_files = list_pid_files(ob_dir)
    for p in pid_files:
        if own_pid is not None and p.name == f"docidx.{own_pid}":
            continue
        raise OBStorageError("Unmerged PID files detected. Run 'ob clean' first.")
    _unmerged_checked[key] = True


def read_manifest(
    ob_dir: Path,
    line_hash: str,
    file: str,
    sources: list[str],
    *,
    own_pid: int | None = None,
) -> dict | None:
    """Look up an existing manifest entry by (line_hash, file, sources).

    Uses an in-memory dedup cache to skip repeated shard reads for the
    same lookup key.

    Args:
        ob_dir: Repository root directory.
        line_hash: 64-char SHA-256 hex of the tracked line.
        file: File path associated with the entry.
        sources: List of section hashes.
        own_pid: If set, skip this PID's own PID file during the
            unmerged-files check.

    Returns:
        The matching record dict, or *None* if not found.

    Raises:
        OBStorageError: If unmerged PID files from other processes are
            detected (run ``ob clean`` first).
    """
    _check_unmerged_pid_files(ob_dir, own_pid=own_pid)

    sorted_sources = tuple(sorted(sources))
    lookup = (line_hash, file, sorted_sources)

    cache = _dedup_key(ob_dir)
    if lookup in cache:
        return {"line_hash": line_hash, "file": file, "sources": list(sorted_sources)}

    records = shard_read(ob_dir, LAYER_MANIFEST, line_hash)
    for record in records:
        if (
            record.get("line_hash") == line_hash
            and record.get("file") == file
            and tuple(sorted(record.get("sources", []))) == sorted_sources
        ):
            cache.add(lookup)
            return record

    cache.add(lookup)
    return None


def read_all_manifest(ob_dir: Path) -> list[dict]:
    """Read all manifest records across all shards (full scan).

    Args:
        ob_dir: Repository root directory.

    Returns:
        List of all manifest records.
    """
    return list(shard_iterate_all(ob_dir, LAYER_MANIFEST))


def is_revoked(ob_dir: Path, section_hash: str) -> bool:
    """Check whether a section has been revoked.

    Args:
        ob_dir: Repository root directory.
        section_hash: Section hash to look up.

    Returns:
        True if the section's ``revoked`` field is True; False if the
        section is not found or not revoked.
    """
    # Lazy import to avoid circular dependency when section module isn't ready
    from ob.register import get_section

    section = get_section(ob_dir, section_hash)
    if section is None:
        return False
    return bool(section.get("revoked", False))
