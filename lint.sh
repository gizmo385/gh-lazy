#!/usr/bin/env bash

uv sync --quiet
uvx ruff check --select I --fix
uvx ruff check --fix
uvx pyrefly check
