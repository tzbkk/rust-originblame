"""JSONL storage engine with hash sharding support."""

import json
import sys
import threading
from itertools import chain
from pathlib import Path
from typing import Iterator

# Thread safety for write operations
_write_lock = threading.Lock()

# Canonical layer names for shard storage
LAYER_AUTHORS = "authors"
LAYER_SECTION = "sections"
LAYER_MANIFEST = "document-index"
LAYER_INDEX = "index"


def bucket_path(ob_dir: Path, layer: str, hash_hex: str) -> Path:
    """Get the path to a shard file based on hash.

    Shard files are stored in .ob/{layer}/{first_2_hex}
    Files have no suffix (e.g., "00", "01", ..., "ff")

    Args:
        ob_dir: Root directory of the repository.
        layer: Layer name (e.g., "authors", "sections", "document-index",
            "index").
        hash_hex: Hash hex string.

    Returns:
        Path to the shard file for the given hash.
    """
    bucket = hash_hex[:2].lower()
    return ob_dir / ".ob" / layer / bucket


def jsonl_read(path: Path) -> list[dict]:
    """Read all records from a JSONL file.

    Args:
        path: Path to JSONL file (one JSON object per line)

    Returns:
        List of records. Empty list if file doesn't exist.
        Malformed lines are skipped with warning.
    """
    if not path.exists():
        return []

    records = []
    with open(path, "r", encoding="utf-8") as f:
        for line_num, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                records.append(json.loads(line))
            except json.JSONDecodeError:
                print(
                    f"Warning: Malformed JSON at line {line_num} in {path}: {line[:50]}...",
                    file=sys.stderr,
                )
    return records


def jsonl_write(path: Path, records: list[dict]) -> None:
    """Write all records to a JSONL file, one per line.

    Creates parent directories if needed. Overwrites entire file.

    Args:
        path: Path to JSONL file
        records: List of records to write
    """
    path.parent.mkdir(parents=True, exist_ok=True)

    with _write_lock:
        with open(path, "w", encoding="utf-8") as f:
            for record in records:
                f.write(json.dumps(record, ensure_ascii=False) + "\n")


def jsonl_append(path: Path, record: dict) -> None:
    """Append one record to a JSONL file.

    Creates file and parent directories if needed.

    Args:
        path: Path to JSONL file
        record: Record to append
    """
    path.parent.mkdir(parents=True, exist_ok=True)

    with _write_lock:
        with open(path, "a", encoding="utf-8") as f:
            f.write(json.dumps(record, ensure_ascii=False) + "\n")


def jsonl_iterate(path: Path) -> Iterator[dict]:
    """Stream records from a JSONL file without loading all into memory.

    Args:
        path: Path to JSONL file

    Yields:
        Records one at a time. Malformed lines are skipped with warning.
    """
    if not path.exists():
        return

    with open(path, "r", encoding="utf-8") as f:
        for line_num, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError:
                print(
                    f"Warning: Malformed JSON at line {line_num} in {path}: {line[:50]}...",
                    file=sys.stderr,
                )


def shard_read(ob_dir: Path, layer: str, hash_hex: str) -> list[dict]:
    """Read records from a shard file for the given hash.

    Args:
        ob_dir: Root directory of the repository
        layer: Layer name (e.g., "authors", "sections", "document-index", "index")
        hash_hex: Hash hex string

    Returns:
        List of records. Empty list if shard file doesn't exist.
    """
    shard_file = bucket_path(ob_dir, layer, hash_hex)
    return jsonl_read(shard_file)


def shard_append(ob_dir: Path, layer: str, hash_hex: str, record: dict) -> None:
    """Append a record to the correct shard file.

    Creates file and directories if needed.

    Args:
        ob_dir: Root directory of the repository
        layer: Layer name (e.g., "authors", "sections", "document-index", "index")
        hash_hex: Hash hex string
        record: Record to append
    """
    shard_file = bucket_path(ob_dir, layer, hash_hex)
    jsonl_append(shard_file, record)


def shard_iterate_all(ob_dir: Path, layer: str) -> Iterator[dict]:
    """Iterate ALL records across all 256 shard files (00..ff).

    Args:
        ob_dir: Root directory of the repository
        layer: Layer name (e.g., "authors", "sections", "document-index", "index")

    Yields:
        Records from all shard files. Non-existent shard files are skipped.
    """
    shard_dir = ob_dir / ".ob" / layer

    if not shard_dir.exists():
        return

    iterators = []
    for i in range(256):
        bucket = f"{i:02x}"
        shard_file = shard_dir / bucket
        if shard_file.exists():
            iterators.append(jsonl_iterate(shard_file))

    yield from chain.from_iterable(iterators)
