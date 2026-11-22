# Roll-up 50: Typing-support-final-shape (509864e95a) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 64730071f0 (roll-up 49, Q470). Theirs: Typing-support-final-shape at 509864e95a,
pinned 2026-11-22 in `tmp/rollup/pins4.txt`. The merge base is e7697f6e8e, roll-up 46. Theirs brings t32's last
stages and t33, every cache moved to kept facts:

- t32 1.4: the digest and blob column types, stored as symbols and checked as a tape is (decision 0103);
- t33 K2: the observer's answers, canonical forms and settings are kept facts of the content store. `corpus_stats`
  goes: a corpus file's content is the observer's answer for its path;
- t33 K3: each native step's answer is declared in the library (`.answer REQ -> R1, R2`);
- t33 steps 2, 3 and 6: a prepared program's pointer, the slice's last run per scope, and the Neural Engine's
  compiled models are kept facts;
- Windows: process and thread CPU, the watchdog, a store's identity, library paths (d32811f00b).

## Items

1. **`boot/Cargo.toml`'s `test-hooks`: both lists, as one.** Ours' list (roll-up 49's crates) and theirs' each added
   crates. The merged list is ours' plus theirs' `rerail-blocks/test-hooks`, the one crate ours' list lacked. The
   audit's theirs' line is the old list, superseded by the union.
2. **`rr/tests.rs`: both new tests.** Ours' `a_texts_declared_policies_read_without_lowering` (a policy's
   `.policy` names read without its lowering) and theirs' `every_native_answer_is_declared_in_the_library` (K3) test
   different things. Both are kept, and both pass.
3. **The prepared program's pointer (theirs' step 2) beside ours' preparation runs.** Theirs moved the pointer from a
   facts-memo entry under the program text's source to kept facts keyed by the program (`pointer::put`/`get`), so
   `prepared::documents` takes the store and the key, not a source. Ours' change to this code is `runs: None` and
   `PreparationRuns`, which record which runs prepared, a separate job. Both are kept. The one caller of `documents`
   (`run_observed_within`) auto-merged to theirs' signature.
4. **The CPU shim: theirs' edit at ours' path.** Ours moved `ukl_cpu_shim.h` to `native/crates/ukl/mldb/` unedited.
   Theirs' Windows commit (d32811f00b) edited it at the old path: the scheduler's thread key, not `pthread` alone.
   The merged file is the same as theirs' (0 lines of diff). The audit's three ours' lines are the base's lines that
   theirs replaced.
5. **Decision 0103's digest and blob types.** Theirs extends 0103's column types. Ours did not change the types.
   The extension keeps 0103's invariant: a column is stored in one form and checked as a tape is.

## Found, and fixed in the commits after the merge

Each of these was red at both sides' tips, and is fixed at its root, not passed over.

1. **`a_policy_run_takes_its_prepared_plans_and_finds_the_same` checked a line that was not its own.** Since Q242
   (cfa47680f4, on both sides), a rule that tests a constant on a `values` column is planned at run time and not
   prepared. The test's rule tested two: `kind(n, "expression_statement")` and `kind(p, "module")`. So its policy
   had no prepared line, and `taken > 0` held only because phase 0's `p0_decl` line is taken in the same run. That
   line is not taken once the process holds a kept plan for it, so the test failed whenever another corpus test ran
   first in its binary (`a_message_edit_finds_the_program_prepared_and_a_rule_edit_does_not`), and under
   `RERAIL_PLAN_CACHE_COUNTS`, whose report sets the counters back to zero.
   - The rule is now the same claim with no constant: a statement at its module's top, the module the node with no
     parent (`hit(n) :- child(p, n, _), !child(_, p, _)`). Its findings add `b.py:1`, which the rule says.
   - The profile's decision for a prepared line names its head relation. The test checks that `hit`'s own line was
     taken, read from the run's decisions.
2. **Two Git modules read relations that no file in their program declares.** `git/messages.rerail` reads
   `git_message`, and `git/selected-tree.rerail` reads `git_tree_entry` and `git_tree_entry_bytes`. `git.rerail`
   declares all three, and includes `messages.rerail`. `rule_syntax_files_pass_check_rr` refuses such a program, as a
   run would.
   - Each module now includes `git.rerail`, the library's convention for a module and its includer
     (`python/imports.rerail` and `python/names.rerail` include each other).
   - The engine's three Git programs (`corpus/git.rs`: source checks, the root tree, the inventory) pasted
     `core.rerail` and `git.rerail` as text and resolved `git.rerail`'s own include by path. A module included
     both ways is read twice: with the modules' includes, the pasted form is refused (`.cache: git_record is named
     twice`). They now include the library's modules by name, as a policy does, as constants (`git::SOURCE_CHECKS`,
     `ROOT_TREE`, `INVENTORY`). The library ships beside the addon (`source_tree::rule_library`), which `git.rerail`'s
     include already required. `the_git_programs_read_their_library_once` reads each as one program.

## Checks

- spark2, CPU: `cargo check --workspace --tests` is clean. `check_crate_dag.py` passes.
- Theirs' tests: rerail-logic `facts_only` 2, `cache_architecture` 3, `tape_type`; rerail-store `kept_facts` 11;
  rerail-tests-logic `cache_tests` 3, `corpus_runs` 1. All pass.
- With the fixes: rerail-tests-logic `prepared` 15, `code_rule_dsl::logic::plan::tests::` 93,
  `rule_syntax_files` 2, the two rr tests of item 2, `inventory` 1. rerail-tests-matcher `context_tests` 74 and
  `git` 3. rerail-workspace `git` 35, rerail-engine `git` 24. All pass.
- The prepared-plans test passes alone, after its sibling in one process, and under `RERAIL_PLAN_CACHE_COUNTS`.
- `the_git_programs_read_their_library_once` passes. No Rust test runs the inventory over a Git work tree: a corpus
  root inside one runs it (`corpus_listing`), and the tests' roots are temporary folders.
- Not run: the addon's end-to-end scans over a Git checkout (`test/cli/headless-scan.test.ts`), which need the addon
  built.
- No bench, by the user's order of 2026-11-22.
