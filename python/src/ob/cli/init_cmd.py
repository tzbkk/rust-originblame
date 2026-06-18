"""Initialize OriginBlame tracking command."""

from __future__ import annotations

import typer

from ob.cli import get_ob_dir

app = typer.Typer(invoke_without_command=True, help="Initialize OriginBlame tracking")


@app.callback(invoke_without_command=True)
def init(
    force: bool = typer.Option(False, "--force", help="Force overwrite invalid .ob/"),
):
    """Create .ob/ directory structure (idempotent)"""
    from _ob_native import init as native_init

    ob_dir = get_ob_dir() or "."
    try:
        native_init(ob_dir)
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)
