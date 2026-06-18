"""Revoke command - revoke author claims."""

from __future__ import annotations

import typer

from ob.cli import get_ob_dir

app = typer.Typer(invoke_without_command=True, help="Revoke author claims")


@app.callback(invoke_without_command=True)
def _revoke_default(ctx: typer.Context):
    """Revoke author claims"""
    if ctx.invoked_subcommand is not None:
        return
    typer.echo(
        "Usage: ob revoke (--email EMAIL | --author ID | --file PATH | --section HASH)"
    )
    raise typer.Exit(1)


@app.command()
def revoke(
    email: str | None = typer.Option(None, "--email", help="Revoke by email"),
    author_id: str | None = typer.Option(None, "--author", help="Revoke by author ID"),
    file: str | None = typer.Option(
        None, "--file", help="Revoke sections by file path"
    ),
    section_hash: str | None = typer.Option(
        None, "--section", help="Revoke by section hash"
    ),
):
    """Revoke author claims at author, section, or line granularity."""
    from _ob_native import revoke_by_author as native_revoke

    if all(v is None for v in (email, author_id, file, section_hash)):
        typer.echo(
            "Error: at least one flag required (--email, --author, --file, --section)",
            err=True,
        )
        raise typer.Exit(1)

    ob_dir = get_ob_dir() or "."
    author_name = email or author_id or ""

    try:
        native_revoke(ob_dir, author_name)
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)
