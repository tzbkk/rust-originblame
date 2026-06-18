"""Reconcile command stub - requires ob-util."""

import typer

app = typer.Typer(
    invoke_without_command=True, help="Reconcile provenance (requires ob-util)"
)


@app.callback(invoke_without_command=True)
def reconcile(
    interactive: bool = typer.Option(False, "--interactive", help="Interactive mode"),
):
    """Reconcile provenance data"""
    raise RuntimeError(
        "This command requires ob-util. Install it with: pip install ob-util"
    )
