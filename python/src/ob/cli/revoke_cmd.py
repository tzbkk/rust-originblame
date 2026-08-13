"""Revoke command - revoke at author, section, or record level."""

from __future__ import annotations

import typer

from ob.cli import get_ob_dir

app = typer.Typer(invoke_without_command=True, help="Revoke author claims")


@app.callback(invoke_without_command=True)
def revoke(
    ctx: typer.Context,
    email: str | None = typer.Option(None, "--email", help="Revoke by author email"),
    author_id: str | None = typer.Option(None, "--author", help="Revoke by author ID"),
    section_hash: str | None = typer.Option(
        None, "--section", help="Revoke by section hash"
    ),
    line_hash: str | None = typer.Option(
        None,
        "--line-hash",
        help="Revoke a single document-index entry (requires --file)",
    ),
    file: str | None = typer.Option(
        None, "--file", help="File path (required with --line-hash)"
    ),
    tokenizer: str | None = typer.Option(
        None,
        "--tokenizer",
        help="Revoke token-index entries for the author (e.g. gpt2)",
    ),
    reverse: bool = typer.Option(
        False, "--reverse", help="Undo a prior revocation"
    ),
):
    """Revoke at author, section, or record granularity.

    \b
    Author-level:   ob revoke --author ID
    Token-level:    ob revoke --author ID --tokenizer gpt2
    Section-level:  ob revoke --section HASH
    Record-level:   ob revoke --line-hash HASH --file FILE
    """
    if ctx.invoked_subcommand is not None:
        return

    ob_dir = get_ob_dir() or "."

    try:
        if line_hash is not None:
            if not file:
                typer.echo("Error: --line-hash requires --file", err=True)
                raise typer.Exit(1)
            from ob._ob_native import revoke_manifest

            count = revoke_manifest(ob_dir, line_hash, file, reverse)
        elif section_hash is not None:
            from ob._ob_native import revoke_section

            count = revoke_section(ob_dir, section_hash, reverse)
        elif email is not None or author_id is not None:
            author_name = email or author_id or ""
            if tokenizer is not None:
                from ob._ob_native import revoke_by_author_token

                count = revoke_by_author_token(
                    ob_dir, author_name, tokenizer, reverse
                )
            else:
                from ob._ob_native import revoke_by_author

                count = revoke_by_author(ob_dir, author_name, reverse)
        else:
            typer.echo(
                "Error: at least one flag required "
                "(--email, --author, --section, or --line-hash)",
                err=True,
            )
            raise typer.Exit(1)

        action = "Restored" if reverse else "Revoked"
        typer.echo(f"{action} {count} record(s).")
    except typer.Exit:
        raise
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)
