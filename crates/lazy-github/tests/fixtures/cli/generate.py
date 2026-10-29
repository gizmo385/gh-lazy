"""Generates the CLI golden for the Rust `lazy-github` binary's subcommand tests (#199, #195).

Run from the repository root with:

    uv run python crates/lazy-github/tests/fixtures/cli/generate.py

Each case runs `python -m lazy_github <args>` in a fresh temporary `HOME`, with a cleared environment, the case's
config file (if any), the case's files under the cache directory (if any) and the case's stdin (`null` means stdin is
closed at once, so a prompt reads end-of-file). It records stdout, stderr, the exit code, and whether `config.json` and
the cache directory exist afterwards. The temporary home is written as `<HOME>` and the app's version as `<VERSION>`, so
the golden doesn't depend on where it ran, and the Rust test substitutes its own home and `CARGO_PKG_VERSION`.

The log file every Python command creates (`lazy_github.log`) isn't recorded: the Rust subcommands don't start logging.

`debug` prints a "Library Versions" list that differs on purpose (decision `debug-info-versions`); the Rust test
compares its output from the `# LazyGithub Config` heading on, and the first two lines. `bug_report_template` is the
template `Open Bug Report` fills, with `{debug_info}` where the debug text goes.

`--out DIR` writes `python-cli.json` to DIR instead of this directory. The `cli_goldens_are_fresh` test uses it.
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

from lazy_github.lib.debug import _BUG_REPORT_TEMPLATE
from lazy_github.version import VERSION

HERE = Path(__file__).parent
REPO = HERE.parents[4]
# The config golden's every-field config, with the log file moved under the home: Python's file logger prints a
# traceback to stderr when it can't write `/var/log/lazy-github/app.log`, and the Rust subcommands don't log.
FULL_CONFIG = json.loads((REPO / "crates/lazy-github-core/tests/fixtures/config/python-full.json").read_text())
FULL_CONFIG["core"]["logfile_path"] = "<HOME>/.config/lazy-github/app.log"

# Strings that Rich's JSON highlighter could color as something other than a string: digits, `true`, `null` and
# brackets inside a value, and non-ASCII text (printed as is).
TRICKY_CONFIG = {
    "appearance": {"theme": "nord"},
    "repositories": {"favorites": ["owner/12", "true/null", "a{b}[c]", "café/ünïcode"]},
    "bindings": {"overrides": {"app.quit": "ctrl+q"}},
    "cache": {"auth_last_checked": "2026-10-24T18:30:05.123456", "default_ttl": 0},
}

# Every case: a name, the arguments, and optionally `config` (a JSON value written as config.json, with `<HOME>`
# replaced by the home), `cache_files` (paths under the cache directory to create), `stdin` (text, or None for a closed
# stdin) and `env` (added to the cleared environment).
CASES: list[dict[str, Any]] = [
    {"name": "version", "args": ["version"]},
    {"name": "dump-config-no-file", "args": ["dump-config"]},
    {"name": "dump-config-full", "args": ["dump-config"], "config": FULL_CONFIG},
    {"name": "dump-config-tricky", "args": ["dump-config"], "config": TRICKY_CONFIG},
    {"name": "dump-config-color", "args": ["dump-config"], "config": TRICKY_CONFIG, "env": {"FORCE_COLOR": "1"}},
    {
        "name": "dump-config-color-no-color",
        "args": ["dump-config"],
        "config": TRICKY_CONFIG,
        "env": {"FORCE_COLOR": "1", "NO_COLOR": "1"},
    },
    {"name": "debug-no-file", "args": ["debug"]},
    {"name": "debug-full", "args": ["debug"], "config": FULL_CONFIG},
    {"name": "debug-tricky", "args": ["debug"], "config": TRICKY_CONFIG},
    {"name": "clear-config-no-confirm", "args": ["clear-config", "--no-confirm"], "config": TRICKY_CONFIG},
    {"name": "clear-config-no-confirm-no-file", "args": ["clear-config", "--no-confirm"]},
    {"name": "clear-config-yes", "args": ["clear-config"], "config": TRICKY_CONFIG, "stdin": "y\n"},
    {"name": "clear-config-yes-word", "args": ["clear-config"], "config": TRICKY_CONFIG, "stdin": "YES\n"},
    {"name": "clear-config-no", "args": ["clear-config"], "config": TRICKY_CONFIG, "stdin": "n\n"},
    {"name": "clear-config-no-word", "args": ["clear-config"], "config": TRICKY_CONFIG, "stdin": " No \n"},
    {"name": "clear-config-empty-answer", "args": ["clear-config"], "config": TRICKY_CONFIG, "stdin": "\n"},
    {"name": "clear-config-eof", "args": ["clear-config"], "config": TRICKY_CONFIG, "stdin": None},
    {"name": "clear-config-eof-after-text", "args": ["clear-config"], "config": TRICKY_CONFIG, "stdin": "y"},
    {"name": "clear-config-invalid-then-yes", "args": ["clear-config"], "config": TRICKY_CONFIG, "stdin": "maybe\ny\n"},
    {"name": "clear-config-invalid-then-eof", "args": ["clear-config"], "config": TRICKY_CONFIG, "stdin": "maybe\n"},
    {"name": "clear-config-yes-no-file", "args": ["clear-config"], "stdin": "y\n"},
    {
        "name": "clear-cache-no-confirm",
        "args": ["clear-cache", "--no-confirm"],
        "cache_files": ["a.json", "sub/b.json"],
    },
    {"name": "clear-cache-no-confirm-no-dir", "args": ["clear-cache", "--no-confirm"]},
    {"name": "clear-cache-yes", "args": ["clear-cache"], "cache_files": ["a.json"], "stdin": "y\n"},
    {"name": "clear-cache-no", "args": ["clear-cache"], "cache_files": ["a.json"], "stdin": "n\n"},
    {"name": "clear-cache-empty-answer", "args": ["clear-cache"], "cache_files": ["a.json"], "stdin": "\n"},
    {"name": "clear-cache-eof", "args": ["clear-cache"], "cache_files": ["a.json"], "stdin": None},
    {"name": "clear-cache-invalid-then-no", "args": ["clear-cache"], "cache_files": ["a.json"], "stdin": "x\nn\n"},
    {
        "name": "clear-cache-configured-dir",
        "args": ["clear-cache"],
        "config": {"cache": {"cache_directory": "<HOME>/elsewhere"}},
        "cache_files": ["a.json"],
        "stdin": "y\n",
    },
]


def config_dir(home: Path) -> Path:
    return home / ".config" / "lazy-github"


def cache_dir(home: Path, config: Any) -> Path:
    if isinstance(config, dict) and "cache_directory" in config.get("cache", {}):
        return Path(config["cache"]["cache_directory"].replace("<HOME>", str(home)))
    return config_dir(home) / ".cache"


def run_case(case: dict[str, Any]) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="lazy-github-cli-golden-") as tmp:
        home = Path(tmp).resolve()
        config = case.get("config")
        config_file = config_dir(home) / "config.json"
        if config is not None:
            config_file.parent.mkdir(parents=True, exist_ok=True)
            config_file.write_text(json.dumps(config).replace("<HOME>", str(home)))
        cache = cache_dir(home, config)
        for name in case.get("cache_files", []):
            (cache / name).parent.mkdir(parents=True, exist_ok=True)
            (cache / name).write_text("{}")

        env = {"HOME": str(home), "PATH": os.environ["PATH"], "LANG": "C.UTF-8", "TERM": "xterm-256color"}
        env.update(case.get("env", {}))
        stdin = case.get("stdin")
        result = subprocess.run(
            [sys.executable, "-m", "lazy_github", *case["args"]],
            input=stdin.encode() if stdin is not None else None,
            stdin=subprocess.DEVNULL if stdin is None else None,
            capture_output=True,
            env=env,
            cwd=home,
            check=False,
        )

        def scrub(data: bytes) -> str:
            return data.decode().replace(str(home), "<HOME>").replace(VERSION, "<VERSION>")

        return {
            "name": case["name"],
            "args": case["args"],
            "config": config,
            "cache_files": case.get("cache_files", []),
            "stdin": stdin,
            "env": case.get("env", {}),
            "stdout": scrub(result.stdout),
            "stderr": scrub(result.stderr),
            "exit_code": result.returncode,
            "config_exists_after": config_file.exists(),
            "cache_exists_after": cache.exists(),
        }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=HERE, help="directory to write the golden to")
    args = parser.parse_args()
    golden = {
        "bug_report_template": _BUG_REPORT_TEMPLATE,
        "cases": [run_case(case) for case in CASES],
    }
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "python-cli.json").write_text(json.dumps(golden, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
