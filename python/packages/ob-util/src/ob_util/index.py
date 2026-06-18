"""Index builder for OriginBlame provenance data."""

from __future__ import annotations

from collections import defaultdict
from pathlib import Path

from ob.storage import (
    LAYER_MANIFEST,
    LAYER_SECTION,
    jsonl_write,
    shard_iterate_all,
)

__all__ = ["build_index"]

LAYER_INDEX = "index"


def build_index(ob_dir: Path) -> dict[str, int]:
    """Scan all three layers and build bucket-routing index.

    Returns:
        Dict with counts: {"authors": N, "sections": N, "total": N}
    """
    section_to_manifest_buckets: dict[str, set[str]] = defaultdict(set)
    for rec in shard_iterate_all(ob_dir, LAYER_MANIFEST):
        sources = rec.get("sources")
        line_hash = rec.get("line_hash", "")
        if not sources or not line_hash:
            continue
        manifest_bucket = line_hash[:2].lower()
        for section_hash in sources:
            section_to_manifest_buckets[section_hash].add(manifest_bucket)

    author_to_section_buckets: dict[str, set[str]] = defaultdict(set)
    section_records: dict[str, list[str]] = {}
    author_ids: set[str] = set()
    section_ids: set[str] = set()

    for rec in shard_iterate_all(ob_dir, LAYER_SECTION):
        section_hash = rec.get("section_hash", "")
        if not section_hash:
            continue
        section_bucket = section_hash[:2].lower()
        manifest_refs = sorted(section_to_manifest_buckets.get(section_hash, set()))

        section_records[section_hash] = manifest_refs
        section_ids.add(section_hash)

        for author_id in rec.get("authors", []):
            author_to_section_buckets[author_id].add(section_bucket)
            author_ids.add(author_id)

    bucket_records: dict[str, list[dict]] = defaultdict(list)

    for author_id in author_ids:
        bucket = author_id[:2].lower()
        bucket_records[bucket].append(
            {
                "id": author_id,
                "refs": sorted(author_to_section_buckets[author_id]),
            }
        )

    for section_hash, refs in section_records.items():
        bucket = section_hash[:2].lower()
        bucket_records[bucket].append(
            {
                "id": section_hash,
                "refs": refs,
            }
        )

    index_dir = ob_dir / ".ob" / LAYER_INDEX
    if index_dir.exists():
        for bucket_file in index_dir.iterdir():
            if bucket_file.is_file():
                bucket_file.unlink()

    for bucket, records in bucket_records.items():
        jsonl_write(ob_dir / ".ob" / LAYER_INDEX / bucket, records)

    return {
        "authors": len(author_ids),
        "sections": len(section_ids),
        "total": len(author_ids) + len(section_ids),
    }
