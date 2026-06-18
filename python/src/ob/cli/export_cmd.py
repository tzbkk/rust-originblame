"""Export-copyright command stub - requires ob-util."""

import typer

app = typer.Typer(
    invoke_without_command=True, help="Export copyright notices (requires ob-util)"
)


@app.callback(invoke_without_command=True)
def export_copyright(
    output: str | None = typer.Option(None, "-o", "--output", help="Output file"),
    data_files: list[str] = typer.Option(
        None, "--data-file", help="Data files to process"
    ),
):
    """Export copyright notices"""
    raise RuntimeError(
        "This command requires ob-util. Install it with: pip install ob-util"
    )
