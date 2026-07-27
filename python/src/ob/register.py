"""Section storage layer with CRUD operations for section records."""

from __future__ import annotations

import hashlib
import json
from collections import defaultdict
from pathlib import Path

from ob.exceptions import OBSectionError
from ob.oplog import append_log
from ob.storage import (
    LAYER_SECTION,
    jsonl_read,
    jsonl_write,
    shard_append,
    shard_iterate_all,
    shard_read,
)

__all__ = [
    "register_section",
    "get_section",
    "query_sections",
    "revoke_section",
    "find_sections_by_path",
    "find_sections_by_path_prefix",
    "invalidate_section_cache",
]

# --- path cache: ob_dir_str -> {path: [section_records]} ---
_path_cache: dict[str, dict[str, list[dict]]] = {}
_record_cache: dict[str, dict[str, dict]] = {}  # ob_dir_str -> {section_hash: record}
_MISSING = object()


def _scache_key(ob_dir: Path) -> str:
    return str(ob_dir.resolve())


def _ensure_scache(ob_dir: Path) -> None:
    key = _scache_key(ob_dir)
    if key not in _path_cache:
        _path_cache[key] = {}


def _scache_put(ob_dir: Path, record: dict) -> None:
    _ensure_scache(ob_dir)
    key = _scache_key(ob_dir)
    p = record.get("path", "")
    if p:
        _path_cache[key].setdefault(p, []).append(record)


def invalidate_section_cache(ob_dir: Path | None = None) -> None:
    if ob_dir is None:
        _path_cache.clear()
        _record_cache.clear()
    else:
        key = _scache_key(ob_dir)
        _path_cache.pop(key, None)
        _record_cache.pop(key, None)


def _resolve_author_ids(ob_dir: Path, authors: list[str]) -> list[str]:
    """Resolve author names/emails/ids to author_ids.

    Tries three strategies: 1) 64-char hex ID verification, 2) name lookup, 3) email lookup.
    """
    from ob.authors import get_author, query_authors

    ids = []
    for a in authors:
        author_id = None

        if len(a) == 64 and all(c in "0123456789abcdef" for c in a.lower()):
            author = get_author(ob_dir, a)
            if author is not None:
                author_id = author["id"]

        if author_id is None:
            results = query_authors(ob_dir, name=a)
            if results:
                author_id = results[0]["id"]

        if author_id is None:
            results = query_authors(ob_dir, email=a)
            if results:
                author_id = results[0]["id"]

        if author_id is None:
            raise OBSectionError(f"Author not found: {a}")

        ids.append(author_id)

    return ids


def _compute_section_hash(
    path: str, author_ids: list[str], license: str, year: str
) -> str:
    """Compute section_hash = sha256(JSON(path, sorted(authors), license, year))."""
    data = {
        "path": path,
        "authors": sorted(author_ids),
        "license": license,
        "year": year,
    }
    return hashlib.sha256(
        json.dumps(data, sort_keys=True, ensure_ascii=False).encode("utf-8")
    ).hexdigest()


def register_section(
    ob_dir: Path,
    path: str,
    authors: list[str],
    license: str,
    year: str,
    chain_manifest: str | None = None,
) -> str:
    """Add a section record. Idempotent -- same hash -> skip and return.

    Resolves each entry in *authors* (name, email, or author_id) to an
    author_id by scanning the authors shard layer.

    Args:
        ob_dir: Repository root directory containing ``.ob/``.
        path: Section path, e.g. ``"raw/wiki.xml"``.
        authors: Author names, emails, or author_ids to resolve.
        license: License identifier, e.g. ``"CC-BY-SA-4.0"``.
        year: Year string, e.g. ``"2024"``.
        chain_manifest: Optional path to a prior manifest JSONL file.
            When provided, ``sources[]`` from every record in the file
            are merged (unioned) into the new section's ``sources``
            field, enabling multi-stage provenance chaining.

    Returns:
        The computed section_hash (64-char SHA-256 hex).

    Raises:
        OBSectionError: If any author cannot be resolved.
    """
    author_ids = _resolve_author_ids(ob_dir, authors)
    section_hash = _compute_section_hash(path, author_ids, license, year)

    # Idempotent: skip if the exact record already exists (cache-only check)
    key = _scache_key(ob_dir)
    rec_cache = _record_cache.get(key)
    if rec_cache is not None and rec_cache.get(section_hash) is not None:
        return section_hash

    record = {
        "section_hash": section_hash,
        "path": path,
        "authors": sorted(author_ids),
        "license": license,
        "year": year,
        "revoked": False,
    }

    if chain_manifest is not None:
        prior_path = Path(chain_manifest)
        if prior_path.exists():
            prior_records = jsonl_read(prior_path)
            chained_sources: list[str] = []
            seen: set[str] = set()
            for rec in prior_records:
                for src in rec.get("sources", []):
                    if src not in seen:
                        seen.add(src)
                        chained_sources.append(src)
            if chained_sources:
                record["sources"] = chained_sources

    shard_append(ob_dir, LAYER_SECTION, section_hash, record)
    _scache_put(ob_dir, record)
    _record_cache.setdefault(_scache_key(ob_dir), {})[section_hash] = record
    return section_hash


def get_section(ob_dir: Path, section_hash: str) -> dict | None:
    """Get a section record by its hash.

    Returns:
        The matching section record, or ``None`` if not found.
    """
    key = _scache_key(ob_dir)
    rec_cache = _record_cache.get(key)
    if rec_cache is not None:
        hit = rec_cache.get(section_hash)
        if hit is not None:
            return hit if hit is not _MISSING else None

    records = shard_read(ob_dir, LAYER_SECTION, section_hash)
    rec_map: dict[str, dict] = {}
    found = None
    for record in records:
        sh = record.get("section_hash")
        if sh:
            rec_map[sh] = record
            if sh == section_hash:
                found = record
    _record_cache.setdefault(key, {}).update(rec_map)
    if found is None:
        _record_cache.setdefault(key, {})[section_hash] = _MISSING
    return found


def query_sections(
    ob_dir: Path,
    path: str | None = None,
    license: str | None = None,
    author_id: str | None = None,
) -> list[dict]:
    results: list[dict] = []

    if path is not None:
        _ensure_scache(ob_dir)
        cached = _path_cache.get(_scache_key(ob_dir), {}).get(path)
        if cached is not None:
            for record in cached:
                if license is not None and record.get("license") != license:
                    continue
                if author_id is not None and author_id not in record.get("authors", []):
                    continue
                results.append(record)
            return results

    for record in shard_iterate_all(ob_dir, LAYER_SECTION):
        if path is not None and record.get("path") != path:
            continue
        if license is not None and record.get("license") != license:
            continue
        if author_id is not None and author_id not in record.get("authors", []):
            continue
        results.append(record)
        _scache_put(ob_dir, record)
    return results


def revoke_section(
    ob_dir: Path,
    section_hash: str | None = None,
    path: str | None = None,
) -> int:
    """Revoke sections by hash or path (tag-based: sets ``revoked=True``).

    Args:
        ob_dir: Repository root directory containing ``.ob/``.
        section_hash: Revoke the section with this exact hash.
        path: Revoke **all** sections matching this path.

    Returns:
        Count of newly revoked records (already-revoked ones are skipped).

    Raises:
        OBSectionError: If neither *section_hash* nor *path* is provided.
    """
    if section_hash is not None:
        section = get_section(ob_dir, section_hash)
        if section is None:
            return 0
        targets = [section]
    elif path is not None:
        targets = find_sections_by_path(ob_dir, path)
    else:
        raise OBSectionError("Must provide either section_hash or path to revoke")

    if not targets:
        return 0

    # Group non-revoked targets by shard bucket
    by_shard: dict[str, set[str]] = defaultdict(set)
    for t in targets:
        if not t.get("revoked"):
            by_shard[t["section_hash"][:2]].add(t["section_hash"])

    if not by_shard:
        return 0

    count = 0
    for bucket, hashes in by_shard.items():
        shard_file = ob_dir / ".ob" / LAYER_SECTION / bucket
        records = jsonl_read(shard_file)
        updated: list[dict] = []
        for r in records:
            if r.get("section_hash") in hashes and not r.get("revoked"):
                r["revoked"] = True
                count += 1
            updated.append(r)
        jsonl_write(shard_file, updated)

    _record_cache.pop(_scache_key(ob_dir), None)
    append_log(
        ob_dir,
        "revoke_section",
        {"count": count, "sections": [h[:16] for bucket_hashes in by_shard.values() for h in bucket_hashes]},
        "",
    )
    return count


def restore_section(
    ob_dir: Path,
    section_hash: str | None = None,
    path: str | None = None,
) -> int:
    if section_hash is not None:
        section = get_section(ob_dir, section_hash)
        if section is None:
            return 0
        targets = [section]
    elif path is not None:
        targets = find_sections_by_path(ob_dir, path)
    else:
        raise OBSectionError("Must provide either section_hash or path to restore")

    if not targets:
        return 0

    by_shard: dict[str, set[str]] = defaultdict(set)
    for t in targets:
        if t.get("revoked"):
            by_shard[t["section_hash"][:2]].add(t["section_hash"])

    if not by_shard:
        return 0

    count = 0
    for bucket, hashes in by_shard.items():
        shard_file = ob_dir / ".ob" / LAYER_SECTION / bucket
        records = jsonl_read(shard_file)
        updated: list[dict] = []
        for r in records:
            if r.get("section_hash") in hashes and r.get("revoked"):
                r["revoked"] = False
                count += 1
            updated.append(r)
        jsonl_write(shard_file, updated)

    _record_cache.pop(_scache_key(ob_dir), None)
    append_log(
        ob_dir,
        "restore_section",
        {"count": count, "sections": [h[:16] for bucket_hashes in by_shard.values() for h in bucket_hashes]},
        "",
    )
    return count


def find_sections_by_path(ob_dir: Path, path: str) -> list[dict]:
    """Find all section records matching *path*.

    Used by ``source.append()`` to look up section metadata.

    Returns:
        List of matching section records (may be empty).
    """
    return query_sections(ob_dir, path=path)


def find_sections_by_path_prefix(ob_dir: Path, path_prefix: str) -> list[dict]:
    """Find all sections whose path starts with *path_prefix*.

    E.g., ``'raw/北京'`` matches ``'raw/北京#历史'``, ``'raw/北京#地理'``, etc.
    """
    results: list[dict] = []
    for record in shard_iterate_all(ob_dir, LAYER_SECTION):
        if record.get("path", "").startswith(path_prefix):
            results.append(record)
            _scache_put(ob_dir, record)
    return results
