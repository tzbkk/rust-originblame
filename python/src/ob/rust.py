"""Rust binary backend for ob operations.

Delegates performance-critical operations to the native Rust binary
while preserving the Python API surface.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from pathlib import Path

__all__ = [
    "find_ob_binary",
    "run_ob",
    "init",
    "author_add",
    "register_section",
    "blame",
    "show",
    "revoke",
    "purge",
    "merge_absorb",
    "index_build",
]


def _is_rust_binary(path: str) -> bool:
    """Check if *path* is the native Rust binary, not the Python CLI wrapper."""
    try:
        result = subprocess.run(
            [path, "version"],
            capture_output=True, text=True, timeout=5,
        )
        return result.returncode == 0 and "originblame" in result.stdout.lower()
    except Exception:
        return False


def find_ob_binary() -> str:
    """Locate the Rust ob binary.

    Search order:
      1. ``OB_RUST_BIN`` environment variable
      2. ``rust-originblame/target/release/ob`` relative to project root
      3. ``~/.local/bin/ob``
      4. ``PATH`` lookup via :func:`shutil.which`

    Each candidate is verified via ``ob version`` to distinguish from the
    Python CLI wrapper.

    Returns:
        Absolute path to the binary.

    Raises:
        FileNotFoundError: If the binary cannot be found.
    """
    # 1. Explicit environment override
    env_bin = os.environ.get("OB_RUST_BIN")
    if env_bin:
        return env_bin

    # 2. Relative to the Python package (inside rust-originblame repo)
    repo_root = Path(__file__).resolve().parent.parent.parent.parent
    candidate = repo_root / "target" / "release" / "ob"
    if candidate.exists() and os.access(candidate, os.X_OK) and _is_rust_binary(str(candidate)):
        return str(candidate)

    # 3. User-local install
    candidate = Path.home() / ".local" / "bin" / "ob"
    if candidate.exists() and os.access(candidate, os.X_OK) and _is_rust_binary(str(candidate)):
        return str(candidate)

    # 4. PATH lookup
    found = shutil.which("ob")
    if found and _is_rust_binary(found):
        return found

    raise FileNotFoundError(
        "Rust ob binary not found. Install rust-originblame or set OB_RUST_BIN."
    )


def run_ob(
    args: list[str],
    *,
    capture: bool = True,
    timeout: int = 300,
) -> subprocess.CompletedProcess[str]:
    """Execute the Rust ob binary with *args*.

    Parameters:
        args: Arguments forwarded after the binary name.
        capture: If ``True`` (default) capture stdout/stderr.
        timeout: Maximum seconds before killing the subprocess.

    Returns:
        The completed-process result.
    """
    bin_path = find_ob_binary()
    cmd = [bin_path] + args
    return subprocess.run(
        cmd,
        capture_output=capture,
        text=True,
        timeout=timeout,
    )


# ---------------------------------------------------------------------------
# High-level wrappers
# ---------------------------------------------------------------------------


def _extract_hash(output: str) -> str:
    m = re.search(r"\(([0-9a-f]{8})\)", output)
    if m:
        return m.group(1)
    m = re.search(r":\s*([0-9a-f]{8})", output)
    if m:
        return m.group(1)
    return output.strip()


def _compute_author_hash(name: str, email: str) -> str:
    import hashlib

    data = {"email": email, "name": name}
    payload = json.dumps(
        data, sort_keys=True, ensure_ascii=False, separators=(",", ":")
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def init(ob_dir: str | Path = ".") -> None:
    """Delegate ``ob init`` to the Rust binary."""
    res = run_ob(["init", str(ob_dir)])
    if res.returncode != 0:
        raise RuntimeError(
            res.stderr.strip() or f"ob init failed (rc={res.returncode})"
        )


def author_add(name: str, email: str, ob_dir: str | Path = ".") -> str:
    """Delegate ``ob author.add`` to the Rust binary.

    Returns:
        The full author_id (64-char SHA-256 hex).
    """
    author_id = _compute_author_hash(name, email)
    res = run_ob(["author.add", name, email, "-d", str(ob_dir)])
    if res.returncode != 0:
        raise RuntimeError(
            res.stderr.strip() or f"ob author.add failed (rc={res.returncode})"
        )
    return author_id


def register_section(
    path: str,
    authors: list[str],
    license: str,
    year: str,
    ob_dir: str | Path = ".",
) -> str:
    """Delegate ``ob register.add`` to the Rust binary.

    Authors may be author IDs (64-char hex) or names/emails. IDs are
    resolved to names by reading the authors shard before calling the binary.
    Returns:
        The full section_hash (64-char SHA-256 hex).
    """
    import hashlib

    ob_path = Path(str(ob_dir))
    resolved = _resolve_author_ids(authors, ob_path)

    args = [
        "register.add",
        "--path", path,
        "--authors", ",".join(resolved),
        "--license", license,
        "--year", year,
        "-d", str(ob_dir),
    ]
    res = run_ob(args)
    if res.returncode != 0:
        raise RuntimeError(
            res.stderr.strip() or f"ob register.add failed (rc={res.returncode})"
        )

    data = {"authors": authors, "license": license, "path": path, "year": year}
    payload = json.dumps(
        data, sort_keys=True, ensure_ascii=False, separators=(",", ":")
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def _resolve_author_ids(authors: list[str], ob_dir: Path) -> list[str]:
    """Resolve author IDs to author names by reading author shards."""
    import re

    resolved = []
    hex_re = re.compile(r"^[0-9a-f]{64}$")
    authors_dir = ob_dir / ".ob" / "authors"

    for a in authors:
        if not hex_re.match(a) or not (authors_dir / a[:2]).exists():
            resolved.append(a)
            continue

        try:
            with open(authors_dir / a[:2], "r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if a in line:
                        try:
                            entry = json.loads(line)
                            if entry.get("id") == a:
                                resolved.append(entry["name"])
                                break
                        except json.JSONDecodeError:
                            continue
                else:
                    resolved.append(a)
        except FileNotFoundError:
            resolved.append(a)

    return resolved


def blame(file: str | Path, line_num: int, ob_dir: str | Path = ".") -> str:
    """Delegate ``ob blame`` to the Rust binary.

    Returns:
        Raw stdout from the Rust binary (formatted provenance text).
    """
    res = run_ob(["blame", str(file), str(line_num), "-d", str(ob_dir)])
    if res.returncode != 0:
        raise RuntimeError(
            res.stderr.strip() or f"ob blame failed (rc={res.returncode})"
        )
    return res.stdout.strip()


def show(
    *,
    author: str | None = None,
    index: bool = False,
    ob_dir: str | Path = ".",
) -> str:
    """Delegate ``ob show`` to the Rust binary.

    Returns:
        Raw stdout from the Rust binary.
    """
    args = ["show", "-d", str(ob_dir)]
    if author:
        args.extend(["--author", author])
    if index:
        args.append("--index")
    res = run_ob(args)
    if res.returncode != 0:
        raise RuntimeError(
            res.stderr.strip() or f"ob show failed (rc={res.returncode})"
        )
    return res.stdout.strip()


def revoke(
    *,
    author: str | None = None,
    ob_dir: str | Path = ".",
) -> str:
    """Delegate ``ob revoke`` to the Rust binary.

    Returns:
        Raw stdout from the Rust binary.
    """
    args = ["revoke", "-d", str(ob_dir)]
    if author:
        args.extend(["--author", author])
    res = run_ob(args)
    if res.returncode != 0:
        raise RuntimeError(
            res.stderr.strip() or f"ob revoke failed (rc={res.returncode})"
        )
    return res.stdout.strip()


def purge(
    file: str | Path,
    *,
    author: str | None = None,
    index: bool = False,
    dry_run: bool = False,
    ob_dir: str | Path = ".",
) -> str:
    """Delegate ``ob purge`` to the Rust binary.

    Returns:
        Raw stdout from the Rust binary.
    """
    args = ["purge", str(file), "-d", str(ob_dir)]
    if author:
        args.extend(["--author", author])
    if index:
        args.append("--index")
    if dry_run:
        args.append("--dry-run")
    res = run_ob(args)
    if res.returncode != 0:
        raise RuntimeError(
            res.stderr.strip() or f"ob purge failed (rc={res.returncode})"
        )
    return res.stdout.strip()


def index_build(ob_dir: str | Path = ".") -> str:
    """Delegate ``ob index build`` to the Rust binary.

    Returns:
        Raw stdout from the Rust binary.
    """
    res = run_ob(["index", "build", "-d", str(ob_dir)])
    if res.returncode != 0:
        raise RuntimeError(
            res.stderr.strip() or f"ob index build failed (rc={res.returncode})"
        )
    return res.stdout.strip()


def merge_absorb(
    source: str | Path,
    ob_dir: str | Path = ".",
) -> dict:
    """Delegate ``ob merge --absorb`` to the Rust binary via PyO3.

    Returns:
        Dict with keys: authors_added, sections_added, document_added,
        token_index_added, skipped.
    """
    from ob._ob_native import merge_absorb as _native_merge

    return _native_merge(str(source), str(ob_dir))
