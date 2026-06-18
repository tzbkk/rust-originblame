"""Show provenance information command."""

from __future__ import annotations

import typer

app = typer.Typer(invoke_without_command=True, help="Show provenance information")


@app.callback(invoke_without_command=True)
def show(
    author: str | None = typer.Option(None, "--author", help="Filter by author name"),
    email: str | None = typer.Option(None, "--email", help="Filter by author email"),
    section: str | None = typer.Option(
        None, "--section", help="Filter by section hash"
    ),
    license_name: str | None = typer.Option(
        None, "--license", help="Filter by license"
    ),
    revoked: bool = typer.Option(False, "--revoked", help="Show only revoked entries"),
    index: bool = typer.Option(
        False, "--index", help="Use index for fast bucket routing"
    ),
):
    """Show provenance information grouped by author, section, and data files."""
    from _ob_native import (
        show_by_author as native_show_by_author,
        show_by_section as native_show_by_section,
        show_by_license as native_show_by_license,
    )
    from ob.cli import get_ob_dir

    ob_dir = get_ob_dir() or "."
    try:
        if section:
            results = native_show_by_section(ob_dir, section)
        elif license_name:
            results = native_show_by_license(ob_dir, license_name)
        elif author:
            results = native_show_by_author(ob_dir, author)
        else:
            results = native_show_by_author(ob_dir, "")
        print(results)
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)
