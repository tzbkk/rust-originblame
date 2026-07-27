"""Section management commands."""

from __future__ import annotations

import typer

from ob.cli import get_ob_dir

app = typer.Typer(invoke_without_command=True, help="Section management")


@app.callback(invoke_without_command=True)
def _section_default(ctx: typer.Context):
    """Section management"""
    if ctx.invoked_subcommand is not None:
        return
    typer.echo(
        "Usage: ob register add --path PATH --authors ID --license NAME --year YEAR"
    )
    raise typer.Exit(1)


@app.command()
def add(
    path: str = typer.Option(..., "--path", help="File or directory path"),
    authors: list[str] = typer.Option(..., "--authors", help="Author IDs"),
    license_name: str = typer.Option(..., "--license", help="License name"),
    year: str = typer.Option(..., "--year", help="Copyright year"),
    contributors: list[str] = typer.Option([], "--contributors", help="Contributor IDs (secondary)"),
):
    """Add a new section to track"""
    from ob._ob_native import register_section as native_register

    ob_dir = get_ob_dir() or "."
    try:
        native_register(path, authors, contributors or [], license_name, year, ob_dir)
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)
