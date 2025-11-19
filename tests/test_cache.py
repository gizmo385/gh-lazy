"""Tests for the cache module's resilient error handling."""

import json
import tempfile
from pathlib import Path

import pytest
from pydantic import BaseModel

from lazy_github.lib.cache import load_models_from_cache, save_models_to_cache
from lazy_github.lib.context import LazyGithubContext
from lazy_github.models.github import Repository, User


class SimpleModel(BaseModel):
    """A simple test model."""

    id: int
    name: str


class ExtendedModel(BaseModel):
    """An extended model with additional required fields to simulate schema changes."""

    id: int
    name: str
    new_required_field: str


class TestCacheResilience:
    """Tests for cache resilience when models change or cache is corrupted."""

    @pytest.fixture
    def temp_cache_dir(self):
        """Create a temporary cache directory."""
        with tempfile.TemporaryDirectory() as tmpdir:
            original_cache_dir = LazyGithubContext.config.cache.cache_directory
            LazyGithubContext.config.cache.cache_directory = Path(tmpdir)
            yield Path(tmpdir)
            LazyGithubContext.config.cache.cache_directory = original_cache_dir

    def test_load_from_valid_cache(self, temp_cache_dir):
        """Test that valid cached data loads successfully."""
        # Save some test data
        test_data = [
            SimpleModel(id=1, name="first"),
            SimpleModel(id=2, name="second"),
        ]
        save_models_to_cache(None, "test_cache", test_data)

        # Load it back
        loaded = load_models_from_cache(None, "test_cache", SimpleModel)

        assert len(loaded) == 2
        assert loaded[0].id == 1
        assert loaded[0].name == "first"
        assert loaded[1].id == 2
        assert loaded[1].name == "second"

    def test_load_with_missing_required_field(self, temp_cache_dir):
        """Test that cache loading gracefully fails when model schema changes."""
        # Save data with the old simple model
        test_data = [
            SimpleModel(id=1, name="first"),
            SimpleModel(id=2, name="second"),
        ]
        save_models_to_cache(None, "test_cache", test_data)

        # Try to load with extended model that has new required fields
        # This should return empty list instead of crashing
        loaded = load_models_from_cache(None, "test_cache", ExtendedModel)

        assert len(loaded) == 0

    def test_load_with_corrupted_json(self, temp_cache_dir):
        """Test that corrupted JSON is handled gracefully."""
        # Manually create a corrupted cache file
        cache_file = temp_cache_dir / "test_cache.json"
        cache_file.write_text("{ this is not valid json }")

        # Should return empty list instead of crashing
        loaded = load_models_from_cache(None, "test_cache", SimpleModel)

        assert len(loaded) == 0

    def test_load_with_invalid_data_types(self, temp_cache_dir):
        """Test that invalid data types are handled gracefully."""
        # Manually create a cache file with wrong data types
        cache_file = temp_cache_dir / "test_cache.json"
        cache_file.write_text(json.dumps([{"id": "not_an_int", "name": "test"}]))

        # Should return empty list instead of crashing
        loaded = load_models_from_cache(None, "test_cache", SimpleModel)

        assert len(loaded) == 0

    def test_load_nonexistent_cache(self, temp_cache_dir):
        """Test that loading from nonexistent cache returns empty list."""
        loaded = load_models_from_cache(None, "nonexistent_cache", SimpleModel)

        assert len(loaded) == 0

    def test_load_with_partial_corruption(self, temp_cache_dir):
        """Test that even one invalid item causes entire cache to be ignored."""
        # Create cache with one valid and one invalid item
        cache_file = temp_cache_dir / "test_cache.json"
        cache_file.write_text(
            json.dumps([{"id": 1, "name": "valid"}, {"id": "invalid", "name": "bad"}])
        )

        # Should return empty list because validation fails on second item
        loaded = load_models_from_cache(None, "test_cache", SimpleModel)

        assert len(loaded) == 0

    def test_repo_based_cache(self, temp_cache_dir):
        """Test that repository-based cache works correctly."""
        owner = User(
            login="owner",
            id=456,
            html_url="https://github.com/owner",
        )
        repo = Repository(
            name="test-repo",
            full_name="owner/test-repo",
            private=False,
            owner=owner,
        )

        # Save and load with repo
        test_data = [SimpleModel(id=1, name="repo_data")]
        save_models_to_cache(repo, "test_cache", test_data)

        loaded = load_models_from_cache(repo, "test_cache", SimpleModel)

        assert len(loaded) == 1
        assert loaded[0].name == "repo_data"

        # Verify the filename uses underscore instead of slash
        expected_filename = "owner_test-repo_test_cache.json"
        cache_file = temp_cache_dir / expected_filename
        assert cache_file.exists()

    def test_empty_cache_file(self, temp_cache_dir):
        """Test that an empty JSON array is handled correctly."""
        cache_file = temp_cache_dir / "test_cache.json"
        cache_file.write_text("[]")

        loaded = load_models_from_cache(None, "test_cache", SimpleModel)

        assert len(loaded) == 0
