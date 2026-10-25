# A broken config file is a one-line error

- Decision: A `config.json` that fails to load prints a one-line error naming the file and suggesting
  `lazy-github clear-config`, then exits 1.
- Decided by: Chris, 2026-10-25
- Link: [project thread](https://claude.ai/code/project/chan_012pc8Wphbdkgs8rn26pJBv9?thread=cmsg_012pc8Wphbdkgs8rn26pJBv91YRDXqF1kGmJaE3iA28mKh), implemented in #72
- Rows: `cli.broken-config-error`

Python crashes with a pydantic `ValidationError` traceback from `lib/config.py`.
