"""Version command - show ob version."""

import typer
import ob

app = typer.Typer(help="Show version information")


@app.command()
def version():
    """Show OriginBlame version"""
    print(f"ob {ob.__version__}")
