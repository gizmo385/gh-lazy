; Match Textual framework imports
; Differentiates between widgets and other API imports

; Textual widgets: from textual.widgets import Label, Button, etc.
(import_from_statement
  module_name: (dotted_name) @_textual_widgets
  name: (dotted_name) @textual.widget
  (#eq? @_textual_widgets "textual.widgets"))

(import_from_statement
  module_name: (dotted_name) @_textual_widgets
  name: (aliased_import
    name: (dotted_name) @textual.widget)
  (#eq? @_textual_widgets "textual.widgets"))

; Textual API imports from other modules: from textual.containers import Horizontal
; from textual.app import App, from textual import work, etc.
(import_from_statement
  module_name: (dotted_name) @_textual_api
  name: (dotted_name) @textual.api
  (#match? @_textual_api "^textual")
  (#not-eq? @_textual_api "textual.widgets"))

(import_from_statement
  module_name: (dotted_name) @_textual_api
  name: (aliased_import
    name: (dotted_name) @textual.api)
  (#match? @_textual_api "^textual")
  (#not-eq? @_textual_api "textual.widgets"))
