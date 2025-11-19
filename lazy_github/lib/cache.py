import json
from typing import Iterable, TypeVar

from pydantic import BaseModel, ValidationError

from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.logging import lg
from lazy_github.models.github import Repository

T = TypeVar("T", bound=BaseModel)


def load_models_from_cache(repo: Repository | None, cache_name: str, expect_type: type[T]) -> list[T]:
    """Loads information from a file cache where the cached information changes based on the repository

    Returns an empty list if the cache file doesn't exist, is corrupted, or contains data that
    doesn't match the expected model schema. This allows graceful fallback to fetching real data.
    """
    results: list[T] = []
    if repo:
        filename = f"{repo.full_name.replace('/', '_')}_{cache_name}.json"
    else:
        filename = f"{cache_name}.json"
    cache_path = LazyGithubContext.config.cache.cache_directory / filename
    lg.debug(f"Loading cached data from: {cache_path}")

    if cache_path.is_file():
        try:
            cached_objects = json.loads(cache_path.read_text())
            # Parse all objects first - if any fail, we'll return empty list
            temp_results: list[T] = []
            for raw_obj in cached_objects:
                temp_results.append(expect_type(**raw_obj))
            # Only update results if ALL items parsed successfully
            results = temp_results
        except json.JSONDecodeError as e:
            lg.warning(
                f"Failed to parse cache file '{cache_path}' as valid JSON: {e}. "
                f"Ignoring cache and will fetch fresh data."
            )
        except ValidationError as e:
            lg.warning(
                f"Cache file '{cache_path}' contains data that doesn't match the expected "
                f"schema for {expect_type.__name__}. This can happen when model fields change. "
                f"Ignoring cache and will fetch fresh data. Error: {e}"
            )
        except Exception as e:
            lg.warning(
                f"Unexpected error loading cache from '{cache_path}': {e}. "
                f"Ignoring cache and will fetch fresh data."
            )
    return results


def save_models_to_cache(repo: Repository | None, cache_name: str, objects: Iterable[T]) -> None:
    """Stores information in a file cache where the cached information changes based on the repository"""
    if repo:
        filename = f"{repo.full_name.replace('/', '_')}_{cache_name}.json"
    else:
        filename = f"{cache_name}.json"
    cache_path = LazyGithubContext.config.cache.cache_directory / filename
    lg.debug(f"Saving cached data to: {cache_path}")

    cache_path.parent.mkdir(parents=True, exist_ok=True)
    cache_path.touch(exist_ok=True)
    cache_path.write_text(json.dumps([o.model_dump(mode="json") for o in objects]))
