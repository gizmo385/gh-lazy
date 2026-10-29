# Terminal caveats

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

| Id | Caveat | Affects | Recommendation |
| --- | --- | --- | --- |
| `terminal-caveats.ctrl-m-enter` | `ctrl+m` and Enter are the same byte (CR) without the kitty keyboard protocol. | `pull_request.merge` | Keep the id and default. Only treat a disambiguated `ctrl+m` as merge; never Enter. See Py bug B30. Implemented in #262 (`pr-merge` sends CSI 109;5 u). |
| `terminal-caveats.shift-enter` | `shift+enter` arrives as plain Enter in legacy terminals. | `modal.submit` | Push kitty enhancement flags when supported; keep the Submit buttons working; don't map plain Enter to submit inside multi-line editors. |
| `terminal-caveats.ctrl-v-paste` | `ctrl+v` is often captured by the terminal as paste. | `link.paste` | Also handle bracketed paste on the main screen as "open paste modal with this text" (needs a decision if it changes UX). |
| `terminal-caveats.uppercase-bindings` | Uppercase bindings (`M`, `I`, `P`, `E`, `O`, `J`, `K`, `G`, `R`, `T`, `F`) arrive as shifted characters; some terminals report `shift+m`. | all uppercase ids | Normalize `shift+<letter>` and `<LETTER>` to the same key. |
| `terminal-caveats.punctuation-key-names` | Punctuation keys are saved by the Python Settings modal as Textual names (`question_mark`, `slash`, `dollar_sign`, `circumflex_accent`, …). | `bindings.overrides` | Keymap parser accepts Textual key names and literal characters (#73). |
| `terminal-caveats.key-capture` | A Key Bindings row records the key the terminal reports. Without the kitty protocol `ctrl+m`, `ctrl+i` and `ctrl+[` arrive as `enter`, `tab` (which moves focus on) and `escape` (which resets the row), and `shift+enter` as `enter`, as Textual sees them too. | Settings Key Bindings tab (#194) | Record what crossterm reports, written as Textual's name; no extra hint (Python shows none; Chris, 2026-10-28, on #194). `tab`, `shift+tab` and `escape` can't be bound from the tab. |
| `terminal-caveats.escape-ambiguity` | `escape` is ambiguous with Alt-prefixed sequences; crossterm handles the timeout. | `modal.close`, maximize restore, link hints | No action beyond crossterm defaults. |
| `terminal-caveats.q-in-inputs` | `q` closes modals and quits the app, but must type a `q` inside text inputs. | `app.quit`, `modal.close` | Text inputs get first refusal on printable keys. |
| `terminal-caveats.footer-key-labels` | Footer key labels: Textual shows `^p` for `ctrl+p`. | footer, help panel | decided: Textual notation (`^p`) |
| `terminal-caveats.terminal-hyperlinks` | Terminal hyperlinks (`[link=…]`) are used for a few links (run number, profile login); others are in-app links. | Run Overview, User profile | Render as OSC 8 where supported; treat as hint targets only if the spec says so. |
