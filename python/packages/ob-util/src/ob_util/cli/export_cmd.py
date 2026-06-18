import typer

app = typer.Typer(
    invoke_without_command=True, help="Export copyright notices (requires ob-util)"
)


@app.callback(invoke_without_command=True)
def export_copyright(
    output: str | None = typer.Option(None, "-o", "--output", help="Output file path"),
    data_file: list[str] | None = typer.Option(
        None, "--data-file", help="Filter by data file"
    ),
):
    """Export copyright notices."""
    from ob_util.export import export_copyright as do_export

    result = do_export(output=output, data_files=data_file)
    if not output:
        typer.echo(result)
