"""Source stack module for tracking active provenance sources.

The source stack is a per-thread stack of source entries. Each entry maps
a file path to the section hashes registered for that file.
"""

from __future__ import annotations

import threading
from contextlib import contextmanager
from pathlib import Path

from ob.exceptions import OBSourceError
from ob.register import find_sections_by_path
from ob.register import find_sections_by_path_prefix as _find_sections_by_path_prefix
from ob.util import find_ob_dir

__all__ = [
    "Source",
    "append",
    "pop",
    "get_active_sources",
    "get_active_section_hashes",
    "sources",
    "find_sections_by_path_prefix",
]

# Thread-local storage for the source stack.
_local = threading.local()


def _get_stack() -> list[dict]:
    if not hasattr(_local, "stack"):
        _local.stack = []
    return _local.stack


def append(file: str, section: str | None = None, ob_dir: Path | None = None) -> None:
    """Append a source entry to the thread-local source stack.

    Looks up sections registered for *file* and pushes them onto the stack.
    If *section* is given, only the section with that exact hash is activated.

    Args:
        file: File path relative to the repo root (e.g. ``"raw/wiki.xml"``).
        section: Optional section hash to activate a single section.
        ob_dir: Repository root directory.  Discovered via ``find_ob_dir()``
            if not provided.

    Raises:
        OBSourceError: If no sections are found for *file*, or if *section*
            is specified but not found among the registered sections.
    """
    if ob_dir is None:
        ob_dir = find_ob_dir()

    section_records = find_sections_by_path(ob_dir, file)
    if not section_records:
        raise OBSourceError(
            f"No sections found for: {file}. "
            "Register a section first with 'ob register.add'."
        )

    if section is not None:
        filtered = [r for r in section_records if r["section_hash"] == section]
        if not filtered:
            raise OBSourceError(
                f"Section {section[:12]}… not found for file: {file}"
            )
        section_records = filtered

    section_hashes = [r["section_hash"] for r in section_records]
    _get_stack().append({"file": file, "sections": section_hashes})


def pop(file: str | None = None, ob_dir: Path | None = None) -> None:
    """Remove a source entry from the thread-local source stack.

    If *file* is specified, removes the first matching entry (may pop from
    the middle -- not strict LIFO).  If no *file* is given, pops the most
    recent entry (LIFO).

    Args:
        file: Optional file path to remove.  If ``None``, pops top of stack.
        ob_dir: Reserved / unused.

    Raises:
        OBSourceError: If the stack is empty, or if *file* is specified but
            not found in the stack.
    """
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


def get_active_sources() -> list[dict]:
    return list(_get_stack())


def get_active_section_hashes() -> set[str]:
    """Return the union of all section hashes across all active sources."""
    result: set[str] = set()
    for entry in _get_stack():
        result.update(entry["sections"])
    return result


@contextmanager
def sources(*files: str, ob_dir: Path | None = None):
    """Context manager that pushes source files on enter and pops on exit.

    Args:
        *files: One or more file paths to push onto the source stack.
        ob_dir: Repository root directory.  Discovered via ``find_ob_dir()``
            if not provided.

    Yields:
        Nothing (``None``).
    """
    for f in files:
        append(f, ob_dir=ob_dir)
    try:
        yield
    finally:
        # Pop in reverse order so that the last pushed is popped first.
        for f in reversed(files):
            pop(f)


class Source:
    """Backward-compatible source namespace for ``source.append(file)`` pattern."""
    append = staticmethod(append)
    pop = staticmethod(pop)
    get_active_sources = staticmethod(get_active_sources)
    get_active_section_hashes = staticmethod(get_active_section_hashes)


def find_sections_by_path_prefix(ob_dir: Path | None = None, path_prefix: str = "") -> list[dict]:
    if ob_dir is None:
        ob_dir = find_ob_dir()
    return _find_sections_by_path_prefix(ob_dir, path_prefix)
