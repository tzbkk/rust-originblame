"""Core utilities for the ob package."""

from __future__ import annotations

import hashlib
import json
import urllib.parse
from pathlib import Path

from ob.exceptions import OBNotInitializedError


__all__ = [
    "compute_hash",
    "bucket_path",
    "shard_path",
    "find_ob_dir",
    "normalize_file",
    "url_encode",
]


def compute_hash(data: dict | str) -> str:
    """Compute SHA-256 hex digest for dict or str data.

    Args:
        data: Dictionary or string to hash.

    Returns:
        SHA-256 hex digest (64 characters).

    Raises:
        TypeError: If data is not a dict or str.

    Examples:
        >>> compute_hash({"key": "value"})
        'a591a6d40bf420404a011733cfb7b190d62c65bf0bcda32b57b277d9ad9f146e'
        >>> compute_hash("hello")
        '2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824'
    """
    if isinstance(data, dict):
        content = json.dumps(data, ensure_ascii=False, sort_keys=True, separators=(',', ':'))
        return hashlib.sha256(content.encode("utf-8")).hexdigest()
    elif isinstance(data, str):
        return hashlib.sha256(data.encode("utf-8")).hexdigest()
    else:
        raise TypeError(f"compute_hash requires dict or str, got {type(data).__name__}")


def bucket_path(hash_hex: str) -> str:
    """Extract first 2 hex characters from hash to form bucket path.

    Args:
        hash_hex: 64-character SHA-256 hex digest.

    Returns:
        2-character hex string representing bucket ("00" to "ff").

    Raises:
        ValueError: If hash_hex is not exactly 64 hex characters.

    Examples:
        >>> bucket_path("a591a6d40bf420404a011733cfb7b190d62c65bf0bcda32b57b277d9ad9f146e")
        'a5'
        >>> bucket_path("ff00000000000000000000000000000000000000000000000000000000000000")
        'ff'
    """
    if len(hash_hex) != 64:
        raise ValueError(f"hash_hex must be 64 characters, got {len(hash_hex)}")
    if not all(c in "0123456789abcdef" for c in hash_hex.lower()):
        raise ValueError("hash_hex must contain only hex characters")
    return hash_hex[:2].lower()


def shard_path(ob_dir: Path, layer: str, hash_hex: str) -> Path:
    """Construct shard path for a given layer and hash.

    Args:
        ob_dir: Path to .ob/ directory.
        layer: Layer name (e.g., "document-index", "sections", "revision").
        hash_hex: 64-character SHA-256 hex digest.

    Returns:
        Path to shard directory: .ob/{layer}/{first_2_hex_chars}

    Examples:
        >>> shard_path(Path(".ob"), "document-index", "a591a6d40bf420404a011733cfb7b190d62c65bf0bcda32b57b277d9ad9f146e")
        Path('.ob/document-index/a5')
    """
    bucket = bucket_path(hash_hex)
    return ob_dir / layer / bucket


def find_ob_dir(start: Path | None = None, ob_dir_override: str | None = None) -> Path:
    """Find .ob/ directory by walking up from start or using override.

    Args:
        start: Starting directory to search from. Defaults to Path.cwd().
        ob_dir_override: If provided, validate this path has .ob/ subdirectory
            and return the parent directory.

    Returns:
        Path to the directory containing .ob/ subdirectory.

    Raises:
        OBNotInitializedError: If .ob/ directory is not found.
        ValueError: If ob_dir_override is provided but .ob/ subdirectory not found.

    Examples:
        >>> # When .ob exists in current directory
        >>> find_ob_dir()  # doctest: +SKIP
        Path('/home/user/project')
    """
    if ob_dir_override:
        override_path = Path(ob_dir_override)
        if not (override_path / ".ob").exists():
            raise ValueError(
                f"ob_dir_override {ob_dir_override} does not contain .ob/ subdirectory"
            )
        return override_path

    if start is None:
        start = Path.cwd()

    current = start.resolve()
    while True:
        ob_path = current / ".ob"
        if ob_path.exists() and ob_path.is_dir():
            return current

        parent = current.parent
        if parent == current:
            break
        current = parent

    raise OBNotInitializedError(
        f".ob/ directory not found in {start} or any parent directory"
    )


def normalize_file(file: str, ob_dir: Path) -> str:
    resolved = Path(file).resolve()
    try:
        return str(resolved.relative_to(ob_dir.resolve()))
    except ValueError:
        try:
            rel = (ob_dir / file).resolve().relative_to(ob_dir.resolve())
            return str(rel)
        except ValueError:
            return file


def url_encode(name: str) -> str:
    """URL-encode a string for use in .ob/split/ filenames.

    Encodes special characters like '/' as '%2F' for safe filename usage.

    Args:
        name: String to encode.

    Returns:
        URL-encoded string with all special characters escaped.

    Examples:
        >>> url_encode("wiki/Paris")
        'wiki%2FParis'
        >>> url_encode("hello world")
        'hello%20world'
    """
    return urllib.parse.quote(name, safe="")
