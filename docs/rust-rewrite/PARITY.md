# Parity Matrix (moved)

The parity matrix now lives in [`parity/`](./parity/README.md), one file per area, so that parallel PRs don't
conflict on a single file:

- [`parity/README.md`](./parity/README.md): status values, columns, editing rules and the index of area files.
- [`parity/areas/`](./parity/areas/): the rows, one table per area.
- [`parity/py-bugs/`](./parity/py-bugs/): suspected Python bugs (`B<nn>.md`).
- [`parity/decisions/`](./parity/decisions/): intentional diffs.

Don't add rows here. `cargo xtask parity-matrix` checks the matrix and prints a summary.
