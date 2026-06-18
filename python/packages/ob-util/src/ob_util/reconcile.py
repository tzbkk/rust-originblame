"""Reconcile data files against tracked provenance (two-phase: hash + embedding)."""

from __future__ import annotations

import hashlib
import json
import logging
import math
import time
import urllib.error

import numpy as np
import urllib.request
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from ob.exceptions import OBStorageError
from ob.indexer import list_pid_files
from ob.storage import (
    LAYER_MANIFEST,
    jsonl_append,
    jsonl_read,
    jsonl_write,
    shard_append,
    shard_iterate_all,
)
from ob.oplog import append_log
from ob.util import compute_hash, find_ob_dir, normalize_file

logger = logging.getLogger(__name__)

__all__ = ["ReconcileResult", "reconcile", "cosine_similarity", "make_api_encode_fn"]

_model_cache: dict[str, Any] = {}


def cosine_similarity(a: list[float], b: list[float]) -> float:
    if len(a) != len(b):
        raise ValueError(f"Dimension mismatch: {len(a)} != {len(b)}")
    dot = sum(x * y for x, y in zip(a, b))
    mag_a = math.sqrt(sum(x * x for x in a))
    mag_b = math.sqrt(sum(x * x for x in b))
    if mag_a == 0.0 or mag_b == 0.0:
        return 0.0
    return dot / (mag_a * mag_b)


def _compute_line_hash(line: str) -> str:
    """Compute hash for a data line, attempting JSON normalization.

    Tries to parse the line as JSON. If it is a dict, normalizes via
    ``json.dumps(sort_keys=True, separators=(",", ":"))`` before hashing
    to match Rust's ``serde_json::to_string`` output (compact, sorted keys).
    Falls back to raw string hash.
    """
    try:
        data = json.loads(line)
        if isinstance(data, dict):
            compact = json.dumps(data, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
            return hashlib.sha256(compact.encode("utf-8")).hexdigest()
    except (json.JSONDecodeError, TypeError):
        pass
    return hashlib.sha256(line.encode("utf-8")).hexdigest()


@dataclass
class ReconcileResult:
    """Result of a reconcile operation."""

    hash_matched: int = 0
    semantic_matched: int = 0
    new_lines: int = 0
    orphans: int = 0
    errors: int = 0
    orphan_hashes: list[str] = field(default_factory=list)
    duration_ms: float = 0.0


@dataclass
class _ManifestChange:
    action: str
    bucket: str
    record: dict


def _load_manifest_index(ob_dir: Path, file: str) -> dict[str, list[dict]]:
    index: dict[str, list[dict]] = defaultdict(list)
    for record in shard_iterate_all(ob_dir, LAYER_MANIFEST):
        if record.get("file") == file and not record.get("revoked"):
            lh = record.get("line_hash")
            if isinstance(lh, str):
                index[lh].append(record)
    return dict(index)


def _load_all_embeddings(ob_dir: Path, model: str) -> dict[str, list[float]]:
    if model is None:
        return {}
    from ob.embeddings import read_all_embeddings

    result: dict[str, list[float]] = {}
    for rec in read_all_embeddings(ob_dir, model):
        lh = rec.get("line_hash")
        emb = rec.get("embedding")
        if isinstance(lh, str) and isinstance(emb, list):
            result[lh] = emb
    return result


def _archive_record(ob_dir: Path, record: dict) -> None:
    line_hash = record.get("line_hash", "")
    if not line_hash:
        return
    bucket = line_hash[:2].lower()
    archive_path = ob_dir / ".ob" / "archive" / f"docidx.{bucket}"
    jsonl_append(archive_path, record)


def _rewrite_shard(ob_dir: Path, bucket: str, records: list[dict]) -> None:
    shard_path = ob_dir / ".ob" / LAYER_MANIFEST / bucket
    jsonl_write(shard_path, records)


def _tag_orphans(ob_dir: Path, manifest_index: dict[str, list[dict]]) -> None:
    by_bucket: dict[str, list[dict]] = defaultdict(list)
    for records in manifest_index.values():
        for rec in records:
            lh = rec.get("line_hash", "")
            if lh:
                by_bucket[lh[:2].lower()].append(rec)

    for bucket, orphan_records in by_bucket.items():
        shard_path = ob_dir / ".ob" / LAYER_MANIFEST / bucket
        existing = jsonl_read(shard_path)
        orphan_keys = {
            (r.get("line_hash", ""), r.get("file", "")) for r in orphan_records
        }
        updated = []
        for r in existing:
            if (r.get("line_hash", ""), r.get("file", "")) in orphan_keys:
                r["orphan"] = True
            updated.append(r)
        jsonl_write(shard_path, updated)


def _apply_manifest_changes(ob_dir: Path, changes: list[_ManifestChange]) -> None:
    grouped: dict[str, list[_ManifestChange]] = defaultdict(list)
    for ch in changes:
        grouped[ch.bucket].append(ch)

    for bucket, bucket_changes in grouped.items():
        shard_path = ob_dir / ".ob" / LAYER_MANIFEST / bucket
        current = jsonl_read(shard_path)

        remove_keys: set[tuple[str, str]] = set()
        add_records: list[dict] = []
        removed_records: list[dict] = []

        for ch in bucket_changes:
            if ch.action == "remove":
                rec = ch.record
                remove_keys.add((rec.get("line_hash", ""), rec.get("file", "")))
                removed_records.append(rec)
            elif ch.action == "add":
                add_records.append(ch.record)

        filtered = [
            r
            for r in current
            if (r.get("line_hash", ""), r.get("file", "")) not in remove_keys
        ]
        filtered.extend(add_records)
        _rewrite_shard(ob_dir, bucket, filtered)

        for rec in removed_records:
            _archive_record(ob_dir, rec)


def _compute_all_embeddings(
    ob_dir: Path,
    data_lines: list[tuple[int, str, str]],
    model: str,
    _encode_fn: Callable[[list[str]], list[list[float]]] | None,
    embedding_api: str | None,
) -> int:
    existing = _load_all_embeddings(ob_dir, model)
    needed = [(lh, txt) for _, txt, lh in data_lines if lh not in existing]
    if not needed:
        return 0
    texts = [t for _, t in needed]
    if _encode_fn is not None:
        embs = _encode_fn(texts)
    elif embedding_api is not None:
        embs = make_api_encode_fn(embedding_api, model)(texts)
    else:
        embs = _get_encode_fn(model)(texts)
    written: set[str] = set()
    for (lh, _), emb in zip(needed, embs):
        if lh not in written:
            shard_append(
                ob_dir,
                f"embeddings.{model}",
                lh,
                {"line_hash": lh, "embedding": emb},
            )
            written.add(lh)
    return len(written)


def _get_encode_fn(model_name: str) -> Callable[[list[str]], list[list[float]]]:
    if model_name in _model_cache:
        model = _model_cache[model_name]
    else:
        try:
            from sentence_transformers import SentenceTransformer
        except ImportError:
            raise ImportError(
                "sentence-transformers is required for embedding reconciliation. "
                "Install with: pip install 'ob-util[reconcile]'"
            )
        model = SentenceTransformer(model_name)
        _model_cache[model_name] = model

    def _encode(texts: list[str]) -> list[list[float]]:
        return model.encode(texts).tolist()

    return _encode


def make_api_encode_fn(
    api_url: str,
    model: str,
    batch_size: int = 64,
    timeout: float = 60.0,
) -> Callable[[list[str]], list[list[float]]]:
    """Create an encode function that calls an OpenAI-compatible /embeddings endpoint.

    Args:
        api_url: Base URL, e.g. ``http://localhost:1234/v1``.
        model: Model name passed in the request body.
        batch_size: Max texts per API call.
        timeout: HTTP request timeout in seconds.
    """
    endpoint = f"{api_url.rstrip('/')}/embeddings"

    def _encode(texts: list[str]) -> list[list[float]]:
        all_embeddings: list[list[float]] = []
        for i in range(0, len(texts), batch_size):
            batch = texts[i : i + batch_size]
            payload = json.dumps({"model": model, "input": batch}).encode("utf-8")
            req = urllib.request.Request(
                endpoint,
                data=payload,
                headers={"Content-Type": "application/json"},
            )
            try:
                with urllib.request.urlopen(req, timeout=timeout) as resp:
                    data = json.loads(resp.read())
            except (urllib.error.URLError, urllib.error.HTTPError, OSError) as exc:
                raise OBStorageError(f"Embedding API request failed: {exc}") from exc
            if "data" not in data:
                raise OBStorageError(
                    f"Embedding API returned unexpected response: {list(data.keys())}"
                )
            sorted_items = sorted(data["data"], key=lambda d: d["index"])
            all_embeddings.extend([item["embedding"] for item in sorted_items])
        return all_embeddings

    return _encode


def reconcile(
    data_file: str,
    model: str | None = None,
    threshold: float = 0.85,
    interactive: bool = False,
    ob_dir: Path | None = None,
    _encode_fn: Callable[[list[str]], list[list[float]]] | None = None,
    embedding_api: str | None = None,
    compute_all_embeddings: bool = False,
) -> ReconcileResult:
    """Reconcile a data file against tracked provenance.

    Two-pass matching:

    1. **Pass 1 -- hash exact match**: compute ``line_hash`` for each line,
       look up in manifest.  Exact content hash matches inherit provenance.
    2. **Pass 2 -- embedding semantic match** (optional): for unmatched
       lines, compute embeddings and find the nearest orphaned manifest
       record above *threshold* cosine similarity.

    Args:
        data_file: Path to the data file to reconcile.
        model: Embedding model name (e.g. ``all-MiniLM-L6-v2``).  ``None``
               skips semantic matching.
        threshold: Minimum cosine similarity for semantic matching.
        interactive: Reserved for future interactive mode.
        ob_dir: Repository root.  Auto-detected if ``None``.
        _encode_fn: Injected encode function (for testing).

    Returns:
        ``ReconcileResult`` with match counts and orphan information.

    Raises:
        OBStorageError: If unmerged PID files are detected.
        FileNotFoundError: If *data_file* does not exist.
    """
    if ob_dir is None:
        ob_dir = find_ob_dir()

    if not Path(data_file).exists():
        raise FileNotFoundError(f"Data file not found: {data_file}")

    pid_files = list_pid_files(ob_dir)
    if pid_files:
        raise OBStorageError(
            "Unmerged manifest PID files detected. Run 'ob clean' first."
        )

    file = normalize_file(data_file, ob_dir)
    t0 = time.monotonic()

    # Load data lines
    data_lines: list[tuple[int, str, str]] = []
    with open(data_file, "r", encoding="utf-8") as f:
        for line_num, raw_line in enumerate(f, 1):
            stripped = raw_line.strip()
            if not stripped:
                continue
            lh = _compute_line_hash(stripped)
            data_lines.append((line_num, stripped, lh))

    manifest_index = _load_manifest_index(ob_dir, file)

    # Pass 1: hash exact match
    hash_matched = 0
    matched_hashes: set[str] = set()
    changes: list[_ManifestChange] = []

    for _line_num, _text, line_hash in data_lines:
        if line_hash in manifest_index:
            hash_matched += 1
            matched_hashes.add(line_hash)
            del manifest_index[line_hash]

    unmatched_lines = [
        (ln, txt, lh) for ln, txt, lh in data_lines if lh not in matched_hashes
    ]

    # Pass 2: embedding semantic match
    semantic_matched = 0
    new_lines = 0

    if model is not None and unmatched_lines:
        existing_embeddings = _load_all_embeddings(ob_dir, model)
        unmatched_manifest_embs: dict[str, list[float]] = {
            lh: emb for lh, emb in existing_embeddings.items() if lh in manifest_index
        }

        texts = [text for (_, text, _) in unmatched_lines]
        if _encode_fn is not None:
            new_embs: list[list[float]] = _encode_fn(texts)
        elif embedding_api is not None:
            new_embs = make_api_encode_fn(embedding_api, model)(texts)
        else:
            new_embs = _get_encode_fn(model)(texts)

        _written_emb_hashes: set[str] = set()
        for (line_hash, _text), embedding in zip(
            [(lh, txt) for _, txt, lh in unmatched_lines], new_embs
        ):
            if line_hash not in _written_emb_hashes:
                shard_append(
                    ob_dir,
                    f"embeddings.{model}",
                    line_hash,
                    {"line_hash": line_hash, "embedding": embedding},
                )
                _written_emb_hashes.add(line_hash)

        if unmatched_manifest_embs:
            old_keys = list(unmatched_manifest_embs.keys())
            old_matrix = np.array([unmatched_manifest_embs[k] for k in old_keys])
            old_norms = np.linalg.norm(old_matrix, axis=1, keepdims=True)
            old_norms[old_norms == 0] = 1.0
            old_matrix = old_matrix / old_norms

            new_matrix = np.array(new_embs)
            new_norms = np.linalg.norm(new_matrix, axis=1, keepdims=True)
            new_norms[new_norms == 0] = 1.0
            new_matrix = new_matrix / new_norms

            sim_matrix = new_matrix @ old_matrix.T

        used_old: set[str] = set()
        best_old_per_new: dict[int, tuple[str, float]] = {}
        if unmatched_manifest_embs:
            for i in range(len(unmatched_lines)):
                row = sim_matrix[i].copy()
                for j, k in enumerate(old_keys):
                    if k in used_old:
                        row[j] = -2.0
                best_j = int(np.argmax(row))
                best_s = float(row[best_j])
                best_old_per_new[i] = (old_keys[best_j], best_s)

        for idx, (_line_num, _text, line_hash) in enumerate(unmatched_lines):
            if idx not in best_old_per_new:
                new_lines += 1
                continue

            best_lh, best_sim = best_old_per_new[idx]

            if best_sim >= threshold and best_lh and best_lh not in used_old:
                semantic_matched += 1
                used_old.add(best_lh)
                old_records = manifest_index.pop(best_lh, [])

                for old_rec in old_records:
                    remove_bucket = old_rec.get("line_hash", "")[:2].lower()
                    changes.append(
                        _ManifestChange(
                            action="remove", bucket=remove_bucket, record=old_rec
                        )
                    )
                    new_rec = {
                        "line_hash": line_hash,
                        "file": file,
                        "sources": old_rec.get("sources", []),
                        "source_type": "reconcile",
                        "revoked": False,
                    }
                    add_bucket = line_hash[:2].lower()
                    changes.append(
                        _ManifestChange(action="add", bucket=add_bucket, record=new_rec)
                    )
            else:
                new_lines += 1
    else:
        new_lines = len(unmatched_lines)

    orphan_hashes = list(manifest_index.keys())
    orphans = sum(len(v) for v in manifest_index.values())

    if changes:
        _apply_manifest_changes(ob_dir, changes)

    if manifest_index:
        _tag_orphans(ob_dir, manifest_index)

    if compute_all_embeddings and model is not None:
        _compute_all_embeddings(ob_dir, data_lines, model, _encode_fn, embedding_api)

    append_log(
        ob_dir,
        "reconcile",
        {
            "file": file,
            "hash_matched": hash_matched,
            "semantic_matched": semantic_matched,
            "unmatched": new_lines,
            "orphans": orphans,
            "threshold": threshold,
            "model": model,
        },
        f"ob reconcile {data_file}",
    )

    duration_ms = (time.monotonic() - t0) * 1000

    logger.info(
        "Reconcile complete for %s: %d hash, %d semantic, %d new, %d orphans (%.1fms)",
        file,
        hash_matched,
        semantic_matched,
        new_lines,
        orphans,
        duration_ms,
    )

    return ReconcileResult(
        hash_matched=hash_matched,
        semantic_matched=semantic_matched,
        new_lines=new_lines,
        orphans=orphans,
        errors=0,
        orphan_hashes=orphan_hashes,
        duration_ms=duration_ms,
    )
