"""Blame command - show line-level provenance."""

from __future__ import annotations

import typer

app = typer.Typer(invoke_without_command=True, help="Show line-level provenance")


@app.callback(invoke_without_command=True)
def blame(
    file: str = typer.Argument(..., help="Data file path"),
    line: int = typer.Argument(..., help="Line number (1-indexed)"),
    json_output: bool = typer.Option(False, "--json", help="Output as JSON"),
):
    """Show provenance for a specific line in a data file."""
    from _ob_native import blame as native_blame
    from ob.cli import get_ob_dir

    ob_dir = get_ob_dir() or "."
    try:
        result = native_blame(ob_dir, file, line)
        print(result)
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)
