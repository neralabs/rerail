# Language registry

One `rerail.language/v1` file per language: `<id>.json`. Each file holds:
- the family
- the dialects: file suffixes, parser, grammar and tape tag
- per-capability support
- suppression comments
- embedded languages
- the status: `supported` or `planned`
- `builtWith` (optional): the language whose `lang-<id>` feature builds this one, for a guest language with no
  feature of its own (`python-format` is built with `lang-python`)

This directory is the single source of truth for both sides:
- `native/build.rs` validates it and generates the engine's language tables.
- `npm run generate:languages` writes the host's module, `src/engine/code-rule/code-rule-language-registry.generated.ts`.

A `planned` language is accepted in rules but reported `language-not-supported`, and no file is routed to it.

Format, statuses and how to add a language: [docs/language-packs.md](../docs/language-packs.md).
