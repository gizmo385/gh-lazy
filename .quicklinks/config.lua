-- Quicklinks configuration for gh-lazy (lazy-github)
-- Context-aware navigation for Textual framework documentation

return {
  patterns = {
    python = { 'textual_imports' },
  },

  actions = {
    -- Textual widgets imported from textual.widgets
    -- Example: from textual.widgets import Label, Button
    ['textual.widget'] = {
      {
        type = 'url',
        title = 'Open {match} widget docs',
        url = 'https://textual.textualize.io/widgets/{match|lowercase}/',
      },
      {
        type = 'search',
        title = 'Find usages of {match}',
        query = '{match}',
        file_patterns = '*.py',
      },
    },

    -- Textual API imports from other modules
    -- Examples: from textual.containers import Horizontal
    --           from textual import work
    ['textual.api'] = {
      {
        type = 'url',
        title = 'Open {module} API docs',
        url = 'https://textual.textualize.io/api/{module|lowercase}/',
      },
      {
        type = 'search',
        title = 'Find usages of {match}',
        query = '{match}',
        file_patterns = '*.py',
      },
    },
  },

  transforms = {},

  context_extractors = {
    -- Extract the module name from textual API imports
    ['textual.api'] = function(match, bufnr, get_parent_of_type)
      local node = match.node

      -- Try to find the import_from_statement parent
      local import_node = get_parent_of_type(node, 'import_from_statement')
      if not import_node then
        return {}
      end

      -- Get the module name (e.g., "textual.containers" -> "containers")
      local module_name_node = import_node:field('module_name')[1]
      if module_name_node then
        local full_module = vim.treesitter.get_node_text(module_name_node, bufnr)
        -- Extract the last part after the dot (e.g., "textual.containers" -> "containers")
        local module = full_module:match('textual%.(.+)') or full_module
        return { module = module }
      end

      return {}
    end,
  },
}
