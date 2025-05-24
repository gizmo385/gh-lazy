from functools import partial

from textual import work
from textual.app import ComposeResult
from textual.containers import Container
from textual.content import Content
from textual.reactive import reactive
from textual.screen import ModalScreen
from textual.widget import Widget
from textual.widgets import Markdown, Rule, Static

import lazy_github.lib.github.auth as auth
from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.github.backends.protocol import BackendType
from lazy_github.lib.logging import lg
from lazy_github.ui.screens.primary import LazyGithubMainScreen
from lazy_github.ui.widgets.common import LazyGithubFooter
from lazy_github.ui.widgets.info import LAZY_GITHUB_INFO


class UserTokenDisplay(Widget):
    user_code: reactive[str | None] = reactive(None)

    def render(self):
        if self.user_code:
            return Content.from_markup(
                "\n".join(
                    [
                        "Please verify at: https://github.com/login/device\n",
                        f"Your verification code is [greenyellow]{self.user_code}[/]",
                    ]
                )
            )
        else:
            return "Loading your device code..."


class GithubCliAuthInstructions(Container):
    DEFAULT_CSS = """
    GithubCliAuthInstructions {
        height: auto;
    }
    """

    def compose(self) -> ComposeResult:
        yield Static(Content.from_markup("To proceed, please log into the Github CLI via [green]gh auth login[/green]"))


class AuthenticationModal(ModalScreen):
    BINDINGS = [LazyGithubBindings.QUIT_APP]
    DEFAULT_CSS = """
    AuthenticationModal {
        border: ascii red;
        align: center middle;
        height: 1fr;
        width: 100%;
        content-align: center middle;
    }
    """

    async def action_quit(self) -> None:
        await self.app.action_quit()

    def compose(self) -> ComposeResult:
        self.border_title = "GitHub Authentication"
        with Container():
            match LazyGithubContext.client_type:
                case BackendType.GITHUB_CLI:
                    yield GithubCliAuthInstructions()
                case BackendType.RAW_HTTP:
                    yield UserTokenDisplay()
            yield Rule()
            yield Markdown(LAZY_GITHUB_INFO)
            yield LazyGithubFooter()

    @work
    async def check_access_token(self, device_code: auth.DeviceCodeResponse):
        access_token = await auth.get_access_token(device_code)
        match access_token.error:
            case "authorization_pending":
                lg.debug("Continuing to wait for auth...")
            case "slow_down":
                lg.debug("Continuing to wait for auth (slower)...")
                self.check_access_timer.stop()
                self.check_access_timer = self.set_interval(
                    device_code.polling_interval + 5,
                    partial(self.check_access_token, device_code),
                )
            case "expired_token":
                lg.warning("your device code is expired :(")
                self.check_access_timer.stop()
                raise ValueError("Your Github API device code has expired. Please restart")
            case "access_denied":
                lg.debug("Access denied :(")
                self.check_access_timer.stop()
            case _:
                lg.debug("Successfully authenticated!")
                auth.save_access_token(access_token)
                self.check_access_timer.stop()
                self.app.switch_screen(LazyGithubMainScreen())

    @work
    async def verify_valid_access_token(self):
        lg.debug("Attempting to get device code...")
        device_code = await auth.get_device_code()
        self.query_one(UserTokenDisplay).user_code = device_code.user_code

        # We want to check that the user has added the
        self.check_access_timer = self.set_interval(
            device_code.polling_interval, partial(self.check_access_token, device_code)
        )

    @work
    async def check_github_cli_access(self) -> None:
        if await auth.is_logged_in_to_cli():
            lg.debug("Github CLI access confirmed")
            self.check_access_timer.stop()
            self.app.switch_screen(LazyGithubMainScreen())
        else:
            lg.debug("Continuing to wait for Github CLI access...")

    @work
    async def verify_github_cli_access(self) -> None:
        self.check_access_timer = self.set_interval(5, partial(self.check_github_cli_access))

    async def on_mount(self):
        # Once this screen starts, we need to start the auth flow. This begins by
        # requesting a device token from github and asking the user to input it on
        # the auth page.
        match LazyGithubContext.client_type:
            case BackendType.GITHUB_CLI:
                self.verify_github_cli_access()
            case BackendType.RAW_HTTP:
                self.verify_valid_access_token()
