"""Index command - build provenance index."""

from __future__ import annotations

import typer

from ob.cli import get_ob_dir

app = typer.Typer(help="Build/query index")


@app.command()
def build(
    ob_dir: str = typer.Option(".", "-d", "--dir", help="Repository root directory"),
):
    """Build the provenance index for fast lookups."""
    from ob._ob_native import build_index as native_build

    target_dir = get_ob_dir() or ob_dir
    try:
        result = native_build(target_dir)
        print(
            f"Index built: {result['authors']} authors, "
            f"{result['sections']} sections, "
            f"{result['total']} total entries"
        )
        if result["token_index_entries"] > 0:
            print(f"Token-index: {result['token_index_entries']} entries")
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)
