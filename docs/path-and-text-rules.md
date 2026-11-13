# Writing path and text rules

A code rule doesn't have to parse code. Three queries work on any text file, in any language or none:

| You want to say | Query | Reported at |
|---|---|---|
| "this text should not appear in these files" | `{ "regex": { "source": "file.content", "pattern": "…" } }` | the match: `path:line:column` |
| "a file with this kind of path should not exist" | `{ "regex": { "source": "file.path", "pattern": "…" } }` | the file: `path`, no line |
| "the files this rule's scope names should not exist" | `{ "file": {} }` | the file: `path`, no line |

For syntax queries (`ast`), see [Writing a code rule](../skills/rerail-create/references/code-rules.md). For the full
grammar, see the [DSL reference](code-rule-dsl-reference.html#queries).

## Which files these rules read

A `regex` or `file` rule reads every text file in its `scope`; binary, skipped, ignored and secret-named files are left
out.

Text files are source files, common text formats such as `.json` and `.md`, and any other file without a NUL byte. A
rule made only of `ast` queries reads only its language's files, however wide its scope.

<details>
<summary>Which files count as text</summary>

- source files of the languages Rerail parses: Python, TypeScript, JavaScript, PHP, Blade, Rust, SQL, Vue, Java;
- files read as text by suffix or name: `.json`, `.yaml`, `.yml`, `.toml`, `.md`, `.mdx`, `.rst`, `.txt`, `.sh`,
  `.bash`, `.zsh`, `.html`, `.css`, `.scss`, `.gitignore`, `.npmignore`;
- any other file without a NUL byte, such as `Justfile`, `LICENSE`, `NOTICE`, `.gitattributes` or `Cargo.lock`, unless
  it is named like one holding secrets.

</details>

<details>
<summary>Files no rule reads</summary>

- **Binary files**, with a NUL byte. `rerail rules test` counts those in a rule's scope, so you can see what it passed
  over.
- **Skipped folders**: `.git`, `node_modules`, `dist`, `build`, `target`, `vendor` and the rest in the
  [reference](code-rule-dsl-reference.html#files-read).
- **Files Git ignores**, while the repository follows `.gitignore`.
- **Other files named like secrets**: `.env`, `.npmrc`, `id_rsa`, `*.pem`, `*.key`, `*.tfstate`, anything under
  `.ssh/` or `.aws/`, and the rest of `SENSITIVE_FILE_NAMES` in `src/engine/path-boundary.ts`. No rule sees their text
  or path.

A listed file such as `secrets.yaml` is read, but a steer about it names only the rule and the place, never the message
or fix.

</details>

A file too large to read is reported as not checked, not passed over. A large `yarn.lock` or `.csv` under a `**/*` rule
is the usual case; exclude it if the rule is not about it. The limit is 32 MB in an audit and 1 MB in a session, which
reads files as the agent edits them.

## Text in files

A `file.content` regex reports each match at its line and column.

This one catches an old folder name anywhere it is still written, `Justfile` and lock files included:

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:no-old-tools-path",
    "revision": 1,
    "mode": "steer",
    "description": "Refer to scripts/ instead of the old tools/ folder, which was renamed.",
    "scope": { "include": ["**/*"], "exclude": ["CHANGELOG.md"] },
    "clauses": [
      {
        "id": "old-path",
        "forbid": { "regex": { "source": "file.content", "pattern": "\\btools/" } },
        "violation": { "code": "OLD_TOOLS_PATH", "anchor": "witness.span", "message": "The tools/ folder is now scripts/: update this reference." }
      }
    ]
  },
  "cases": {
    "caught": [{ "path": "Justfile", "content": "lint:\n    node tools/lint.mjs\n" }],
    "passed": [
      { "path": "Justfile", "content": "lint:\n    node scripts/lint.mjs\n" },
      { "path": "Justfile", "content": "lint:\n    node devtools/lint.mjs\n" },
      { "path": "CHANGELOG.md", "content": "- tools/ is now scripts/\n" }
    ]
  }
}
```

The dialect is Rust's: no look-around, no backreferences. `(?m)` makes `^` and `$` match at each line. In JSON, write
every backslash twice. Put exceptions a pattern cannot express in `scope.exclude`.

## Where files live

A `file.path` regex reads the file's repository-relative path, with `/` between folders.

A match is about the file, so the violation names the path with no line, once per file however often the pattern
matches. Named groups (`(?<NAME>…)`) still fill in `$NAME` in the message.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:kebab-case-components",
    "revision": 1,
    "mode": "steer",
    "description": "Name component files in kebab-case.",
    "scope": { "include": ["src/components/**/*.tsx"], "exclude": [] },
    "clauses": [
      {
        "id": "case",
        "forbid": { "regex": { "source": "file.path", "pattern": "(?<NAME>[^/]*[A-Z_][^/]*)\\.tsx$" } },
        "violation": { "code": "COMPONENT_FILE_CASE", "anchor": "witness.span", "message": "Rename $NAME.tsx to kebab-case, for example user-card.tsx." }
      }
    ]
  },
  "cases": {
    "caught": [{ "path": "src/components/UserCard.tsx", "content": "export {};\n" }],
    "passed": [
      { "path": "src/components/user-card.tsx", "content": "export {};\n" },
      { "path": "src/components/Admin/user-table.tsx", "content": "export {};\n" }
    ]
  }
}
```

A path match takes no `fix` and no `capture.NAME` anchor. Anchor it at `file.start` only if a tool you compare with
reports such files at line 1.

## Files that should not exist

A `file` query matches every file in scope, so the scope names the unwanted files and `exclude` the exceptions.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:pack-json-placement",
    "revision": 1,
    "mode": "steer",
    "description": "Keep each pack's pack.json at the top of its pack folder.",
    "scope": { "include": ["**/pack.json"], "exclude": ["packs/*/pack.json"] },
    "clauses": [
      {
        "id": "misplaced",
        "forbid": { "file": {} },
        "violation": { "code": "MISPLACED_PACK_JSON", "anchor": "witness.span", "message": "Move this pack.json to the top of its pack folder, packs/<name>/pack.json." }
      }
    ]
  },
  "cases": {
    "caught": [{ "path": "packs/python/rules/pack.json", "content": "{}\n" }],
    "passed": [
      { "path": "packs/python/pack.json", "content": "{}\n" },
      { "path": "packs/python/rules/rule.json", "content": "{}\n" }
    ]
  }
}
```

To silence one file, exclude it from the rule's scope: `// rerail-ignore` cannot silence a path or `file` match.

<details>
<summary>Cases, sessions and suppression</summary>

- **Cases:** a `file` query reads no text, so `content` can be anything.
- **Sessions:** the default audit scope (`ai-output`) tells the agent only about what its own edit wrote. A path or
  `file` match covers the whole file, so only the edit that creates it (write, copy or move) is told, not an edit to
  an existing file. The `touched-files` and `repository` scopes report every such file.
- **Suppression:** `// rerail-ignore` covers code, and a path or `file` match sits on no line.

</details>

## Test it

Each example above is a complete draft: the rule plus the cases it must catch and let pass.

Save one to a file outside the repository and run:

```sh
rerail rules test draft.json          # validate, run the cases, list the matches in this repository
rerail rules test draft.json --json   # the same, with every match and every file it could not check
rerail rules add draft.json           # store it in your library and add it to this repository
rerail rules test --help              # the draft format
```

The text report shows the first five matches; `--json` lists every one, for comparing with another tool's results. A
path or `file` match has no `line` or `column`.

<details>
<summary>Files the repository pass skips</summary>

The repository pass of `rules test` shows the coding agent lines of your files. So it skips hidden files (such as
`.gitattributes`, `.editorconfig`), secret-named files and Git-ignored files, and counts them without names
([Reference](reference.md#scan-and-settings-controls)). Sessions and audits do read hidden files, so a rule about one
can still catch it there: prove it works with its cases.

</details>
