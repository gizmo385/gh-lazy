import enum
from pathlib import Path
from typing import Any, Optional

from pydantic import BaseModel
from pydantic.fields import FieldInfo
from textual import on, work
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Container, Horizontal, ScrollableContainer, Vertical
from textual.css.query import NoMatches
from textual.events import Key
from textual.fuzzy import Matcher
from textual.screen import ModalScreen
from textual.theme import BUILTIN_THEMES, Theme
from textual.validation import ValidationResult, Validator
from textual.widget import Widget
from textual.widgets import Button, Collapsible, Input, Label, Markdown, RichLog, Rule, Select, Static, Switch
from textual_fspicker import FileOpen
from textual_fspicker.path_filters import Filters

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.messages import SettingsModalDismissed
from lazy_github.ui.widgets.common import LazyGithubFooter, ToggleableSearchInput


def _field_name_to_readable_name(name: str) -> str:
    return name.replace("_", " ").title()


def _id_for_field_input(field_name: str) -> str:
    return f"adjust_{field_name}_input"


class ListOfStringValidator(Validator):
    def validate(self, value: str) -> ValidationResult:
        stripped_value = value.strip()
        if stripped_value.endswith(",") or stripped_value.startswith(","):
            return self.failure("No trailing or preceeding commas")
        return self.success()


class FileSelector(Vertical):
    DEFAULT_CSS = """
    FileSelector {
        height: 10;
    }

    Input {
        width: 70;
    }
    """

    def __init__(self, field_name: str, field: FieldInfo, selected_file: Path | None) -> None:
        super().__init__()
        self.field_name = field_name
        self.field = field
        self.selected_file: Path | None = selected_file

        self.input_id = _id_for_field_input(self.field_name)
        self.submit_button = Button("Select File")
        self.remove_button = Button("Remove File")
        self.remove_button.visible = self.selected_file is not None
        self.remove_button.display = self.selected_file is not None

    def compose(self) -> ComposeResult:
        current_filename = str(self.selected_file) if self.selected_file else ""
        yield Input(current_filename, disabled=True, id=self.input_id, placeholder="No file selected")
        yield self.submit_button
        yield self.remove_button

    @work
    async def select_file(self) -> None:
        markdown_filter = Filters(("Markdown", lambda p: p.suffix.lower() == ".md"))
        file_picker = FileOpen(
            open_button="Select",
            default_file=self.selected_file,
            must_exist=True,
            filters=markdown_filter,
        )
        if new_selected_file := await self.app.push_screen_wait(file_picker):
            self.selected_file = new_selected_file
            self.query_one(f"#{self.input_id}", Input).value = str(new_selected_file)

    @on(Button.Pressed)
    async def handle_select_file_pressed(self, press: Button.Pressed) -> None:
        if press.button is self.submit_button:
            self.select_file()
            self.remove_button.visible = True
            self.remove_button.display = True
        elif press.button is self.remove_button:
            self.query_one(f"#{self.input_id}", Input).value = ""
            self.selected_file = None
            self.remove_button.visible = False
            self.remove_button.display = False


class FieldSetting(Container):
    DEFAULT_CSS = """
    FieldSetting {
        layout: grid;
        grid-size: 2;
        height: auto;
    }

    Input {
        width: 70;
    }
    """

    def _field_to_widget(self) -> Widget:
        id = _id_for_field_input(self.field_name)
        if self.field.annotation is bool:
            # If the setting is a boolean, render a on/off switch
            return Switch(value=self.value, id=id)
        elif isinstance(self.field.annotation, type) and issubclass(self.field.annotation, enum.StrEnum):
            # If the setting is an enum, then we'll render a dropdown with all of the available options
            return Select(options=[(t.title(), t) for t in list(self.field.annotation)], value=self.value, id=id)
        elif isinstance(self.field.annotation, type) and issubclass(self.field.annotation, Theme):
            theme_options = [(t.title().replace("-", " "), t) for t in BUILTIN_THEMES.keys()]
            if isinstance(self.value, Theme):
                return Select(options=theme_options, value=self.value.name, id=id)
            else:
                return Select(options=theme_options, value=self.value, id=id)
        elif self.field.annotation == list[str]:
            return Input(value=str(", ".join(self.value)), id=id, validators=[ListOfStringValidator()])
        elif self.field.annotation == Optional[Path]:
            return FileSelector(self.field_name, self.field, self.value)
        else:
            # If no other input mechanism fits, then we'll fallback to just a raw string input field
            return Input(value=str(self.value), id=id)

    def __init__(self, field_name: str, field: FieldInfo, value: Any) -> None:
        super().__init__()
        self.field_name = field_name
        self.field = field
        self.value = value

    def compose(self) -> ComposeResult:
        yield Label(f"[bold]{_field_name_to_readable_name(self.field_name)}:[/bold]")
        yield self._field_to_widget()


class SettingsSection(Vertical):
    DEFAULT_CSS = """
    SettingsSection {
        border: blank white;
        height: auto;
    }

    Static {
        margin-bottom: 1;
    }
    """

    def __init__(self, parent_field_name: str, model: BaseModel) -> None:
        super().__init__()
        self.parent_field_name = parent_field_name
        self.model = model
        self.fields = model.__class__.model_fields

        self.field_settings_widgets: list[FieldSetting] = []

    def filter_field_settings(self, matcher: Matcher | None) -> None:
        at_least_one_displayed = False
        for field_setting in self.field_settings_widgets:
            if matcher is None or matcher.match(field_setting.field_name):
                at_least_one_displayed = True
                field_setting.display = True
            else:
                field_setting.display = False

        self.query_one(Collapsible).collapsed = not at_least_one_displayed

    def compose(self) -> ComposeResult:
        setting_description = self.model.__doc__ or ""
        field_name = f"[bold]{_field_name_to_readable_name(self.parent_field_name)}[/bold]"
        with Collapsible(collapsed=False, title=field_name):
            yield Static(f"{setting_description}".strip())
            for field_name, field_info in self.fields.items():
                if field_info.exclude:
                    continue
                current_value = getattr(self.model, field_name)
                new_field_setting = FieldSetting(field_name, field_info, current_value)
                self.field_settings_widgets.append(new_field_setting)
                yield new_field_setting


class KeySelectionInput(Container):
    DEFAULT_CSS = """
    KeySelectionInput {
        height: 2;
        width: auto;
    }

    KeySelectionInput:focus-within {
        height: 3;
        border: solid $accent;
    }
    """

    def __init__(self, binding: Binding) -> None:
        super().__init__()
        self.binding = binding
        self.key_input = RichLog()

        if binding.id and binding.id in LazyGithubContext.config.bindings.overrides:
            self.key_input.write(LazyGithubContext.config.bindings.overrides[binding.id])
            self.value = LazyGithubContext.config.bindings.overrides[binding.id]
        else:
            self.key_input.write(binding.key)
            self.value = binding.key

    def compose(self) -> ComposeResult:
        with Horizontal():
            yield Label(f"[bold]{self.binding.description or self.binding.id}[/bold]: ")
            yield self.key_input

    async def on_key(self, key_event: Key) -> None:
        if key_event.key not in ["tab", "shift+tab"]:
            key_event.stop()
            self.key_input.clear()
            updated_key = self.binding.key if key_event.key == "escape" else key_event.key
            self.key_input.write(updated_key)
            self.value = updated_key


class BindingsSettingsSection(SettingsSection):
    def __init__(self) -> None:
        super().__init__("bindings", LazyGithubContext.config.bindings)

    def filter_field_settings(self, matcher: Matcher | None) -> None:
        """Overridden filter handler for the bindings settings"""
        at_least_one_displayed = False
        key_selection_inputs = self.query(KeySelectionInput)
        for ksi in key_selection_inputs:
            if (
                # We'll show the binding if there is no query or if the query matches the description/id
                matcher is None
                or (ksi.binding.description and matcher.match(ksi.binding.description))
                or (ksi.binding.id and matcher.match(ksi.binding.id))
            ):
                at_least_one_displayed = True
                ksi.display = True
            else:
                ksi.display = False

        self.query_one(Collapsible).collapsed = not at_least_one_displayed

    def compose(self) -> ComposeResult:
        with Collapsible(collapsed=False, title="[bold]Keybinding Overrides[/bold]"):
            if LazyGithubContext.config.bindings.__doc__:
                yield Static(LazyGithubContext.config.bindings.__doc__)
            bindings_by_id = LazyGithubBindings.all_by_id()
            sorted_binding_keys = sorted(bindings_by_id.keys())
            for key in sorted_binding_keys:
                yield KeySelectionInput(bindings_by_id[key])


class SettingsContainer(Container):
    DEFAULT_CSS = """
    SettingsContainer {
        height: 80%;
        align: center middle;
    }

    #settings_search_input {
        margin-bottom: 1;
        margin-top: 1;
    }

    #settings_buttons {
        width: auto;
        height: auto;
        padding-left: 35;
    }
    """

    BINDINGS = [LazyGithubBindings.SUBMIT_DIALOG, LazyGithubBindings.CLOSE_DIALOG, LazyGithubBindings.SEARCH_DIALOG]

    def __init__(self) -> None:
        super().__init__()
        self.search_input = ToggleableSearchInput(placeholder="Search settings...", id="settings_search_input")
        self.search_input.display = False
        self.search_input.can_focus = False

        self.settings_sections: list[SettingsSection] = []

    def compose(self) -> ComposeResult:
        yield Markdown("# LazyGithub Settings")
        yield self.search_input
        with ScrollableContainer():
            for field, value in LazyGithubContext.config:
                if field == "bindings":
                    yield BindingsSettingsSection()
                else:
                    new_section = SettingsSection(field, value)
                    self.settings_sections.append(new_section)
                    yield new_section

        yield Rule()

        with Horizontal(id="settings_buttons"):
            yield Button("Save", id="save_settings", variant="success")
            yield Button("Cancel", id="cancel_settings", variant="error")

    async def action_search(self) -> None:
        self.search_input.can_focus = True
        self.search_input.display = True
        self.search_input.focus()

    async def change_displayed_settings(self, query: str) -> None:
        all_sections = self.query(SettingsSection)
        matcher = Matcher(query) if query else None
        for section in all_sections:
            section.filter_field_settings(matcher)

    @on(Input.Submitted, "#settings_search_input")
    async def handle_submitted_search(self) -> None:
        search_query = self.search_input.value.strip().lower()
        await self.change_displayed_settings(search_query)

    def _update_settings(self):
        with LazyGithubContext.config.to_edit() as updated_config:
            for section_setting_name, model in updated_config:
                if not isinstance(model, BaseModel) or section_setting_name == "bindings":
                    continue

                for field_name, field_info in model.__class__.model_fields.items():
                    value_adjustment_id = _id_for_field_input(field_name)
                    try:
                        updated_value_input = self.query_one(f"#{value_adjustment_id}")
                    except NoMatches:
                        # If there isn't a way to adjust this setting, skip it
                        continue

                    if not isinstance(updated_value_input, (Switch, Input, Select)):
                        raise TypeError(
                            f"Unexpected value input type: {type(updated_value_input)}. Please file an issue"
                        )

                    # We want to handle paths specially
                    new_value = updated_value_input.value
                    if field_info.annotation == Path | None:
                        new_value = Path(str(new_value)) if new_value else None

                    setattr(model, field_name, new_value)

            # We want to handle the binding settings update differently
            keybinding_adjustments = self.query(KeySelectionInput)
            for adjustment in keybinding_adjustments:
                if adjustment.value != adjustment.binding.key and adjustment.binding.id:
                    LazyGithubContext.config.bindings.overrides[adjustment.binding.id] = adjustment.value
                elif adjustment.binding.id in LazyGithubContext.config.bindings.overrides:
                    del LazyGithubContext.config.bindings.overrides[adjustment.binding.id]

    @on(Button.Pressed, "#save_settings")
    async def save_settings(self, _: Button.Pressed) -> None:
        self._update_settings()
        self.post_message(SettingsModalDismissed(True))

    async def action_submit(self) -> None:
        self._update_settings()
        self.post_message(SettingsModalDismissed(True))

    @on(Button.Pressed, "#cancel_settings")
    async def cancel_settings(self, _: Button.Pressed) -> None:
        self.post_message(SettingsModalDismissed(False))

    async def action_close(self) -> None:
        self.post_message(SettingsModalDismissed(False))


class SettingsModal(ModalScreen):
    DEFAULT_CSS = """
    SettingsModal {
        height: 80%;
        align: center middle;
        content-align: center middle;
    }

    SettingsContainer {
        width: 100;
        height: 50;
        border: thick $background 80%;
        background: $surface-lighten-3;
    }
    """

    def on_settings_modal_dismissed(self, _: SettingsModalDismissed) -> None:
        self.dismiss()

    def compose(self) -> ComposeResult:
        yield SettingsContainer()
        yield LazyGithubFooter()
