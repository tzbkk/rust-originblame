import typer

from ob_util.reconcile import reconcile as do_reconcile

app = typer.Typer(
    invoke_without_command=True, help="Reconcile provenance (requires ob-util)"
)


@app.callback(invoke_without_command=True)
def reconcile(
    interactive: bool = typer.Option(
        False, "--interactive", "-i", help="Interactive mode (not yet implemented)"
    ),
    data_file: str | None = typer.Argument(None, help="Data file to reconcile"),
    model: str | None = typer.Option(
        None, "--model", "-m", help="Embedding model name (enables Pass 2)"
    ),
    threshold: float = typer.Option(
        0.85, "--threshold", "-t", help="Cosine similarity threshold (0-1)"
    ),
    embedding_api: str | None = typer.Option(
        None,
        "--embedding-api",
        "-e",
        help="OpenAI-compatible embedding API URL (e.g. http://localhost:1234/v1)",
    ),
    compute_all_embeddings: bool = typer.Option(
        False,
        "--compute-all-embeddings",
        help="Compute and store embeddings for ALL lines, not just unmatched",
    ),
):
    """Reconcile provenance data with hash and optional embedding matching."""
    if interactive:
        typer.echo("Warning: Interactive mode not yet implemented, using auto mode.")

    if data_file is None:
        typer.echo("Error: data_file argument is required.", err=True)
        raise typer.Exit(1)

    result = do_reconcile(
        data_file,
        model=model,
        threshold=threshold,
        interactive=interactive,
        embedding_api=embedding_api,
        compute_all_embeddings=compute_all_embeddings,
    )

    typer.echo(f"Hash matched:     {result.hash_matched}")
    typer.echo(f"Semantic matched: {result.semantic_matched}")
    typer.echo(f"New lines:        {result.new_lines}")
    typer.echo(f"Orphans:          {result.orphans}")
    typer.echo(f"Errors:           {result.errors}")
    typer.echo(f"Time:             {result.duration_ms:.0f}ms")

    if result.new_lines > 0:
        typer.echo(f"\n{result.new_lines} line(s) need manual tracking.")

    if result.orphans > 0:
        typer.echo(
            f"{result.orphans} orphan record(s) detected. Run 'ob clean' to archive."
        )

    if result.errors > 0:
        raise typer.Exit(1)
