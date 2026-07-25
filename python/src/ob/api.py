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
    if ob_dir is not None and not isinstance(ob_dir, Path):
        ob_dir = Path(ob_dir)
    if ob_dir is None:
        ob_dir = Path.cwd()

    if _NATIVE:
        return _native_author_add(name, email, str(ob_dir))

    from ob.rust import author_add as _rust_author_add
    return _rust_author_add(name, email, str(ob_dir))


def register_section(
    path: str,
    authors: list[str],
    license: str,
    year: str,
    contributors: list[str] | None = None,
    ob_dir: Path | str | None = None,
) -> str:
    if contributors is None:
        contributors = []
    if ob_dir is not None and not isinstance(ob_dir, Path):
        ob_dir = Path(ob_dir)
    if ob_dir is None:
        ob_dir = Path.cwd()

    if _NATIVE:
        return _native_register_section(path, authors, contributors, license, year, str(ob_dir))

    from ob.rust import register_section as _rust_register_section
    return _rust_register_section(path, authors, contributors, license, year, str(ob_dir))
