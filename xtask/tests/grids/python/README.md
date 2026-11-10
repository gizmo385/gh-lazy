# Frozen Python screen dumps

Grid dumps of the Python app's screens, captured by the parity harness before #397 removed its Python side. They are
frozen test vectors for `structure.rs`, `focus.rs`, `check.rs` and `screen.rs` (`test_grid::load("python/...")`):
reading them needs no Python, and nothing regenerates them. The Rust app's screens are covered by the scenario
snapshots in `parity/snapshots/`, which every parity run extracts and checks.
