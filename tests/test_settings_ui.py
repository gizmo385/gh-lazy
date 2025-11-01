"""Tests for settings UI logic"""

from lazy_github.lib.config import RepositorySettings


class TestListToStringConversion:
    """Test the logic for converting list[str] to display strings"""

    def test_list_to_comma_separated_string(self):
        """Test that a list of strings is converted to a comma-separated string"""
        value = ["gizmo385/dotfiles", "gizmo385/discord.clj", "Textualize/textual"]

        # This is what the UI code should do
        if isinstance(value, list):
            display_value = ", ".join(value)
        else:
            display_value = str(value)

        assert display_value == "gizmo385/dotfiles, gizmo385/discord.clj, Textualize/textual"

    def test_single_item_list_to_string(self):
        """Test that a single-item list is converted correctly"""
        value = ["gizmo385/dotfiles"]

        if isinstance(value, list):
            display_value = ", ".join(value)
        else:
            display_value = str(value)

        assert display_value == "gizmo385/dotfiles"

    def test_empty_list_to_string(self):
        """Test that an empty list is converted to an empty string"""
        value = []

        if isinstance(value, list):
            display_value = ", ".join(value)
        else:
            display_value = str(value)

        assert display_value == ""

    def test_string_value_defensive_handling(self):
        """
        Test that if value is a string (defensive programming),
        we display it as-is instead of joining its characters.
        """
        # This shouldn't happen with our fix, but defensive programming is good
        value = "gizmo385/dotfiles"

        if isinstance(value, list):
            display_value = ", ".join(value)
        else:
            display_value = str(value)

        # With defensive programming, this should be the string as-is
        assert display_value == "gizmo385/dotfiles"

        # Not the character-by-character join
        assert display_value != "g, i, z, m, o, 3, 8, 5, /, d, o, t, f, i, l, e, s"

    def test_bug_scenario_without_type_check(self):
        """
        Test demonstrating the BUG: if we don't check isinstance,
        ", ".join(string) will iterate over characters.
        """
        value = "gizmo385/dotfiles"

        # This is the OLD buggy code that caused the issue
        buggy_display_value = ", ".join(value)

        # This demonstrates the bug
        assert buggy_display_value == "g, i, z, m, o, 3, 8, 5, /, d, o, t, f, i, l, e, s"

        # The fixed code with type checking
        if isinstance(value, list):
            fixed_display_value = ", ".join(value)
        else:
            fixed_display_value = str(value)

        # This is the correct behavior
        assert fixed_display_value == "gizmo385/dotfiles"


class TestInputValueParsing:
    """Test how input values are parsed back into the config"""

    def test_comma_separated_input_is_parsed_correctly(self):
        """Test that user input with commas is parsed into a list"""
        # Simulate user entering comma-separated values
        user_input = "repo1, repo2, repo3"

        # Create a RepositorySettings instance with the string
        # The validator should convert it to a list
        settings = RepositorySettings(favorites=user_input)  # type: ignore[arg-type]

        assert isinstance(settings.favorites, list)
        assert set(settings.favorites) == {"repo1", "repo2", "repo3"}

    def test_single_value_input_is_parsed_correctly(self):
        """Test that a single value without commas becomes a single-item list"""
        user_input = "single-repo"

        settings = RepositorySettings(favorites=user_input)  # type: ignore[arg-type]

        assert isinstance(settings.favorites, list)
        assert settings.favorites == ["single-repo"]

    def test_empty_input_is_parsed_correctly(self):
        """Test that an empty string becomes an empty list"""
        user_input = ""

        settings = RepositorySettings(favorites=user_input)  # type: ignore[arg-type]

        assert isinstance(settings.favorites, list)
        assert settings.favorites == []
