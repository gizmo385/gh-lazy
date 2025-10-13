import os
import shutil

import click
import rich

from lazy_github.lib.config import _CONFIG_FILE_LOCATION, Config
from lazy_github.lib.debug import collect_debug_info
from lazy_github.lib.github.backends.protocol import BackendType
from lazy_github.ui.app import app

_CLI_CONTEXT_SETTINGS = dict(help_option_names=["-h", "--help"])


@click.group(invoke_without_command=True, context_settings=_CLI_CONTEXT_SETTINGS)
@click.pass_context
def cli(ctx: click.Context) -> None:
    """A Terminal UI for interacting with Github"""
    # Set LAZY_GITHUB_ORIGINAL_PWD if not already set (fixes repo detection when installed via flake)
    if "LAZY_GITHUB_ORIGINAL_PWD" not in os.environ:
        os.environ["LAZY_GITHUB_ORIGINAL_PWD"] = os.getcwd()

    if ctx.invoked_subcommand is None:
        ctx.invoke(run)


@cli.command
@click.option(
    "--auth-backend",
    help="Specifies which authentication backend to use",
    envvar="AUTH_BACKEND",
    type=click.Choice(list(BackendType)),
)
def run(auth_backend: BackendType | None):
    """Run LazyGithub"""
    if isinstance(auth_backend, BackendType):
        with Config.to_edit() as config:
            config.api.client_type = auth_backend
    app.run()


@cli.command
def dump_config():
    """Dump the current configuration, as it would be loaded by LazyGithub"""
    print(f"Config file location: {_CONFIG_FILE_LOCATION} (exists => {_CONFIG_FILE_LOCATION.exists()})")
    rich.print_json(Config.load_config().model_dump_json())


@cli.command
def clear_auth():
    """Clears out any existing authentication config for LazyGithub, forcing the user to relogin"""
    from lazy_github.lib.github.auth import _AUTHENTICATION_CACHE_LOCATION

    _AUTHENTICATION_CACHE_LOCATION.unlink(missing_ok=True)


@cli.command
def clear_config():
    """Reset the user's settings"""
    _CONFIG_FILE_LOCATION.unlink(missing_ok=True)
    print("Your settings have been cleared")


@cli.command
@click.option("--no-confirm", is_flag=True, default=False, help="Don't ask for confirmation")
def clear_cache(no_confirm: bool):
    """Reset the lazy-github cache"""
    from lazy_github.lib.context import LazyGithubContext

    cache_directory = LazyGithubContext.config.cache.cache_directory
    if no_confirm or click.confirm(f"Confirm deletion of everything in {cache_directory}"):
        if cache_directory.exists():
            shutil.rmtree(cache_directory)
        print("Cache cleared")
    else:
        print("Canceling cache deletion")


@cli.command
def debug():
    """Outputs LazyGithub debug info"""
    print(collect_debug_info())
