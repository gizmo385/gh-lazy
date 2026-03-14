#!/usr/bin/env bash

uv sync
uvx ruff check --select I --fix
uvx ruff check --fix
uvx ty check
