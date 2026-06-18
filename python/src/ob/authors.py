"""Authors storage layer -- CRUD operations for author records.

Author record format: {"id": "<sha256 of name+email>", "name": "...", "email": "...", "revoked": false}
Layer: "authors" in .ob/authors/{bucket}
"""

from __future__ import annotations

from pathlib import Path

from ob.exceptions import OBAuthorError
from ob.oplog import append_log
from ob.storage import LAYER_AUTHORS, shard_append, shard_iterate_all, shard_read
from ob.util import compute_hash

__all__ = [
    "add_author",
    "get_author",
    "query_authors",
    "revoke_author",
    "list_all_authors",
    "invalidate_cache",
]

_LAYER = LAYER_AUTHORS
_MISSING = object()
_name_cache: dict[str, dict[str, set[str]]] = {}  # ob_dir_str -> {name: {author_ids}}
_email_cache: dict[str, dict[str, set[str]]] = {}  # ob_dir_str -> {email: {author_ids}}
_record_cache: dict[str, dict[str, dict]] = {}  # ob_dir_str -> {author_id: record}


def _cache_key(ob_dir: Path) -> str:
    return str(ob_dir.resolve())


def _ensure_cache(ob_dir: Path) -> None:
    key = _cache_key(ob_dir)
    if key not in _name_cache:
        _name_cache[key] = {}
        _email_cache[key] = {}


def _cache_author(ob_dir: Path, name: str, email: str, author_id: str) -> None:
    _ensure_cache(ob_dir)
    key = _cache_key(ob_dir)
    _name_cache[key].setdefault(name, set()).add(author_id)
    _email_cache[key].setdefault(email, set()).add(author_id)


def invalidate_cache(ob_dir: Path | None = None) -> None:
    if ob_dir is None:
        _name_cache.clear()
        _email_cache.clear()
        _record_cache.clear()
    else:
        key = _cache_key(ob_dir)
        _name_cache.pop(key, None)
        _email_cache.pop(key, None)
        _record_cache.pop(key, None)


def add_author(ob_dir: Path, name: str, email: str) -> str:
    """Register an author. Idempotent -- returns existing id if already registered.

    Raises OBAuthorError if name or email is empty.
    """
    if not name:
        raise OBAuthorError("Author name must not be empty")
    if not email:
        raise OBAuthorError("Author email must not be empty")

    author_id = compute_hash(name + email)

    key = _cache_key(ob_dir)
    rec_cache = _record_cache.get(key)
    if rec_cache is not None and rec_cache.get(author_id) is not None:
        return author_id

    record = {
        "id": author_id,
        "name": name,
        "email": email,
        "revoked": False,
    }
    shard_append(ob_dir, _LAYER, author_id, record)
    _cache_author(ob_dir, name, email, author_id)
    _record_cache.setdefault(key, {})[author_id] = record
    return author_id


def get_author(ob_dir: Path, author_id: str) -> dict | None:
    key = _cache_key(ob_dir)
    rec_cache = _record_cache.get(key)
    if rec_cache is not None:
        hit = rec_cache.get(author_id)
        if hit is not None:
            return hit if hit is not _MISSING else None

    bucket = author_id[:2].lower()
    records = shard_read(ob_dir, _LAYER, author_id)
    rec_map: dict[str, dict] = {}
    found = None
    for record in records:
        rid = record.get("id")
        if rid:
            rec_map[rid] = record
            if rid == author_id:
                found = record
    _record_cache.setdefault(key, {}).update(rec_map)
    if found is None:
        _record_cache.setdefault(key, {})[author_id] = _MISSING
    return found


def query_authors(
    ob_dir: Path, name: str | None = None, email: str | None = None
) -> list[dict]:
    _ensure_cache(ob_dir)
    key = _cache_key(ob_dir)

    if name is not None:
        ids = _name_cache.get(key, {}).get(name)
        if ids is not None:
            results = []
            for aid in ids:
                if email is not None:
                    rec = get_author(ob_dir, aid)
                    if rec and rec.get("email") == email:
                        results.append(rec)
                else:
                    rec = get_author(ob_dir, aid)
                    if rec:
                        results.append(rec)
            return results

    if email is not None:
        ids = _email_cache.get(key, {}).get(email)
        if ids is not None:
            results = []
            for aid in ids:
                if name is not None:
                    rec = get_author(ob_dir, aid)
                    if rec and rec.get("name") == name:
                        results.append(rec)
                else:
                    rec = get_author(ob_dir, aid)
                    if rec:
                        results.append(rec)
            return results

    results: list[dict] = []
    for record in shard_iterate_all(ob_dir, _LAYER):
        if name is not None and record.get("name") != name:
            continue
        if email is not None and record.get("email") != email:
            continue
        results.append(record)
        _cache_author(
            ob_dir, record.get("name", ""), record.get("email", ""), record["id"]
        )
    return results


def revoke_author(
    ob_dir: Path, email: str | None = None, author_id: str | None = None
) -> int:
    """Tag-only revoke -- sets revoked=True. No cascade. Raises OBAuthorError if not found.

    Exactly one of email or author_id must be provided.
    """
    if email is None and author_id is None:
        raise OBAuthorError("Must provide either email or author_id to revoke")
    if email is not None and author_id is not None:
        raise OBAuthorError("Cannot specify both email and author_id")

    if author_id is not None:
        record = get_author(ob_dir, author_id)
        if record is None:
            raise OBAuthorError(f"Author not found: {author_id}")
        record["revoked"] = True
        _rewrite_shard(ob_dir, author_id, record)
        append_log(
            ob_dir,
            "revoke_author",
            {"author": record["name"], "email": record["email"]},
            "",
        )
        return 1
    else:
        matches = query_authors(ob_dir, email=email)
        if not matches:
            raise OBAuthorError(f"Author not found with email: {email}")
        count = 0
        for match in matches:
            match["revoked"] = True
            _rewrite_shard(ob_dir, match["id"], match)
            append_log(
                ob_dir,
                "revoke_author",
                {"author": match["name"], "email": match["email"]},
                "",
            )
            count += 1
        return count


def restore_author(
    ob_dir: Path, email: str | None = None, author_id: str | None = None
) -> int:
    if email is None and author_id is None:
        raise OBAuthorError("Must provide either email or author_id to restore")
    if email is not None and author_id is not None:
        raise OBAuthorError("Cannot specify both email and author_id")

    if author_id is not None:
        record = get_author(ob_dir, author_id)
        if record is None:
            raise OBAuthorError(f"Author not found: {author_id}")
        if not record.get("revoked"):
            return 0
        record["revoked"] = False
        _rewrite_shard(ob_dir, author_id, record)
        append_log(ob_dir, "restore_author", {"author": record["name"], "email": record["email"]}, "")
        return 1
    else:
        matches = query_authors(ob_dir, email=email)
        if not matches:
            raise OBAuthorError(f"Author not found with email: {email}")
        count = 0
        for match in matches:
            if match.get("revoked"):
                match["revoked"] = False
                _rewrite_shard(ob_dir, match["id"], match)
                append_log(ob_dir, "restore_author", {"author": match["name"], "email": match["email"]}, "")
                count += 1
        return count


def list_all_authors(ob_dir: Path) -> list[dict]:
    return list(shard_iterate_all(ob_dir, _LAYER))


def _rewrite_shard(ob_dir: Path, author_id: str, updated_record: dict) -> None:
    from ob.storage import bucket_path, jsonl_read, jsonl_write

    shard_file = bucket_path(ob_dir, _LAYER, author_id)
    records = jsonl_read(shard_file)

    new_records = []
    found = False
    for record in records:
        if record.get("id") == author_id and not found:
            new_records.append(updated_record)
            found = True
        else:
            new_records.append(record)

    if not found:
        new_records.append(updated_record)

    jsonl_write(shard_file, new_records)
