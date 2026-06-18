"""Log command - show operation log."""

from pathlib import Path

import typer

from ob.oplog import query_log

app = typer.Typer(invoke_without_command=True, help="Show operation log")


def _resolve_ob_dir() -> Path:
    from ob.cli import get_ob_dir

    ob_dir_override = get_ob_dir()
    if ob_dir_override:
        return Path(ob_dir_override)
    return Path.cwd()


def _format_detail(detail: dict) -> str:
    if not detail:
        return ""
    parts = []
    for k, v in detail.items():
        parts.append(f"{k}={v}")
    return ", ".join(parts)


@app.callback(invoke_without_command=True)
def log(
    op: str | None = typer.Option(None, "--op", help="Filter by operation type"),
    since: str | None = typer.Option(None, "--since", help="ISO 8601 timestamp"),
):
    """Show operation log entries, most recent first."""
    ob_dir = _resolve_ob_dir()

    try:
        entries = query_log(ob_dir, op=op, since=since)
    except Exception as e:
        typer.echo(f"Error: {e}", err=True)
        raise typer.Exit(1)

    if not entries:
        typer.echo("No log entries found.")
        return

    for entry in entries:
        ts = entry.get("ts", "")
        operation = entry.get("op", "?")
        detail = _format_detail(entry.get("detail", {}))
        line = f"{ts}  {operation}"
        if detail:
            line += f"  {detail}"
        typer.echo(line)
