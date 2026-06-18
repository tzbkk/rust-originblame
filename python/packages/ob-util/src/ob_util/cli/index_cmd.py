import typer
from pathlib import Path

app = typer.Typer(invoke_without_command=True, help="Build and manage provenance index")


@app.callback(invoke_without_command=True)
def index(ctx: typer.Context):
    """Build or query the provenance index"""
    if ctx.invoked_subcommand is not None:
        return
    typer.echo("Use 'ob index build' to build the index.")
    raise typer.Exit(1)


@app.command()
def build():
    """Build the provenance index from .ob/ data."""
    from ob.util import find_ob_dir
    from ob_util.index import build_index

    ob_dir = find_ob_dir()
    counts = build_index(ob_dir)
    typer.echo(
        f"Index built: {counts['authors']} authors, {counts['sections']} sections "
        f"({counts['total']} total records)"
    )
