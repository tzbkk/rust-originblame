"""Status command - show repository status."""

from pathlib import Path
from collections import defaultdict

import typer

import ob
from ob.indexer import list_pid_files
from ob.oplog import query_log
from ob.storage import LAYER_AUTHORS, LAYER_MANIFEST, LAYER_SECTION, shard_iterate_all

app = typer.Typer(invoke_without_command=True, help="Show repository status")


def _resolve_ob_dir() -> Path:
    from ob.cli import get_ob_dir

    ob_dir_override = get_ob_dir()
    if ob_dir_override:
        return Path(ob_dir_override)
    return Path.cwd()


def _count_layer(ob_dir: Path, layer: str):
    total = 0
    revoked = 0
    for record in shard_iterate_all(ob_dir, layer):
        total += 1
        if record.get("revoked"):
            revoked += 1
    return total, revoked


def _get_embedding_models(ob_dir: Path) -> dict[str, int]:
    model_counts: dict[str, int] = defaultdict(int)
    ob_dot = ob_dir / ".ob"
    if not ob_dot.exists():
        return model_counts
    for child in ob_dot.iterdir():
        if child.name.startswith("embeddings.") and child.is_dir():
            model_name = child.name[len("embeddings.") :]
            count = sum(
                1 for _ in shard_iterate_all(ob_dir, f"embeddings.{model_name}")
            )
            if count > 0:
                model_counts[model_name] = count
    return model_counts


@app.callback(invoke_without_command=True)
def status():
    """Show repository tracking status."""
    ob_dir = _resolve_ob_dir()
    ob_dot = ob_dir / ".ob"

    if not ob_dot.exists():
        typer.echo("Not initialized. Run 'ob init' first.")
        raise typer.Exit(1)

    author_total, author_revoked = _count_layer(ob_dir, LAYER_AUTHORS)
    section_total, section_revoked = _count_layer(ob_dir, LAYER_SECTION)
    manifest_total, manifest_revoked = _count_layer(ob_dir, LAYER_MANIFEST)

    typer.echo(f"Authors: {author_total}", nl=False)
    if author_revoked > 0:
        typer.echo(f" ({author_revoked} revoked)")
    else:
        typer.echo("")

    typer.echo(f"Sections: {section_total}", nl=False)
    if section_revoked > 0:
        typer.echo(f" ({section_revoked} revoked)")
    else:
        typer.echo("")

    typer.echo(f"Document index records: {manifest_total}", nl=False)
    if manifest_revoked > 0:
        typer.echo(f" ({manifest_revoked} revoked)")
    else:
        typer.echo("")

    pid_files = list_pid_files(ob_dir)
    if pid_files:
        typer.echo(f"Unmerged PID files: {len(pid_files)} (run 'ob clean')")
    else:
        typer.echo("Unmerged PID files: 0")

    log_entries = query_log(ob_dir)
    typer.echo(f"Log entries: {len(log_entries)}")

    model_counts = _get_embedding_models(ob_dir)
    if model_counts:
        models_str = ", ".join(f"{m} ({c})" for m, c in sorted(model_counts.items()))
        typer.echo(f"Embedding models: {models_str}")
    else:
        typer.echo("Embedding models: none")

    typer.echo(f"ob version: {ob.__version__}")
