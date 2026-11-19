# Anchor lab against P on the same batches (list step 12)

1. Corpus: 174 Python files (every third `packs/python/upstream` file, 150 at most, then every
   30th stdlib file outside `test*`, 100 at most), copied as `fNNN.py`.
2. P's addon with its capture feature, built without touching P's worktree: clone P's `target/lab`
   (`cp -cR`, an APFS clone) into a scratch target and
   `cargo build --manifest-path <P>/native/Cargo.toml --profile lab --features code-rule-profile --lib --locked --target-dir <scratch>`;
   copy `lab/librerail_engine.dylib` to `p-profile.node`.
3. P's captures: `P_ADDON=p-profile.node RERAIL_SQL_PROFILE_DIR=<p> node p-capture.mjs <corpus> <out.txt>`
   (P's workspace over the corpus with every pack policy; it batches one file at a time).
4. P's batches by path, from each capture's `files` and its `file` table, as a JSON array of arrays.
5. Our captures over the same batches: `RERAIL_LAB_CORPUS=<corpus> RERAIL_SQL_PROFILE_DIR=<w>
   RERAIL_LAB_BATCHES=<batches.json> RERAIL_CAPTURE_TAPES=1 <test binary> capture_lab_corpus --ignored`.
6. Policies as three conflict-free merged documents: `P_ADDON=p-profile.node node chunk.cjs <dir>`.
7. Each lab, `RERAIL_LAB_BATCH=1 RERAIL_LAB_POLICY=$(cat <dir>/policies.txt)`: P's
   `target/lab/deps/rerail_engine-<hash> anchor_lab --ignored` over P's captures, ours
   `code_rule_dsl::prefilter::lab::anchor_lab --exact --ignored` over ours (add
   `RERAIL_LAB_FALLBACK=1` for the pure-SQL fallback, `RERAIL_TREE_DESCENT=1` for parents by
   descent).
