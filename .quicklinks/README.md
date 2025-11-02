# gh-lazy Quicklinks

Context-aware navigation for the gh-lazy (lazy-github) project using [nvim-quicklinks](https://github.com/gizmo385/nvim-quicklinks).

## What This Provides

This configuration provides intelligent code navigation for:

1. **Textual Framework Documentation** - Quick access to Textual widget and API docs

## Quicklinks Included

### 1. Textual Widget Imports (`textual.widget`)

**When cursor is on:** Widget imports from `textual.widgets`

```python
from textual.widgets import Label, Button, DataTable
```

**Available actions:**
- **Open {widget} widget docs** - Opens the Textual documentation for the widget
  - Example: `Label` → https://textual.textualize.io/widgets/label/
- **Find usages** - Search for all usages of the widget in the codebase

**Try it:** Place cursor on `Label`, `Button`, `DataTable`, etc. in any import statement

---

### 2. Textual API Imports (`textual.api`)

**When cursor is on:** API imports from other Textual modules

```python
from textual.containers import Horizontal, Vertical
from textual import work
from textual.app import ComposeResult
```

**Available actions:**
- **Open {module} API docs** - Opens the Textual API documentation for the module
  - Example: `Horizontal` → https://textual.textualize.io/api/containers/
  - Example: `work` → https://textual.textualize.io/api/work/
- **Find usages** - Search for all usages in the codebase

**Try it:** Place cursor on `Horizontal`, `work`, `ComposeResult`, etc. in any import statement

## Setup

### Prerequisites

1. Install [nvim-quicklinks](https://github.com/gizmo385/nvim-quicklinks) plugin
2. Enable project-specific configuration:

```lua
-- In your init.lua
require('quicklinks').setup({
  enable_project_config = true,
  -- Your other config...
})
```

### Usage

1. Open any Python file in the gh-lazy project
2. Place cursor on:
   - A Textual widget import (e.g., `Label`)
   - A Textual API import (e.g., `Horizontal`, `work`)
3. Run `:Quicklinks` (or use your keybinding)
4. Select the action you want to perform

## How It Works

### Files

- **`.quicklinks/config.lua`** - Action definitions and context extractors
- **`.quicklinks/queries/python/patterns/textual_imports.scm`** - Treesitter query for Textual imports

### Context Extraction

The configuration uses intelligent context extraction:

- **For Textual APIs**: Extracts the module name from the import statement
  - `from textual.containers import Horizontal` → module: `containers`
  - Used to construct the correct API documentation URL

### Customization

You can customize the quicklinks by editing `.quicklinks/config.lua`:

- Adjust URL patterns for documentation
- Add additional actions or search patterns
- Add new Textual modules or widgets

## Examples

### Navigating Textual Widget Documentation

```python
# Place cursor on "Label" and run :Quicklinks
from textual.widgets import Label

# Opens: https://textual.textualize.io/widgets/label/
```

### Navigating Textual API Documentation

```python
# Place cursor on "Horizontal" and run :Quicklinks
from textual.containers import Horizontal

# Opens: https://textual.textualize.io/api/containers/
```

## Contributing

Have suggestions for additional Textual patterns or other documentation quicklinks? Feel free to update `.quicklinks/config.lua` and submit a PR!
