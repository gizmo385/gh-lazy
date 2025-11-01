#!/usr/bin/env bash

uv sync
uv run ruff check --select I --fix
uv run ruff check --fix
uv run pyright
