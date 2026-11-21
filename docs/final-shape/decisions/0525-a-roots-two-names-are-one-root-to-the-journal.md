# 0525: A root's two names are one root to the journal

Status: done by the implementer, while fixing failing tests (the user, 2026-11-20: "please address these"). Follows
0105, 0197, 0400.

## Intro

Two corpus tests failed on one Mac and passed on a farm host, at this commit and at `ea99ffb5bd`:
`a_header_edit_asks_again_only_for_the_units_that_read_it` and `the_run_after_a_cold_one_writes_nothing`. They failed
with any temporary directory reached through a symbolic link (macOS's `/var`, which is `/private/var`), and passed with
a canonical one. A run asks a directory by both names: the name it was given, and the canonical one that a path it
resolved (`Observer::canonical`) is under.

Two faults:

- **A stale answer.** The header test's second run kept the answer of the unit that read `h.h` after `h.h` was
  edited. The listing check of the canonical root named `/private/…/ws/h.h`. The cone's file-system requests
  (`still_fn` in `corpus.rs`) read `…/ws/h.h`, and no path named was a prefix of it.
- **A write in a warm run.** The store's collection (`observe::collect`, 0197) kept one name of a directory and dropped
  the other's cursor. Its rule was "newest cursor first", but listing cursors (`L0`) did not parse as numbers, so which
  name stayed was arbitrary. The next run asked the dropped name, found no cursor, and stored one.

## The decision

1. **`Observer::changed_since` names a path under each name of a root it was asked for.** For each root, it takes the
   watched root's canonical form (`Root::real`, else one `realpath`). A path named under one name is named under the
   other too. Every caller gets it: the cone's rounds and the Git file lists.
2. **`collect` drops another name of a kept directory only where that name's cursor is older** than the kept name's.
   Two cursors compare only when they are of one kind: FSEvents ids with each other, listing generations with each
   other. Two names asked by one run have the same cursor, so both are kept. A name with no cursor that parses (a
   Watchman clock) is now kept beside another name too, where it was dropped before.

These are the implementer's choices. Another way was to ask the observer by one name only, the canonical one; that
changes every path the runs record, and was not tried.

## Also: switches held over a test's runs

Run beside the other corpus tests, `the_run_after_a_cold_one_writes_nothing` and
`a_maintained_run_decodes_and_copies_no_tree` failed in three of five runs of the module. Another test set its own
switches (`RERAIL_INLINE_ONLY`, `RERAIL_CROSS_MEMO`, through `switches::set`) between two runs that compare. The child
processes took the switches as they started without holding them. The five tests that compare runs now hold
`switches::reading` over all their runs: those two, `a_header_edit_asks_again_only_for_the_units_that_read_it`,
`a_rule_edit_over_known_files_writes_their_entries_with_no_sql_in_the_waves` and
`a_second_run_in_one_process_records_the_first_runs_manifest`.

## Checks

- The header test and the warm-run test now reach their sources through a symbolic link of their own, so they test
  both names on every host. Each was run with the fix taken out, on a canonical temporary directory: each failed (an
  edit to `h.h` asked for no unit; a warm run wrote a cursor).
- Both pass with the machine's temporary directory, one through `/tmp`, and a canonical one.
- `collection_drops_what_no_run_asks_any_more`: two names of one directory with the same listing cursor are both kept,
  and a third with an older generation is dropped.
- `the_journal_names_an_added_entry_and_not_a_racy_answer_that_holds` and
  `a_run_record_is_answered_from_the_observers_own_journal` expect the path under both names where they differ.
- `rerail-store`: 251 tests pass, with the machine's temporary directory and a canonical one.
- The corpus tests (30) pass in three runs of the module, all at once.
- `a_tests_store_is_a_copy_of_the_template_and_goes_with_its_test`, reported as failing beside other tests, passed in
  each of the twelve module runs here. Its cause, a lock held open, was not found.

## Not checked

- Windows, and Linux hosts.
- Whether other places compare a path the observer named with one a reader holds, outside `changed_since`'s callers.
- Nothing about speed was measured.
