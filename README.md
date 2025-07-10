![PyPI - Version](https://img.shields.io/pypi/v/lazy-github) ![PyPI - Downloads](https://img.shields.io/pypi/dw/lazy-github)

LazyGithub is a terminal UI client for interacting with [GitHub](https://github.com). It draws heavy inspiration from the
[lazygit](https://github.com/jesseduffield/lazygit) project and uses [Textual](https://textual.textualize.io/) to drive the terminal UI interactions.

![Example screenshot](https://raw.githubusercontent.com/gizmo385/lazy-github/main/images/lazy-github-conversation-ui.svg)

## How to Use It

### Installing with the Github CLI

If you have the Github CLI installed, you can install LazyGithub as an extension with `gh extension install gizmo385/gh-lazy` and then running `gh lazy` to run it.

### Installing from PyPi

You can run the [most recently built version](https://pypi.org/project/lazy-github/) by installing it from PyPI. If you have [uv installed](https://github.com/astral-sh/uv), you can do that easily with `uvx lazy-github`.

When you first start LazyGithub, you will be prompted with a device login code and a link to GitHub
where you will be able to authenticate the app against your account. This allows the app to act on
your behalf and is necessary for LazyGithub to function.

Currently, it supports the following:

- Listing the repositories associated with your account 
- Listing the issues, pull requests, and actions on your repositories
- Listing the details, diff, and reviews on any of those pull requests
- Detailed issue and pull request views, including conversation participation

### Reporting Bugs

If you run into any bugs while using LazyGithub, please let me know! You can do this by creating a
new [bug report](https://github.com/gizmo385/gh-lazy/issues/new?template=bug_report.md) or search
for `Open Bug Report` in the command pallete from inside LazyGithub.

[![In-App Bug Reporter](https://asciinema.org/a/JiTZqOWyEjPzG4v1marSyTqUV.svg)](https://asciinema.org/a/JiTZqOWyEjPzG4v1marSyTqUV)

### Running Locally

If you wish to run it from a local clone of the repository, you can do so by running the `./start.sh` located in the root of the repo.

## Customization

LazyGithub supports a number of customization options, all of which are stored in `$HOME/.config/lazy-github/config.json`.
These can be edited manually via changing the config or by opening the settings management UI within LazyGithub. That UI
can be accessed via the command pallete (`CMD+p`) and then searching for settings.

![Settings screenshot](https://raw.githubusercontent.com/gizmo385/lazy-github/main/images/lazy-github-settings-ui.png)
