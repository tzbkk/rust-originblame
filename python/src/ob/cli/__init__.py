"""OriginBlame CLI application."""

import typer

app = typer.Typer(
    name="ob",
    help="OriginBlame -- Line-level provenance tracking for AI training datasets",
    no_args_is_help=True,
    add_completion=True,
)

# Global --ob-dir override
_ob_dir_override: str | None = None


def get_ob_dir() -> str | None:
    return _ob_dir_override


def ob_dir_callback(ctx: typer.Context, value: str | None):
    global _ob_dir_override
    _ob_dir_override = value
    return value


@app.callback()
def main(
    ob_dir: str | None = typer.Option(
        None,
        "--ob-dir",
        help="Override .ob/ directory path",
        callback=ob_dir_callback,
    ),
):
    """OriginBlame -- Line-level provenance tracking for AI training datasets"""
    pass


# Import and register all core CLI commands
from ob.cli.init_cmd import app as init_app
from ob.cli.author_cmd import app as author_app
from ob.cli.register_cmd import app as register_app
from ob.cli.show_cmd import app as show_app
from ob.cli.blame_cmd import app as blame_app
from ob.cli.revoke_cmd import app as revoke_app
from ob.cli.purge_cmd import app as purge_app
from ob.cli.clean_cmd import app as clean_app
from ob.cli.status_cmd import app as status_app
from ob.cli.log_cmd import app as log_app
from ob.cli.merge_cmd import app as merge_app

# Register core commands
app.add_typer(init_app, name="init")
app.add_typer(author_app, name="author")
app.add_typer(register_app, name="register")
app.add_typer(show_app, name="show")
app.add_typer(blame_app, name="blame")
app.add_typer(revoke_app, name="revoke")
app.add_typer(purge_app, name="purge")
app.add_typer(clean_app, name="clean")
app.add_typer(status_app, name="status")
app.add_typer(log_app, name="log")
app.add_typer(merge_app, name="merge")

# Version command is implemented directly (not a sub-app)
import ob


@app.command()
def version():
    """Show OriginBlame version"""
    print(f"ob {ob.__version__}")


# Import and register ob-util stub commands
from ob.cli.parse_cmd import app as parse_app
from ob.cli.reconcile_cmd import app as reconcile_app
from ob.cli.export_cmd import app as export_app
from ob.cli.index_cmd import app as index_app

# Register ob-util stub commands
app.add_typer(parse_app, name="parse")
app.add_typer(reconcile_app, name="reconcile")
app.add_typer(export_app, name="export-copyright")
app.add_typer(index_app, name="index")

# Discover and register additional commands via entry points (for future ob-util)
try:
    from importlib.metadata import entry_points

    eps = entry_points(group="ob.commands")
    for ep in eps:
        sub_app = ep.load()
        if isinstance(sub_app, typer.Typer):
            app.add_typer(sub_app, name=ep.name)
except Exception:
    # No additional entry points, that's fine
    pass
