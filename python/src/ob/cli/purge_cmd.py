"""Purge command - physically delete revoked data from tracked files."""

from __future__ import annotations

import typer

from ob.cli import get_ob_dir

app = typer.Typer(
    invoke_without_command=True,
    help="Physically delete revoked data from tracked files",
)


@app.callback(invoke_without_command=True)
def _purge_default(ctx: typer.Context):
    """Purge revoked data"""
    if ctx.invoked_subcommand is not None:
        return
    typer.echo("Usage: ob purge --file FILE [--dry-run]")
    raise typer.Exit(1)


@app.command()
def purge(
    file: str = typer.Option(None, "--file", help="File path to purge"),
    index: bool = typer.Option(
        False, "--index", help="Use index for author/section routing"
    ),
    dry_run: bool = typer.Option(
        False, "--dry-run", help="List what would be purged without deleting"
    ),
):
    """Physically delete revoked data lines from a tracked file."""
    from _ob_native import purge_revoked as native_purge

    if not file:
        typer.echo("Error: specify --file", err=True)
        raise typer.Exit(1)

    ob_dir = get_ob_dir() or "."
    try:
        native_purge(ob_dir, file, dry_run)
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)
