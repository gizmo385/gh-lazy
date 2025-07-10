import importlib.metadata
import textwrap

from lazy_github.lib.config import Config

LIBRARY_VERSIONS_TO_CHECK = ["textual", "click", "httpx", "hishel", "pydantic"]


def collect_debug_info() -> str:
    """Generates a debug string for the modal and issue reports"""
    # Import libraries to determine versions
    from lazy_github.version import VERSION as LAZY_GITHUB_VERSION

    debug_info = "# Library Versions\n\n"

    debug_info += f"- LazyGithub Version: {LAZY_GITHUB_VERSION}\n"
    for lib in LIBRARY_VERSIONS_TO_CHECK:
        debug_info += f"- {lib.title()} Version: {importlib.metadata.version(lib)}\n"

    debug_info += "\n# LazyGithub Config\n\n"
    debug_info += f"```json\n{Config.load_config().model_dump_json(indent=4)}\n```"

    return textwrap.dedent(debug_info.strip())


_BUG_REPORT_TEMPLATE = """
**Describe the bug**

**To Reproduce**

**Expected behavior**

**Installation Mechanism:** [Github CLI, nix, PyPi, etc]

**Debug Info:**
{debug_info}

**Additional context**
""".strip()


def get_bug_report_template() -> str:
    """Generates a LazyGithub bug report that is prefilled with debug information"""
    return _BUG_REPORT_TEMPLATE.format(debug_info=collect_debug_info())
