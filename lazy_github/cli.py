import click

from lazy_github.lib.config import _CONFIG_FILE_LOCATION, Config
from lazy_github.ui.app import app


@click.group(invoke_without_command=True)
@click.pass_context
def cli(ctx: click.Context) -> None:
    if ctx.invoked_subcommand is None:
        run()


@cli.command
def run():
    """Run LazyGithub"""
    app.run()


@cli.command
def dump_config():
    """Dump the current configuration, as it would be loaded by LazyGithub"""
    print(f"Config file location: {_CONFIG_FILE_LOCATION} (exists => {_CONFIG_FILE_LOCATION.exists()})")
    print(Config.load_config().model_dump_json(indent=4))
