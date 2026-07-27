"""Merge command - merge provenance data."""

from pathlib import Path

import typer

from ob.cli import get_ob_dir

app = typer.Typer(invoke_without_command=True, help="Merge provenance data")


@app.callback(invoke_without_command=True)
def _merge_default(ctx: typer.Context):
    """Merge provenance data"""
    if ctx.invoked_subcommand is not None:
        return
    typer.echo("Usage: ob merge absorb PATH")
    raise typer.Exit(1)


@app.command("absorb")
def absorb_cmd(
    source: str = typer.Argument(..., help="Source repository root containing .ob/"),
):
    """Merge another repository's provenance into current repository."""
    from ob._ob_native import merge_absorb

    ob_dir_override = get_ob_dir()
    target = str(Path(ob_dir_override).resolve()) if ob_dir_override else "."

    try:
        result = merge_absorb(source, target)
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)

    typer.echo(
        f"Merged: {result['authors_added']} authors, "
        f"{result['sections_added']} sections, "
        f"{result['document_added']} document index records, "
        f"{result['skipped']} skipped"
    )
    if result["token_index_added"] > 0:
        typer.echo(f"Token-index added: {result['token_index_added']}")
