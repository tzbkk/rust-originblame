from __future__ import annotations

from pathlib import Path

import ob
from ob.exceptions import OBInitError

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
    from _ob_native import (
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


def _resolve_author_ids(names: list[str], ob_dir: Path) -> list[str]:
    import json
    name_to_id: dict[str, str] = {}
    authors_root = ob_dir / ".ob" / "authors"
    if authors_root.exists():
        for f in authors_root.iterdir():
            if not f.is_file():
                continue
            for line in f.read_text(encoding="utf-8").splitlines():
                if not line.strip():
                    continue
                try:
                    d = json.loads(line)
                    name_to_id[d["name"]] = d["id"]
                except (json.JSONDecodeError, KeyError):
                    continue
    return [name_to_id[n] for n in names if n in name_to_id]


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
