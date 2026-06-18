"""Clean command - merge PID files and archive revoked records."""

from pathlib import Path

import typer

from ob.cli import get_ob_dir
from ob.exceptions import OBCleanError

app = typer.Typer(invoke_without_command=True, help="Clean up orphaned provenance data")


@app.callback(invoke_without_command=True)
def clean(
    split: bool = typer.Option(
        False, "--split", help="Also remove .ob/split/ contents"
    ),
):
    """Merge PID files into shards, archive revoked records, rotate log."""
    from ob.clean import clean as ob_clean

    ob_dir_val = get_ob_dir()
    ob_dir = Path(ob_dir_val) if ob_dir_val else None

    try:
        result = ob_clean(split=split, ob_dir=ob_dir)
    except OBCleanError as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)

    typer.echo(
        f"Cleaned: {result.document_merged} document index, "
        f"{result.embeddings_merged} embedding records merged, "
        f"{result.pid_files_deleted} PID files deleted, "
        f"{result.locked_pid_files} locked (skipped), "
        f"{result.archived_records} revoked archived, "
        f"{result.log_rotated} log entries rotated"
    )
    if result.split_cleaned:
        typer.echo("Split directory cleaned.")
