from __future__ import annotations

import threading
from pathlib import Path

from ob.exceptions import OBTrackError, OBSourceError

__all__ = ["track", "source"]

_local = threading.local()


def _get_stack() -> list[dict]:
    if not hasattr(_local, "stack"):
        _local.stack = []
    return _local.stack


class _SourceCtx:
    @staticmethod
    def append(file: str, section: str | None = None, ob_dir: Path | None = None) -> None:
        if section is not None:
            raise OBSourceError("Section filtering not yet implemented")
        if ob_dir is None:
            ob_dir = Path.cwd()

        from ob._ob_native import author_query, register_section as _rs, shard_iterate_all

        section_records = shard_iterate_all(str(ob_dir), "sections")
        matching = [r for r in section_records if r.get("path") == file]
        if not matching:
            raise OBSourceError(f"No sections found for: {file}")

        section_hashes = [r["section_hash"] for r in matching]
        _get_stack().append({"file": file, "sections": section_hashes})

    @staticmethod
    def pop(file: str | None = None, ob_dir: Path | None = None) -> None:
        stack = _get_stack()
        if not stack:
            raise OBSourceError("Source stack is empty")
        if file is not None:
            for i, entry in enumerate(stack):
                if entry["file"] == file:
                    stack.pop(i)
                    return
            raise OBSourceError(f"No source entry found for: {file}")
        else:
            stack.pop()

    @staticmethod
    def get_active_sources() -> list[dict]:
        return list(_get_stack())

    @staticmethod
    def get_active_section_hashes() -> set[str]:
        result: set[str] = set()
        for entry in _get_stack():
            result.update(entry["sections"])
        return result


source = _SourceCtx


def track(
    data: dict | str,
    file: str,
    ob_dir: Path | None = None,
) -> str:
    if ob_dir is None:
        ob_dir = Path.cwd()

    if not _get_stack():
        raise OBTrackError("source stack is empty; call source.append() first")

    section_hashes = source.get_active_section_hashes()
    sources = sorted(section_hashes)

    from ob._ob_native import track as _native_track

    return _native_track(data, file, sources, str(ob_dir))
