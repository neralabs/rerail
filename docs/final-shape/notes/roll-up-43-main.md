# Roll-up 43: main (1609ac3a30) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 7999a180f2 (roll-up 42, engine-crating, Q463).

Theirs: main at 1609ac3a30. Its merge base with ours is 21f86868e9 (roll-up 35). Theirs brings three PRs:

- #959: on Windows, a Claude Code hook call reuses the code Node compiled for the last one;
- #973: a repository's rules compile off the daemon's thread (`CodeRuleDslRuntime.compileAsync`, a napi `AsyncTask`
  on the worker pool). A tool call's start builds its files' rules before the edit's capture (`prepareRules`), and a
  session builds ahead the rules it will need (`rulesBuiltAhead`). A panic in a compile reads the same on either
  thread (`compile_contained`);
- #977: a session in a worktree whose main checkout was deleted keeps its record. An identity file's candidate id is
  made only when no file is there (`readOrCreateIdentity(path, () => …)`).

## The invariants every item is checked against

From ours:

- **The crate split** (steps 1b–3). The rule's compile lives in `rule_api` (in `rerail-classic`), the one body the
  host's napi entry and the engine's glue both call. The engine crate hosts only napi.
- **Owners look again at a point, not per path** (`refreshOwners()`, `CheckoutOwners.lookAgain`). A
  `git worktree add` from any repository can change a folder's owner, so the point refreshes every answer, not only
  the call's paths.
- **A bare repository is a folder** (0497). Its id is in `.rerail/repository-id`, a linked worktree takes it, and
  `checkoutIdentity` returns early on a marker, so a marked checkout starts no Git process.

From theirs:

- **The daemon's thread compiles nothing.** A compile runs on a pool thread, and the host's thread stays free.
- **A panic reads the same from both entries.**
- **An id is made only when none is recorded.**

## Items

1. **The compile: one body in `rule_api`, two host entries.** Ours had moved compile's body to `rule_api::compile`.
   Theirs split the same body in `mod.rs` into `compile_rule` behind `compile_contained`, for `compile` and
   `compile_async`/`CompileTask`. The merge keeps the body in `rule_api`. `rule_api::compile_rule` returns the
   rule, and `rule_api::compile` wraps it for the glue. `mod.rs` keeps theirs' napi entries, `compile_contained`
   and `CompileTask`. Its `compile_rule` is theirs' test panic hook followed by `rule_api::compile_rule`.
   Test: `compile_panic_tests::a_compile_that_panics_reads_the_same_from_compile_and_from_compile_async` passes on
   the merged compile.
2. **`refreshOwners()` keeps ours' signature, and theirs' `prepareRules(paths)` follows it.** A call's start looks
   again at every owner, then builds the rules of the checkouts that own the call's paths. The order matters:
   `prepareRules` resolves owners, and it reads them after the look-again point. The `CodeRuleAuditor` interface
   has ours' `refreshOwners?(): void` and theirs' `prepareRules?` and `rulesBuiltAhead?`. Theirs' router already
   had them, and auto-merged with ours' `refreshOwners()`.
3. **Identity: ours' bare-folder ids, theirs' lazy candidate.** `readOrCreateIdentity` takes theirs' `() => string`.
   Ours' `gitCloneId` passes `() => stable ?? git:<uuid>`, and the folder case is theirs' (auto-merged). A
   checkout with a marker still returns before any candidate is made. Test: `repository-identity.test.ts` 12/12.

## Checks

- On spark2, `cargo check --tests -p rerail-engine -p rerail-classic` is clean, and the compile panic test passes.
- Here, `repository-identity.test.ts` passes, 12/12.
- Not run here: the router's and the bridge's tests, which need the native addon that this checkout does not
  build. A full `tsc --noEmit` also didn't run: it passes the 3 GB cap. The TS changes are the two signatures
  above, and every caller was read: `refreshOwners?.()` at both call sites, and lambdas at all three
  `readOrCreateIdentity` calls.
