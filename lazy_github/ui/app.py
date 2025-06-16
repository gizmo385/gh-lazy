from datetime import datetime

from textual import on
from textual.app import App, ComposeResult
from textual.containers import Horizontal
from textual.css.query import NoMatches
from textual.screen import Screen
from textual.theme import Theme
from textual.widgets import Button, HelpPanel, Markdown, RadioButton, RadioSet

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.github import auth
from lazy_github.lib.github.auth import GithubAuthenticationRequired
from lazy_github.lib.github.backends.protocol import BackendType
from lazy_github.lib.logging import lg
from lazy_github.lib.messages import SettingsModalDismissed
from lazy_github.ui.screens.auth import AuthenticationModal
from lazy_github.ui.screens.primary import LazyGithubMainScreen
from lazy_github.ui.widgets.common import LazyGithubFooter


class FirstStartScreen(Screen[BackendType | None]):
    DEFAULT_CSS = """
    RadioSet {
        margin: 1;
    }
    Static {
        margin: 1;
    }
    Button {
        margin: 1;
    }
    """

    def compose(self) -> ComposeResult:
        yield Markdown("# Welcome to LazyGithub! How would you like to authenticate with the Github API?")
        with RadioSet(id="auth-select"):
            yield RadioButton("Via the Github CLI", id=str(BackendType.GITHUB_CLI))
            yield RadioButton("Directly via HTTPS", value=True, id=str(BackendType.RAW_HTTP))
        with Horizontal():
            yield Button("Continue", id="continue", variant="success")
            yield Button("Cancel", id="cancel", variant="error")
        yield LazyGithubFooter()

    @on(Button.Pressed, "#continue")
    def handle_submit(self, _: Button) -> None:
        radio_set = self.query_one("#auth-select", RadioSet)
        if radio_set.pressed_button:
            self.dismiss(BackendType(radio_set.pressed_button.id))
        else:
            self.dismiss(None)

    @on(Button.Pressed, "#cancel")
    async def handle_cancel(self, _: Button) -> None:
        await self.app.action_quit()


class LazyGithub(App):
    BINDINGS = [
        LazyGithubBindings.QUIT_APP,
        LazyGithubBindings.OPEN_COMMAND_PALLETE,
        LazyGithubBindings.MAXIMIZE_WIDGET,
        LazyGithubBindings.OPEN_HELP,
    ]

    has_shown_maximize_toast: bool = False
    main_screen: LazyGithubMainScreen | None = None

    async def authenticate_with_github(self):
        current_time = datetime.now()
        auth_last_checked = LazyGithubContext.config.cache.auth_last_checked
        auth_cache_duration = LazyGithubContext.config.cache.auth_cache_duration

        if auth_last_checked and (current_time - auth_last_checked).total_seconds() < auth_cache_duration:
            lg.debug("Triggering auth with github")
            self.main_screen = LazyGithubMainScreen(id="main-screen")
            self.push_screen(self.main_screen)
        else:
            try:
                # We pull the user here to validate auth
                await auth.assert_is_logged_in()
                with LazyGithubContext.config.to_edit() as config:
                    config.cache.auth_last_checked = current_time

                self.main_screen = LazyGithubMainScreen(id="main-screen")
                self.push_screen(self.main_screen)
            except GithubAuthenticationRequired:
                lg.debug("Triggering auth with github")
                self.push_screen(AuthenticationModal(id="auth-modal"))

    async def handle_first_start_screen_dismiss(self, selected_backend_type: BackendType | None) -> None:
        if selected_backend_type:
            with LazyGithubContext.config.to_edit() as config:
                config.api.client_type = selected_backend_type
                config.core.first_start = False
        await self.authenticate_with_github()

    async def on_ready(self):
        if LazyGithubContext.config.core.first_start:
            self.push_screen(FirstStartScreen(), self.handle_first_start_screen_dismiss)
        else:
            await self.authenticate_with_github()

    async def on_settings_modal_dismissed(self, message: SettingsModalDismissed) -> None:
        if not message.changed:
            return

        self.notify("Settings updated")
        if isinstance(LazyGithubContext.config.appearance.theme, Theme):
            self.theme = LazyGithubContext.config.appearance.theme.name
        else:
            self.theme = LazyGithubContext.config.appearance.theme

        self.set_keymap(LazyGithubContext.config.bindings.overrides)

        if not self.main_screen:
            raise ValueError("Error: main screen missing!")
        self.main_screen.handle_settings_update()

    def on_mount(self) -> None:
        self.theme = LazyGithubContext.config.appearance.theme.name
        self.set_keymap(LazyGithubContext.config.bindings.overrides)

    async def action_open_help(self) -> None:
        try:
            help_panel = self.screen.query_one(HelpPanel)
        except NoMatches:
            self.action_show_help_panel()
        else:
            help_panel.remove()

    def action_maximize(self) -> None:
        if self.screen.is_maximized:
            return
        if self.screen.focused is not None:
            # We don't need to repeatedly show this to the user
            if self.screen.maximize(self.screen.focused) and not self.has_shown_maximize_toast:
                self.notify("Current view maximized. Press [b]escape[/b] to return.", title="View Maximized")
                self.has_shown_maximize_toast = True


app = LazyGithub()
