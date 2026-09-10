from __future__ import annotations

import json
from pathlib import Path

import ob
from ob.exceptions import OBInitError, OBSectionError

__all__ = ["init", "author_add", "register_section"]

_OB_SUBDIRS = [
    "document-index",
    "sections",
    "authors",
    "embeddings",
    "backup",
    "archive",
    "split",
]

_OB_GITIGNORE_CONTENT = "*.pid\nlock.*\nbackup/*\narchive/*\n"

try:
    from ob._ob_native import (
        init as _native_init,
        author_add as _native_author_add,
        register_section as _native_register_section,
        oplog_append as _native_oplog_append,
    )
    _NATIVE = True
except ImportError:
    _NATIVE = False


def init(force: bool = False, ob_dir: Path | str | None = None) -> None:
    """Initialize a `.ob/` provenance tracking directory.

    Args:
        force: If True, re-initialize even if `.ob/` exists with a valid version.
        ob_dir: Repository root directory. Defaults to cwd if None.

    Raises:
        OBInitError: If `.ob/` exists but lacks the `ob-version` file
            (invalid state) and force is False.
    """
    if ob_dir is not None and not isinstance(ob_dir, Path):
        ob_dir = Path(ob_dir)
    if ob_dir is None:
        ob_dir = Path.cwd()

    ob_path = ob_dir / ".ob"

    if ob_path.exists():
        version_file = ob_path / "ob-version"
        if version_file.exists():
            if force:
                version_file.write_text(ob.__version__)
                _ensure_subdirs(ob_path)
            return
        if not force:
            raise OBInitError(f".ob/ exists but is invalid (no ob-version) in {ob_dir}")

    if _NATIVE:
        _native_init(str(ob_dir))
    else:
        try:
            from ob.rust import init as _rust_init
            _rust_init(str(ob_dir))
        except (ImportError, FileNotFoundError, RuntimeError):
            ob_path.mkdir(parents=True, exist_ok=True)
            _ensure_subdirs(ob_path)

    version_file = ob_path / "ob-version"
    if not version_file.exists() or force:
        version_file.write_text(ob.__version__)

    gitignore = ob_path / ".gitignore"
    if not gitignore.exists():
        gitignore.write_text(_OB_GITIGNORE_CONTENT)

    _update_root_gitignore(Path(str(ob_dir)))

    if _NATIVE:
        _native_oplog_append(str(ob_dir), "init", f"force={force} version={ob.__version__}")


def _ensure_subdirs(ob_path: Path) -> None:
    for d in _OB_SUBDIRS:
        (ob_path / d).mkdir(exist_ok=True)


def _update_root_gitignore(ob_dir: Path) -> None:
    gitignore = ob_dir / ".gitignore"
    if gitignore.exists():
        content = gitignore.read_text()
        if ".ob/" not in content:
            with open(gitignore, "a", encoding="utf-8") as f:
                f.write("\n.ob/\n")


def author_add(name: str, email: str, ob_dir: Path | str | None = None) -> str:
    """Register an author and return their author_id.

    Args:
        name: Author display name.
        email: Author email.
        ob_dir: Repository root directory. Defaults to cwd if None.

    Returns:
        The 64-character SHA-256 author_id computed from name and email.
    """
    if ob_dir is not None and not isinstance(ob_dir, Path):
        ob_dir = Path(ob_dir)
    if ob_dir is None:
        ob_dir = Path.cwd()

    if _NATIVE:
        return _native_author_add(name, email, str(ob_dir))

    from ob.rust import author_add as _rust_author_add
    return _rust_author_add(name, email, str(ob_dir))


_author_state: dict[str, dict] = {}
# ob_dir → {"shards": {分片名: (签名, {name: {id}})}, "merged": {name: {id}}}
# 解析按分片增量刷新:stat 对比签名,只有变化的分片才读盘,merged 原地增减


def _parse_author_shard(path: Path) -> dict[str, set[str]]:
    names: dict[str, set[str]] = {}
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            try:
                d = json.loads(line)
            except json.JSONDecodeError:
                continue
            nid, nm = d.get("id"), d.get("name")
            if nid and nm:
                names.setdefault(nm, set()).add(nid)
    except OSError:
        pass
    return names


def _subtract(merged: dict[str, set[str]], shard_names: dict[str, set[str]]) -> None:
    for nm, ids in shard_names.items():
        left = merged.get(nm)
        if left is None:
            continue
        left -= ids
        if not left:
            del merged[nm]


def _resolve_author_ids(names: list[str], ob_dir: Path) -> list[str]:
    """Resolve author names to author_ids via a per-shard incremental cache.

    The pre-0.2.2 implementation re-read and re-parsed the whole authors
    shard layer on *every* call -- O(total authors) per resolve and
    O(sections x authors) over a build. Now each shard carries a
    (size, mtime_ns) signature; a resolve call stats the shards and
    re-reads only the ones that changed, so both read-mostly consumers
    and interleaved append-then-register writers stay cheap.

    Raises:
        OBSectionError: If any name cannot be resolved (previously such
            names were silently dropped from the section record).
    """
    key = str(ob_dir)
    st = _author_state.get(key)
    if st is None:
        st = _author_state[key] = {"shards": {}, "merged": {}}
    shards, merged = st["shards"], st["merged"]

    root = ob_dir / ".ob" / "authors"
    try:
        files = sorted(root.iterdir())
    except OSError:
        files = []
    live: set[str] = set()
    for f in files:
        try:
            if not f.is_file():
                continue
            fstat = f.stat()
        except OSError:
            continue
        live.add(f.name)
        sig = (fstat.st_size, fstat.st_mtime_ns)
        cached = shards.get(f.name)
        if cached is not None and cached[0] == sig:
            continue
        if cached is not None:
            _subtract(merged, cached[1])
        new_names = _parse_author_shard(f)
        for nm, ids in new_names.items():
            merged.setdefault(nm, set()).update(ids)
        shards[f.name] = (sig, new_names)

    for gone in set(shards) - live:
        _subtract(merged, shards.pop(gone)[1])

    resolved: list[str] = []
    missing: list[str] = []
    for n in names:
        ids = merged.get(n)
        if ids:
            resolved.append(sorted(ids)[0])
        else:
            missing.append(n)
    if missing:
        preview = ", ".join(missing[:5])
        raise OBSectionError(f"unresolvable author name(s): {preview}")
    return resolved


def register_section(
    path: str,
    authors: list[str],
    license: str,
    year: str,
    contributors: list[str] | None = None,
    ob_dir: Path | str | None = None,
) -> str:
    """Register a section (provenance metadata for a file path) and return its section_hash.

    Args:
        path: File path, e.g. "raw/wiki.xml".
        authors: List of author names/emails to resolve to author_ids.
        license: SPDX identifier, e.g. "CC-BY-SA-4.0".
        year: Year string, e.g. "2024".
        contributors: Optional list of contributor names/emails.
        ob_dir: Repository root directory. Defaults to cwd if None.

    Returns:
        The 64-character SHA-256 section_hash.

    Raises:
        OBSectionError: If any author/contributor cannot be resolved
            (delegated to register.register_section).
    """
    if contributors is None:
        contributors = []
    if ob_dir is not None and not isinstance(ob_dir, Path):
        ob_dir = Path(ob_dir)
    if ob_dir is None:
        ob_dir = Path.cwd()

    author_ids = _resolve_author_ids(authors, ob_dir)
    contributor_ids = _resolve_author_ids(contributors, ob_dir)

    if _NATIVE:
        return _native_register_section(path, author_ids, contributor_ids, license, year, str(ob_dir))

    from ob.rust import register_section as _rust_register_section
    return _rust_register_section(path, author_ids, contributor_ids, license, year, str(ob_dir))
