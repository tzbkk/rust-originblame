"""Operation log -- append, query, rotate."""

from datetime import datetime
from pathlib import Path

from ob.storage import jsonl_append, jsonl_read

__all__ = ["append_log", "query_log", "rotate_log"]


def _log_path(ob_dir: Path) -> Path:
    return ob_dir / ".ob" / "log"


def append_log(ob_dir: Path, op: str, detail: dict, cmd: str) -> None:
    """Append an operation log entry.

    Args:
        ob_dir: Root directory of the repository.
        op: Operation name (e.g. "revoke", "purge", "init").
        detail: Arbitrary detail dict.
        cmd: Full command string that triggered the operation.
    """
    entry: dict = {
        "ts": datetime.now().isoformat(),
        "op": op,
        "detail": detail,
        "cmd": cmd,
    }
    jsonl_append(_log_path(ob_dir), entry)


def query_log(ob_dir: Path, op: str | None = None, since: str | None = None) -> list[dict]:
    """Read and filter operation log entries.

    Args:
        ob_dir: Root directory of the repository. If None, auto-detected.
        op: If given, only return entries matching this operation name.
        since: If given (ISO 8601 string), only return entries with ts >= since.

    Returns:
        Filtered list of log entries, most recent first.
    """
    if ob_dir is None:
        from ob.util import find_ob_dir
        ob_dir = find_ob_dir()

    entries = jsonl_read(_log_path(ob_dir))

    if op is not None:
        entries = [e for e in entries if e.get("op") == op]

    if since is not None:
        entries = [e for e in entries if e.get("ts", "") >= since]

    entries.reverse()
    return entries


def rotate_log(ob_dir: Path, max_size_bytes: int = 10_485_760) -> int:
    """Rotate the log file if it exceeds *max_size_bytes*.

    Moves the current log to ``.ob/archive/log.<timestamp>`` and starts a
    fresh empty log.

    Args:
        ob_dir: Root directory of the repository.
        max_size_bytes: Rotation threshold in bytes (default 10 MiB).

    Returns:
        Number of lines rotated (0 if no rotation needed).
    """
    log_file = _log_path(ob_dir)

    if not log_file.exists() or log_file.stat().st_size < max_size_bytes:
        return 0

    lines = jsonl_read(log_file)
    count = len(lines)
    if count == 0:
        return 0

    ts = datetime.now().strftime("%Y-%m-%dT%H-%M-%S")
    archive_dir = ob_dir / ".ob" / "archive"
    archive_dir.mkdir(parents=True, exist_ok=True)
    archive_file = archive_dir / f"log.{ts}"

    import shutil

    shutil.move(str(log_file), str(archive_file))
    return count
