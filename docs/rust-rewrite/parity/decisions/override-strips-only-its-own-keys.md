# An override removes only the overridden binding's default keys

- Decision: When `bindings.overrides` rebinds a binding, only that binding loses its default keys. Another binding
  in the same component that has one of those keys by default keeps it. Textual 8.2.5's `BindingsMap.apply_keymap`
  removes the old keys by deleting each whole key entry from the component's `key_to_bindings`, so every binding on
  that key loses it, including bindings with no id (Textual built-ins). Rust doesn't copy that.
- Decided by: Chris, 2026-10-25
- Link: [#119 decision](https://github.com/gizmo385/gh-lazy/issues/119#issuecomment-6407804287)
- Rows: `keybindings.override-keeps-shared-keys`

Checked against the venv's Textual 8.2.5, with two bindings in one component sharing `r`:

```python
m = BindingsMap([Binding('r', 'a', 'A', id='x.a'), Binding('r', 'b', 'B', id='x.b'), Binding('h', 'c', 'C', id='x.c')])
m.apply_keymap({'x.a': 'z'})
# {'h': ['x.c'], 'z': ['x.a']}: x.b lost 'r' as well
```

In Rust, `x.b` keeps `r`. Users override a binding to move it, not to turn off another binding's key, and the
Settings screen shows each binding's keys on its own.

**No visible difference today.** For this rule to show, one component would need two bindings that share a default
key. None does, in either app:

- The case #119 was filed for doesn't arise. `_VimLikeDataTable`'s `common.table.select` (`enter,space`) replaces
  `DataTable`'s own `enter` → `select_cursor` when Textual merges the class bindings (`DOMNode._merge_bindings`
  keeps only the subclass's bindings for a key both declare). The merged table therefore has no id-less `enter`
  binding, and overriding `common.table.select` to `o` leaves `enter` and `space` unbound in Python. Rust's
  `TABLE_BINDINGS` leaves out `textual.table.select_cursor` for the same reason, and the table test
  `rebinding_select_takes_enter_with_it` checks that Enter and Space go unbound.
- A scan of every `lazy_github` widget's merged bindings finds no key held by two different bindings. Since a
  subclass's key replaces the inherited one, a shared key would have to come from a single `BINDINGS` list. The
  Rust component lists (`TABLE_BINDINGS`, `INPUT_BINDINGS`, `INFO_BINDINGS`, the main screen's and the app's) don't
  have one either.

A separate rule does match Python: an override *onto* a key that another binding in the same component has by
default takes that key away from it (Textual's clash handling in `apply_keymap`). `Keymap::effective_specs`
implements that.

The keymap test `an_override_leaves_a_shared_default_key_on_the_other_binding` in
`crates/lazy-github-tui/src/keymap/keymap.rs` pins the Rust rule.
