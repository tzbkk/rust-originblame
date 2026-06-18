import typer

app = typer.Typer(invoke_without_command=True, help="Parse files (requires ob-util)")


@app.callback(invoke_without_command=True)
def parse(
    parser: str = typer.Option(..., "--parser", help="Parser name (e.g., mediawiki)"),
    file: str = typer.Argument(..., help="File to parse"),
    license: str = typer.Option("CC-BY-SA-4.0", "--license", help="Default license"),
):
    """Parse file with specified parser."""
    if parser == "mediawiki":
        from ob_util.parsers.mediawiki import MediawikiParser

        mw = MediawikiParser(license=license)
        result = mw.parse(file)
        typer.echo(f"Pages parsed:       {result.pages_parsed}")
        typer.echo(f"Authors registered: {result.authors_registered}")
        typer.echo(f"Sections created:   {result.sections_created}")
        typer.echo(f"Split files:        {result.split_files_created}")
    else:
        typer.echo(f"Unknown parser: {parser}. Available: mediawiki")
        raise typer.Exit(1)
