#!/usr/bin/env bash

uv sync --quiet
uvx ruff check --select I --fix --quiet
uvx ruff check --fix --quiet
uvx ty check --quiet
