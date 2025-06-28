#!/usr/bin/env bash

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
cd "$SCRIPT_DIR"
uv sync --quiet --frozen
TERM=xterm-256color .venv/bin/python -m lazy_github "$@"
