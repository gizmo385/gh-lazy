"""Tests for config save/load logic"""

import json
import tempfile
from pathlib import Path

from lazy_github.lib.config import (
    Config,
    PullRequestSettings,
    RepositorySettings,
)


class TestRepositorySettings:
    """Test RepositorySettings with string list fields"""

    def test_favorites_accepts_list(self):
        """Test that favorites can be set with a list"""
        settings = RepositorySettings(favorites=["gizmo385/dotfiles", "gizmo385/discord.clj"])
        assert settings.favorites == ["gizmo385/dotfiles", "gizmo385/discord.clj"]

    def test_favorites_single_item_list(self):
        """Test favorites with a single item in a list"""
        settings = RepositorySettings(favorites=["gizmo385/dotfiles"])
        assert settings.favorites == ["gizmo385/dotfiles"]

    def test_favorites_empty_list(self):
        """Test favorites with an empty list"""
        settings = RepositorySettings(favorites=[])
        assert settings.favorites == []

    def test_additional_repos_accepts_list(self):
        """Test that additional_repos_to_track can be set with a list"""
        settings = RepositorySettings(additional_repos_to_track=["org/repo1", "org/repo2"])
        assert settings.additional_repos_to_track == ["org/repo1", "org/repo2"]

    def test_model_dump_preserves_list_type(self):
        """Test that model_dump returns favorites as a list, not a string"""
        settings = RepositorySettings(favorites=["gizmo385/dotfiles", "gizmo385/discord.clj"])
        dumped = settings.model_dump()
        assert isinstance(dumped["favorites"], list)
        assert dumped["favorites"] == ["gizmo385/dotfiles", "gizmo385/discord.clj"]


class TestPullRequestSettings:
    """Test PullRequestSettings with string list fields"""

    def test_additional_reviewers_accepts_list(self):
        """Test that additional_suggested_pr_reviewers can be set with a list"""
        settings = PullRequestSettings(additional_suggested_pr_reviewers=["user1", "user2"])
        assert settings.additional_suggested_pr_reviewers == ["user1", "user2"]


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
        assert loaded_config.repositories.favorites == [
            "gizmo385/dotfiles",
            "gizmo385/discord.clj",
            "Textualize/textual",
        ]

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
        assert json_data["repositories"]["favorites"] == [
            "gizmo385/dotfiles",
            "gizmo385/discord.clj",
        ]

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


class TestListSerializationDeserialization:
    """Test list[str] field serialization and deserialization"""

    def test_repository_favorites_serialization(self):
        """Test that favorite repos serialize correctly to JSON arrays"""
        repo_settings = RepositorySettings(favorites=["user/repo1", "user/repo2", "user/repo3"])

        # Serialize to JSON
        json_data = repo_settings.model_dump_json()
        data = json.loads(json_data)

        # Verify it's a proper JSON array
        assert isinstance(data["favorites"], list)
        assert data["favorites"] == ["user/repo1", "user/repo2", "user/repo3"]

    def test_repository_favorites_deserialization(self):
        """Test that favorite repos deserialize correctly from JSON arrays"""
        json_data = {"favorites": ["user/repo1", "user/repo2", "user/repo3"], "additional_repos_to_track": []}

        repo_settings = RepositorySettings(**json_data)

        assert isinstance(repo_settings.favorites, list)
        assert repo_settings.favorites == ["user/repo1", "user/repo2", "user/repo3"]

    def test_additional_repos_to_track_serialization(self):
        """Test that additional repos serialize correctly to JSON arrays"""
        repo_settings = RepositorySettings(additional_repos_to_track=["org/repo1", "org/repo2"])

        json_data = repo_settings.model_dump_json()
        data = json.loads(json_data)

        assert isinstance(data["additional_repos_to_track"], list)
        assert data["additional_repos_to_track"] == ["org/repo1", "org/repo2"]

    def test_additional_repos_to_track_deserialization(self):
        """Test that additional repos deserialize correctly from JSON arrays"""
        json_data = {"favorites": [], "additional_repos_to_track": ["org/repo1", "org/repo2"]}

        repo_settings = RepositorySettings(**json_data)

        assert isinstance(repo_settings.additional_repos_to_track, list)
        assert repo_settings.additional_repos_to_track == ["org/repo1", "org/repo2"]

    def test_pr_reviewers_serialization(self):
        """Test that PR reviewers serialize correctly to JSON arrays"""
        pr_settings = PullRequestSettings(additional_suggested_pr_reviewers=["reviewer1", "reviewer2", "reviewer3"])

        json_data = pr_settings.model_dump_json()
        data = json.loads(json_data)

        assert isinstance(data["additional_suggested_pr_reviewers"], list)
        assert data["additional_suggested_pr_reviewers"] == ["reviewer1", "reviewer2", "reviewer3"]

    def test_pr_reviewers_deserialization(self):
        """Test that PR reviewers deserialize correctly from JSON arrays"""
        json_data = {"additional_suggested_pr_reviewers": ["reviewer1", "reviewer2", "reviewer3"]}

        pr_settings = PullRequestSettings.model_validate(json_data)

        assert isinstance(pr_settings.additional_suggested_pr_reviewers, list)
        assert pr_settings.additional_suggested_pr_reviewers == ["reviewer1", "reviewer2", "reviewer3"]

    def test_empty_list_serialization(self):
        """Test that empty lists serialize correctly"""
        repo_settings = RepositorySettings(favorites=[], additional_repos_to_track=[])

        json_data = repo_settings.model_dump_json()
        data = json.loads(json_data)

        assert data["favorites"] == []
        assert data["additional_repos_to_track"] == []

    def test_empty_list_deserialization(self):
        """Test that empty lists deserialize correctly"""
        json_data = {"favorites": [], "additional_repos_to_track": []}

        repo_settings = RepositorySettings(**json_data)

        assert repo_settings.favorites == []
        assert repo_settings.additional_repos_to_track == []

    def test_special_characters_in_list(self):
        """Test that special characters in list items are handled correctly"""
        special_items = ["user/repo-with-dashes", "user/repo_with_underscores", "user/repo.with.dots", "user/repo123"]

        repo_settings = RepositorySettings(favorites=special_items)

        # Serialize and deserialize
        json_data = repo_settings.model_dump_json()
        data = json.loads(json_data)

        assert data["favorites"] == special_items

        # Deserialize back
        repo_settings_2 = RepositorySettings(**data)
        assert repo_settings_2.favorites == special_items

    def test_no_nested_quotes_in_serialization(self):
        """Test that serialization doesn't create nested quotes (the bug we're fixing)"""
        items = ["person1", "person2", "person3"]
        pr_settings = PullRequestSettings(additional_suggested_pr_reviewers=items)

        # Serialize
        json_data = pr_settings.model_dump_json()
        data = json.loads(json_data)

        # Verify no nested quotes or malformed strings
        for item in data["additional_suggested_pr_reviewers"]:
            assert isinstance(item, str)
            assert not item.startswith('"')
            assert not item.endswith('"')
            assert "[" not in item
            assert "]" not in item
            assert "'" not in item


class TestFullConfigSerialization:
    """Test full Config object serialization and deserialization"""

    def test_full_config_save_and_load(self):
        """Test that a full config can be saved and loaded correctly"""
        with tempfile.TemporaryDirectory() as tmpdir:
            config_file = Path(tmpdir) / "config.json"

            # Create a config with list settings
            config = Config()
            config.repositories.favorites = ["user/repo1", "user/repo2"]
            config.repositories.additional_repos_to_track = ["org/repo1"]
            config.pull_requests.additional_suggested_pr_reviewers = ["reviewer1", "reviewer2"]

            # Save to file
            config_file.write_text(config.model_dump_json(indent=4))

            # Load from file
            loaded_config = Config(**json.loads(config_file.read_text()))

            # Verify lists are correct
            assert loaded_config.repositories.favorites == ["user/repo1", "user/repo2"]
            assert loaded_config.repositories.additional_repos_to_track == ["org/repo1"]
            assert loaded_config.pull_requests.additional_suggested_pr_reviewers == ["reviewer1", "reviewer2"]

    def test_config_with_empty_lists(self):
        """Test that config with empty lists saves and loads correctly"""
        with tempfile.TemporaryDirectory() as tmpdir:
            config_file = Path(tmpdir) / "config.json"

            config = Config()
            config.repositories.favorites = []
            config.repositories.additional_repos_to_track = []
            config.pull_requests.additional_suggested_pr_reviewers = []

            config_file.write_text(config.model_dump_json(indent=4))
            loaded_config = Config(**json.loads(config_file.read_text()))

            assert loaded_config.repositories.favorites == []
            assert loaded_config.repositories.additional_repos_to_track == []
            assert loaded_config.pull_requests.additional_suggested_pr_reviewers == []

    def test_json_file_format(self):
        """Test that the saved JSON file has proper array format"""
        with tempfile.TemporaryDirectory() as tmpdir:
            config_file = Path(tmpdir) / "config.json"

            config = Config()
            config.repositories.favorites = ["user/repo1", "user/repo2"]

            config_file.write_text(config.model_dump_json(indent=4))

            # Read as text and verify format
            file_content = config_file.read_text()

            # Should contain proper JSON array syntax
            assert '"favorites": [' in file_content
            assert '"user/repo1"' in file_content
            assert '"user/repo2"' in file_content

            # Should NOT contain comma-separated string format
            assert '"favorites": "user/repo1, user/repo2"' not in file_content

    def test_roundtrip_preserves_list_order(self):
        """Test that list order is preserved through save/load cycles"""
        items = ["item3", "item1", "item2", "item4"]

        config = Config()
        config.repositories.favorites = items

        # Serialize and deserialize
        json_str = config.model_dump_json()
        loaded_config = Config(**json.loads(json_str))

        # Order should be preserved
        assert loaded_config.repositories.favorites == items

    def test_roundtrip_preserves_duplicates_if_added_manually(self):
        """Test behavior with duplicate items (shouldn't happen but test edge case)"""
        # Pydantic should preserve duplicates if they're in the list
        pr_settings = PullRequestSettings(additional_suggested_pr_reviewers=["user1", "user2", "user1"])

        json_str = pr_settings.model_dump_json()
        loaded_settings = PullRequestSettings(**json.loads(json_str))

        # Should preserve what was given
        assert loaded_settings.additional_suggested_pr_reviewers == ["user1", "user2", "user1"]
