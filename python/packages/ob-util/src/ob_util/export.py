"""DEP-5 copyright export for OriginBlame data."""

from __future__ import annotations

from pathlib import Path

from ob.authors import get_author
from ob.exceptions import OBStorageError
from ob.indexer import list_pid_files, read_all_manifest
from ob.register import get_section
from ob.util import find_ob_dir


def export_copyright(
    output: str | None = None,
    data_files: list[str] | None = None,
    ob_dir: Path | None = None,
) -> str:
    """Export copyright information in DEP-5 format.

    Reads manifest records, resolves section/author metadata, and produces
    DEP-5 copyright blocks.  Does NOT log export operations.

    Args:
        output: Output file path.  If None, returns the string.
        data_files: Filter by data file paths.  If None, exports all.
        ob_dir: Repository root directory.  If None, auto-detected.

    Returns:
        DEP-5 formatted copyright string.

    Raises:
        OBStorageError: If unmerged PID files are detected.
    """
    if ob_dir is None:
        ob_dir = find_ob_dir()

    pid_files = list_pid_files(ob_dir)
    if pid_files:
        raise OBStorageError(
            "Unmerged PID files detected. Run 'ob clean' first."
        )

    records = read_all_manifest(ob_dir)

    if data_files:
        data_files_set = set(data_files)
        records = [r for r in records if r.get("file") in data_files_set]

    blocks: list[str] = []
    for record in records:
        line_hash = record.get("line_hash", "")
        file_path = record.get("file", "")
        sources = record.get("sources", [])

        if not line_hash or not file_path:
            continue

        files_field = f"{file_path}:{line_hash}"

        for section_hash in sources:
            section = get_section(ob_dir, section_hash)
            if section is None:
                continue

            license_name = section.get("license") or "UNKNOWN"
            year = section.get("year", "")
            author_ids = section.get("authors", [])

            copyright_lines: list[str] = []
            for author_id in author_ids:
                author = get_author(ob_dir, author_id)
                if author is not None:
                    name = author.get("name", "")
                    email = author.get("email", "")
                    if year:
                        copyright_lines.append(f"{year} {name} <{email}>")
                    else:
                        copyright_lines.append(f"{name} <{email}>")

            if not copyright_lines:
                continue

            copyright_field = "\n".join(
                f"Copyright: {line}" for line in copyright_lines
            )

            block = (
                f"Files: {files_field}\n"
                f"{copyright_field}\n"
                f"License: {license_name}\n"
            )
            blocks.append(block)

    result = "\n".join(blocks)

    if output:
        output_path = Path(output)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(result, encoding="utf-8")

    return result
