# Structure pack: candidates

Drafts not shipped. Both count across files, and a pack example is evaluated as a one-file repository
(`evaluateExample` in `conformance/pack-validation.ts`), so neither can have a caught example and check 2 and check 3
cannot prove them. Each is waiting on multi-file examples, or on another proof for cross-file rules (a repository pass
at fixed thresholds over pinned open-source checkouts).

## Rule details

### `structure-directory-file-limit`

Counts the source files of one language in each directory, tests, stories, declarations, migrations, generated code,
vendored code, `components/ui` and `icons` excluded, and flags a directory with more than 20. TypeScript, JavaScript,
Python and Rust are counted separately. One match per directory, on its first file. The message asks for new files in a
subfolder named for the feature, and not to move existing files unasked.

### `structure-duplicate-function-bodies`

Groups function bodies of 10 lines or more by their exact text, and flags a body that appears in two or more files.
TypeScript, JavaScript and Rust; Rust `mod tests` blocks, generated files, tests, stories, migrations and UI kits are
out of scope. Text must match exactly, so a copy with a renamed variable is missed, and two copies in one file are not
counted. Starts in Log.
