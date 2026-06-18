"""Parse command stub - requires ob-util."""

import typer

app = typer.Typer(invoke_without_command=True, help="Parse files (requires ob-util)")


@app.callback(invoke_without_command=True)
def parse(
    parser: str = typer.Option(..., "--parser", help="Parser name"),
    file: str = typer.Argument(..., help="File to parse"),
):
    """Parse file with specified parser"""
    raise RuntimeError(
        "This command requires ob-util. Install it with: pip install ob-util"
    )
