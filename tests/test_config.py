"""Tests for config save/load logic"""

import json
from pathlib import Path

from lazy_github.lib.config import (
    Config,
    PullRequestSettings,
    RepositorySettings,
    _parse_string_list,
    _serialize_string_list,
)


class TestStringListHelpers:
    """Test the helper functions for parsing and serializing string lists"""

    def test_parse_string_list_from_comma_separated_string(self):
        """Test parsing a comma-separated string into a list"""
        result = _parse_string_list(None, "repo1, repo2, repo3")
        # Use set comparison because order is not guaranteed (uses set internally)
        assert set(result) == {"repo1", "repo2", "repo3"}

    def test_parse_string_list_from_list(self):
        """Test that parsing a list returns the list unchanged"""
        input_list = ["repo1", "repo2", "repo3"]
        result = _parse_string_list(None, input_list)
        assert result == input_list

    def test_parse_string_list_with_single_item(self):
        """Test parsing a string with a single item"""
        result = _parse_string_list(None, "single-repo")
        assert result == ["single-repo"]

    def test_parse_string_list_removes_duplicates(self):
        """Test that parsing removes duplicates"""
        result = _parse_string_list(None, "repo1, repo2, repo1, repo3")
        assert set(result) == {"repo1", "repo2", "repo3"}

    def test_parse_string_list_strips_whitespace(self):
        """Test that parsing strips whitespace from items"""
        result = _parse_string_list(None, "  repo1  ,  repo2  ")
        assert set(result) == {"repo1", "repo2"}

    def test_parse_string_list_filters_empty_strings(self):
        """Test that empty strings are filtered out"""
        result = _parse_string_list(None, "repo1, , repo2,  ")
        assert set(result) == {"repo1", "repo2"}

    def test_serialize_string_list_from_list(self):
        """Test serializing a list returns it unchanged"""
        input_list = ["repo1", "repo2", "repo3"]
        result = _serialize_string_list(None, input_list)
        assert result == input_list

    def test_serialize_string_list_from_string(self):
        """Test serializing a comma-separated string into a list"""
        result = _serialize_string_list(None, "repo1, repo2, repo3")
        assert set(result) == {"repo1", "repo2", "repo3"}


class TestRepositorySettings:
    """Test RepositorySettings with string list fields"""

    def test_favorites_accepts_list(self):
        """Test that favorites can be set with a list"""
        settings = RepositorySettings(favorites=["gizmo385/dotfiles", "gizmo385/discord.clj"])
        assert settings.favorites == ["gizmo385/dotfiles", "gizmo385/discord.clj"]

    def test_favorites_accepts_comma_separated_string(self):
        """Test that favorites can be set with a comma-separated string"""
        # We want to test these string assignments intentionally incase the user config gets mangled somehow
        settings = RepositorySettings(
            favorites="gizmo385/dotfiles, gizmo385/discord.clj"  # type: ignore[arg-type]
        )
        assert set(settings.favorites) == {"gizmo385/dotfiles", "gizmo385/discord.clj"}

    def test_favorites_single_item_list(self):
        """Test favorites with a single item in a list"""
        settings = RepositorySettings(favorites=["gizmo385/dotfiles"])
        assert settings.favorites == ["gizmo385/dotfiles"]

    def test_favorites_single_item_string(self):
        """Test favorites with a single item as a string"""
        # We want to test these string assignments intentionally incase the user config gets mangled somehow
        settings = RepositorySettings(favorites="gizmo385/dotfiles")  # type: ignore[arg-type]
        assert settings.favorites == ["gizmo385/dotfiles"]

    def test_favorites_empty_list(self):
        """Test favorites with an empty list"""
        settings = RepositorySettings(favorites=[])
        assert settings.favorites == []

    def test_additional_repos_accepts_list(self):
        """Test that additional_repos_to_track can be set with a list"""
        settings = RepositorySettings(additional_repos_to_track=["org/repo1", "org/repo2"])
        assert settings.additional_repos_to_track == ["org/repo1", "org/repo2"]

    def test_additional_repos_accepts_comma_separated_string(self):
        """Test that additional_repos_to_track can be set with a comma-separated string"""
        # We want to test these string assignments intentionally incase the user config gets mangled somehow
        settings = RepositorySettings(additional_repos_to_track="org/repo1, org/repo2")  # type: ignore[arg-type]
        assert set(settings.additional_repos_to_track) == {"org/repo1", "org/repo2"}

    def test_model_dump_preserves_list_type(self):
        """Test that model_dump returns favorites as a list, not a string"""
        settings = RepositorySettings(favorites=["gizmo385/dotfiles", "gizmo385/discord.clj"])
        dumped = settings.model_dump()
        assert isinstance(dumped["favorites"], list)
        assert set(dumped["favorites"]) == {"gizmo385/dotfiles", "gizmo385/discord.clj"}


class TestPullRequestSettings:
    """Test PullRequestSettings with string list fields"""

    def test_additional_reviewers_accepts_list(self):
        """Test that additional_suggested_pr_reviewers can be set with a list"""
        settings = PullRequestSettings(additional_suggested_pr_reviewers=["user1", "user2"])
        assert settings.additional_suggested_pr_reviewers == ["user1", "user2"]

    def test_additional_reviewers_accepts_comma_separated_string(self):
        """Test that additional_suggested_pr_reviewers can be set with a comma-separated string"""
        # We want to test these string assignments intentionally incase the user config gets mangled somehow
        settings = PullRequestSettings(additional_suggested_pr_reviewers="user1, user2")  # type: ignore[arg-type]
        assert set(settings.additional_suggested_pr_reviewers) == {"user1", "user2"}


class TestConfigSaveLoad:
    """Test the full config save/load cycle"""

    def test_config_saves_and_loads_favorites_correctly(self, tmp_path: Path):
        """Test that saving and loading config preserves favorites as a list"""
        config_file = tmp_path / "config.json"

        # Create a config with favorites
        config = Config()
        config.repositories.favorites = [
            "gizmo385/dotfiles",
            "gizmo385/discord.clj",
            "Textualize/textual",
        ]
        config_file.write_text(config.model_dump_json(indent=4))

        # Load from file
        loaded_data = json.loads(config_file.read_text())
        loaded_config = Config(**loaded_data)

        # Verify favorites is still a list
        assert isinstance(loaded_config.repositories.favorites, list)
        assert set(loaded_config.repositories.favorites) == {
            "gizmo385/dotfiles",
            "gizmo385/discord.clj",
            "Textualize/textual",
        }

    def test_config_json_structure(self, tmp_path: Path):
        """Test that the JSON file has the correct structure for favorites"""
        config_file = tmp_path / "config.json"

        # Create a config with favorites
        config = Config()
        config.repositories.favorites = ["gizmo385/dotfiles", "gizmo385/discord.clj"]
        config_file.write_text(config.model_dump_json(indent=4))

        # Read the raw JSON
        json_data = json.loads(config_file.read_text())

        # Verify favorites is a list in the JSON
        assert isinstance(json_data["repositories"]["favorites"], list)
        assert set(json_data["repositories"]["favorites"]) == {
            "gizmo385/dotfiles",
            "gizmo385/discord.clj",
        }

    def test_config_roundtrip_preserves_type(self, tmp_path: Path):
        """Test that a full save/load cycle preserves the list type"""
        config_file = tmp_path / "config.json"

        # Create a config
        original_config = Config()
        original_config.repositories.favorites = [
            "gizmo385/dotfiles",
            "gizmo385/discord.clj",
        ]
        config_file.write_text(original_config.model_dump_json(indent=4))

        # Load
        loaded_config = Config(**json.loads(config_file.read_text()))

        # Verify type is preserved
        assert isinstance(loaded_config.repositories.favorites, list)
        assert loaded_config.repositories.favorites == original_config.repositories.favorites

    def test_config_handles_single_item_list(self, tmp_path: Path):
        """Test that a single-item list is handled correctly"""
        config_file = tmp_path / "config.json"

        # Create a config with a single favorite
        config = Config()
        config.repositories.favorites = ["gizmo385/dotfiles"]
        config_file.write_text(config.model_dump_json(indent=4))
        loaded_config = Config(**json.loads(config_file.read_text()))

        # Verify it's still a list with one item
        assert isinstance(loaded_config.repositories.favorites, list)
        assert loaded_config.repositories.favorites == ["gizmo385/dotfiles"]

    def test_config_handles_manually_edited_json_with_string(self, tmp_path: Path):
        """Test that if someone manually edits the JSON to have a string, it's converted to a list"""
        config_file = tmp_path / "config.json"

        # Create a manually edited JSON with a string instead of a list
        manual_json = {
            "repositories": {
                "favorites": "gizmo385/dotfiles, gizmo385/discord.clj",
                "additional_repos_to_track": [],
            }
        }
        config_file.write_text(json.dumps(manual_json, indent=4))

        # Load the config
        loaded_config = Config(**json.loads(config_file.read_text()))

        # Verify it's converted to a list
        assert isinstance(loaded_config.repositories.favorites, list)
        assert set(loaded_config.repositories.favorites) == {
            "gizmo385/dotfiles",
            "gizmo385/discord.clj",
        }

    def test_config_update_with_string_converts_to_list(self):
        """Test that updating a config field with a string converts it to a list"""
        config = Config()

        # We want to test these string assignments intentionally incase the user config gets mangled somehow
        config.repositories.favorites = "gizmo385/dotfiles, gizmo385/discord.clj"  # type: ignore[assignment]

        # Verify it's converted to a list
        assert isinstance(config.repositories.favorites, list)
        assert set(config.repositories.favorites) == {
            "gizmo385/dotfiles",
            "gizmo385/discord.clj",
        }

    def test_config_serialization_after_string_update(self, tmp_path: Path):
        """Test that after updating with a string, serialization produces a list"""
        config_file = tmp_path / "config.json"
        config = Config()

        # We want to test these string assignments intentionally incase the user config gets mangled somehow
        config.repositories.favorites = "gizmo385/dotfiles"  # type: ignore[assignment]
        config_file.write_text(config.model_dump_json(indent=4))

        # Check the JSON
        json_data = json.loads(config_file.read_text())
        assert isinstance(json_data["repositories"]["favorites"], list)
        assert json_data["repositories"]["favorites"] == ["gizmo385/dotfiles"]
