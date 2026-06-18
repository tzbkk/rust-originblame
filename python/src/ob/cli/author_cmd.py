"""Author management commands."""

from __future__ import annotations

import typer

from ob.cli import get_ob_dir

app = typer.Typer(invoke_without_command=True, help="Author management")


@app.callback(invoke_without_command=True)
def _author_default(ctx: typer.Context):
    """Author management"""
    if ctx.invoked_subcommand is not None:
        return
    typer.echo("Usage: ob author add NAME EMAIL")
    raise typer.Exit(1)


@app.command()
def add(
    name: str = typer.Argument(..., help="Author name"),
    email: str = typer.Argument(..., help="Author email"),
):
    """Add a new author to the repository"""
    from _ob_native import author_add as native_author_add

    ob_dir = get_ob_dir() or "."
    try:
        native_author_add(name, email, ob_dir)
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)
