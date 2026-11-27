# Rerail rule files for Orca

An Orca data plugin: syntax highlighting for `.rerail` rule files, as a TextMate grammar
(`syntaxes/rerail.tmLanguage.json`) with comment markers and auto-closing pairs (`syntaxes/language-configuration.json`).
It runs no code and needs no approval.

Orca has no language server client, so `rerail lsp` cannot reach its editor; the grammar is what colors `.rerail`
files there. It follows the engine's own reading of a file (`rerail_rr_syntax::highlight`, what `rerail lsp` serves
as semantic tokens): on every `.rerail` file of this repository, the class Orca gives each token matches the engine's
for all but about 70 of 905,000 tokens. The scopes it uses, by what they name:

| What | Scope | Orca's token |
|---|---|---|
| `.decl`, `.violation`, `.mode` and every other directive, `#include` | `keyword.control.*` | `keyword.flow` |
| A relation `.decl`, `.violation`, `.edit`, `.lattice`, `.input` declares | `entity.name.type.relation` | `type` |
| A relation or function a clause calls | `entity.name.function.relation` | `function` |
| A column (`x` in `r(x: node)`) and its type | `variable.parameter`, `support.type` | `variable.parameter`, `type` |
| `:-`, `!`, `must`, `count`, `code`, `fix`, a declaration's modifiers | `keyword.*`, `storage.modifier` | `keyword` |
| A setting's word (`log` in `.mode log`) | `constant.other.setting` | `constant` |
| The pattern of `match("…", n)` | `string.regexp` | `regexp` |
| A backtick code pattern or fix template, and its `$X` | `markup.inline.raw.code`, `support.variable.metavariable` | `string.raw`, `variable.predefined` |

Orca's built-in themes leave `function` and `identifier` plain, so relations a clause calls and variables take a
color only from a theme that styles them.

Two limits are Orca's: a line over 1,000 characters leaves the rest of the file plain (54 of this repository's
`.rerail` files have one, most a long `.scope exclude` or `.options`), and so does a file over 20,000 lines.

The configuration declares no `brackets` on purpose. Monaco colors and matches a language's brackets by its own
tokenizer, not by the grammar's tokens, and a plugin language has none, so it cannot tell a parenthesis in a comment
or a string from one in code: with `brackets`, every `(` in a `//` comment took a bracket-pair color. Orca's
configuration has no `colorizedBracketPairs` to turn coloring off alone, so `.rerail` brackets are neither colored nor
matched.

## Use it

```
orca plugin dev load plugins/orca      # this folder, as a development plugin, reloaded on each change
orca plugin install plugins/orca       # or installed from the folder
```

`orca plugin validate plugins/orca` checks it the way Orca loads it.
